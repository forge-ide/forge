//! JSON-RPC request handling and method dispatch.

use std::collections::HashSet;
use std::sync::Mutex;

use forge_api::events::{EVENT_NOTIFICATION, Topic};
use forge_api::methods::*;
use forge_api::model::{Platform, Project};
use forge_api::rpc::{JSONRPC_VERSION, Notification, Request, Response, RpcError, codes};
use forge_api::{API_VERSION, ApiError, SubscriptionId};
use forge_events::Hub;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::db::Db;
use crate::projects;

/// Daemon-wide state shared by every connection.
pub struct State {
    pub db: Db,
    pub hub: Hub,
}

/// Per-connection state.
pub struct Conn {
    /// Serialized messages to write to this client.
    pub outgoing: mpsc::UnboundedSender<String>,
    subs: Mutex<HashSet<SubscriptionId>>,
}

impl Conn {
    pub fn new(outgoing: mpsc::UnboundedSender<String>) -> Self {
        Self {
            outgoing,
            subs: Mutex::default(),
        }
    }

    /// Drop every subscription this connection made.
    pub fn close(&self, state: &State) {
        for id in self.lock_subs().drain() {
            state.hub.unsubscribe(&id);
        }
    }

    fn lock_subs(&self) -> std::sync::MutexGuard<'_, HashSet<SubscriptionId>> {
        self.subs.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// A handler failure: an application error for the client, or an internal one for the log.
#[derive(Debug)]
pub enum HandlerError {
    Api(ApiError),
    Internal(anyhow::Error),
}

impl From<ApiError> for HandlerError {
    fn from(e: ApiError) -> Self {
        HandlerError::Api(e)
    }
}

impl From<anyhow::Error> for HandlerError {
    fn from(e: anyhow::Error) -> Self {
        HandlerError::Internal(e)
    }
}

/// Handle one text frame; returns the response to send.
pub async fn handle_text(state: &State, conn: &Conn, text: &str) -> Response {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            return Response::err(
                Value::Null,
                RpcError::protocol(codes::PARSE_ERROR, e.to_string()),
            );
        }
    };
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let request: Request = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => {
            return Response::err(
                id,
                RpcError::protocol(codes::INVALID_REQUEST, e.to_string()),
            );
        }
    };
    if request.jsonrpc != JSONRPC_VERSION {
        return Response::err(
            id,
            RpcError::protocol(codes::INVALID_REQUEST, "jsonrpc must be \"2.0\""),
        );
    }
    match dispatch(state, conn, &request.method, request.params).await {
        Ok(result) => Response::ok(request.id, result),
        Err(error) => Response::err(request.id, error),
    }
}

async fn dispatch(
    state: &State,
    conn: &Conn,
    method: &str,
    params: Value,
) -> Result<Value, RpcError> {
    match method {
        ServerHello::NAME => run::<ServerHello>(params, async |_| Ok(hello())).await,
        ProjectOpen::NAME => {
            run::<ProjectOpen>(params, async |p| projects::open(&state.db, &p.path).await).await
        }
        ProjectList::NAME => {
            run::<ProjectList>(params, async |_| Ok(state.db.list_projects().await?)).await
        }
        ProjectGet::NAME => {
            run::<ProjectGet>(params, async |p| get_project(state, p.project_id).await).await
        }
        EventsSubscribe::NAME => {
            run::<EventsSubscribe>(params, async |p| subscribe(state, conn, p).await).await
        }
        EventsUnsubscribe::NAME => {
            run::<EventsUnsubscribe>(params, async |p| unsubscribe(state, conn, p)).await
        }
        EventsQuery::NAME => {
            run::<EventsQuery>(params, async |p| Err(no_session(&p.session_id))).await
        }
        name if METHOD_NAMES.contains(&name) => Err(RpcError::protocol(
            codes::METHOD_NOT_FOUND,
            format!("{name} is part of API {API_VERSION} but not implemented by this daemon yet"),
        )),
        name => Err(RpcError::protocol(
            codes::METHOD_NOT_FOUND,
            format!("unknown method {name}"),
        )),
    }
}

/// Parse params for `M`, run the handler, and encode its result or error.
async fn run<M: Method>(
    params: Value,
    handler: impl AsyncFnOnce(M::Params) -> Result<M::Result, HandlerError>,
) -> Result<Value, RpcError> {
    // `{}` params may be omitted entirely.
    let params = if params.is_null() {
        Value::Object(Default::default())
    } else {
        params
    };
    let params: M::Params = serde_json::from_value(params)
        .map_err(|e| ApiError::validation(format!("invalid params for {}: {e}", M::NAME)))?;
    match handler(params).await {
        Ok(result) => serde_json::to_value(result).map_err(|e| {
            RpcError::protocol(codes::INTERNAL_ERROR, format!("encoding result: {e}"))
        }),
        Err(HandlerError::Api(e)) => Err(e.into()),
        Err(HandlerError::Internal(e)) => {
            tracing::error!(method = M::NAME, error = format!("{e:#}"), "internal error");
            Err(RpcError::protocol(
                codes::INTERNAL_ERROR,
                "internal error; see the daemon log for details",
            ))
        }
    }
}

fn hello() -> HelloResult {
    HelloResult {
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        api_version: API_VERSION.to_owned(),
        platform: Platform::current(),
        // No tier is available until forge-sandbox implements them (M3).
        sandbox_tiers: Vec::new(),
    }
}

async fn get_project(state: &State, id: forge_api::ProjectId) -> Result<Project, HandlerError> {
    match state.db.get_project(id.clone()).await? {
        Some(project) => Ok(project),
        None => Err(ApiError::not_found(format!("project {id}")).into()),
    }
}

/// Sessions arrive in M2; until then no session ID exists.
fn no_session(id: &forge_api::SessionId) -> HandlerError {
    ApiError::not_found(format!("session {id}")).into()
}

async fn subscribe(
    state: &State,
    conn: &Conn,
    params: EventsSubscribeParams,
) -> Result<SubscribeResult, HandlerError> {
    if params.topics.is_empty() {
        return Err(ApiError::validation("topics must not be empty").into());
    }
    for topic in &params.topics {
        match topic {
            Topic::Board | Topic::Approvals => {}
            Topic::Project(id) => {
                if state.db.get_project(id.clone()).await?.is_none() {
                    return Err(ApiError::not_found(format!("project {id}")).into());
                }
            }
            Topic::Session(id) => return Err(no_session(id)),
        }
    }
    // `afterSeq` replay needs the event log, which lands in M2. Until then there is nothing to replay.
    let outgoing = conn.outgoing.clone();
    let id = state.hub.subscribe(
        params.topics,
        Box::new(move |note| {
            let Ok(params) = serde_json::to_value(note) else {
                return true;
            };
            let message = Notification {
                jsonrpc: JSONRPC_VERSION.into(),
                method: EVENT_NOTIFICATION.into(),
                params,
            };
            serde_json::to_string(&message).is_ok_and(|text| outgoing.send(text).is_ok())
        }),
    );
    conn.lock_subs().insert(id.clone());
    Ok(SubscribeResult {
        subscription_id: id,
    })
}

fn unsubscribe(
    state: &State,
    conn: &Conn,
    params: EventsUnsubscribeParams,
) -> Result<Empty, HandlerError> {
    let id = params.subscription_id;
    if !conn.lock_subs().remove(&id) {
        return Err(ApiError::not_found(format!("subscription {id} on this connection")).into());
    }
    state.hub.unsubscribe(&id);
    Ok(Empty {})
}
