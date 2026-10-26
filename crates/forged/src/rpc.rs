//! JSON-RPC request handling and method dispatch.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use forge_api::events::{EVENT_NOTIFICATION, EventNotification, Topic, TurnSource};
use forge_api::methods::*;
use forge_api::model::{Platform, Project};
use forge_api::rpc::{JSONRPC_VERSION, Notification, Request, Response, RpcError, codes};
use forge_api::{API_VERSION, ApiError, SubscriptionId};
use forge_config::Paths;
use forge_events::{EventLog, Query};
use forge_session::SessionManager;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::db::Db;
use crate::{agents, projects, sessions};

/// Most events one `events.query` returns.
const MAX_QUERY_LIMIT: u32 = 1000;

/// Daemon-wide state shared by every connection.
pub struct State {
    pub db: Db,
    pub log: Arc<EventLog>,
    pub sessions: Arc<SessionManager>,
    pub sandbox: Arc<dyn forge_sandbox::Sandbox>,
    pub paths: Paths,
    /// Where agents' secrets come from.
    pub secrets: Arc<dyn forge_sandbox::secrets::SecretStore>,
    /// The user's real home, where agents' own sign-ins and settings are read from.
    pub home: std::path::PathBuf,
}

/// Work to run once a request's response is queued, such as replaying events to a new
/// subscription, which the client can only match up after it has the subscription ID.
pub type Deferred = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The bounded queue of messages to one client. A client that stops reading fills it, and is
/// then disconnected rather than allowed to hold memory or stall event delivery.
#[derive(Clone)]
pub struct Outbox {
    tx: mpsc::Sender<String>,
    /// Cancelled to drop the connection.
    pub drop: CancellationToken,
    replay_timeout: Duration,
}

impl Outbox {
    pub fn new(tx: mpsc::Sender<String>, replay_timeout: Duration) -> Self {
        Self {
            tx,
            drop: CancellationToken::new(),
            replay_timeout,
        }
    }

    /// Queue `text` without waiting. A full queue drops the connection. Returns whether it was
    /// queued.
    pub fn send(&self, text: String) -> bool {
        match self.tx.try_send(text) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!("client is not reading; dropping the connection");
                self.drop.cancel();
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// Queue `text`, waiting for room for a while; for replays, which can outrun the queue.
    async fn send_patiently(&self, text: String) -> bool {
        match tokio::time::timeout(self.replay_timeout, self.tx.send(text)).await {
            Ok(Ok(())) => true,
            Ok(Err(_)) => false,
            Err(_) => {
                tracing::warn!("client stopped reading during replay; dropping the connection");
                self.drop.cancel();
                false
            }
        }
    }
}

#[derive(Default)]
struct Subs {
    ids: HashSet<SubscriptionId>,
    /// Set once the connection is gone; later subscriptions are removed at once.
    closed: bool,
}

/// Per-connection state.
pub struct Conn {
    pub outbox: Outbox,
    subs: Mutex<Subs>,
}

impl Conn {
    pub fn new(outbox: Outbox) -> Self {
        Self {
            outbox,
            subs: Mutex::default(),
        }
    }

    /// Drop every subscription this connection made, now and later.
    pub fn close(&self, state: &State) {
        let mut subs = self.lock_subs();
        subs.closed = true;
        for id in subs.ids.drain() {
            state.log.hub().unsubscribe(&id);
        }
    }

    fn lock_subs(&self) -> MutexGuard<'_, Subs> {
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

impl From<forge_events::LogError> for HandlerError {
    fn from(e: forge_events::LogError) -> Self {
        match e {
            forge_events::LogError::UnknownSession(id) => {
                ApiError::not_found(format!("session {id}")).into()
            }
            other => HandlerError::Internal(other.into()),
        }
    }
}

/// Handle one text frame; returns the response to send and any work to run after it.
pub async fn handle_text(state: &State, conn: &Conn, text: &str) -> (Response, Option<Deferred>) {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            let error = RpcError::protocol(codes::PARSE_ERROR, e.to_string());
            return (Response::err(Value::Null, error), None);
        }
    };
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let request: Request = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => {
            let error = RpcError::protocol(codes::INVALID_REQUEST, e.to_string());
            return (Response::err(id, error), None);
        }
    };
    if request.jsonrpc != JSONRPC_VERSION {
        let error = RpcError::protocol(codes::INVALID_REQUEST, "jsonrpc must be \"2.0\"");
        return (Response::err(id, error), None);
    }
    let mut deferred = None;
    let response = match dispatch(state, conn, &request.method, request.params, &mut deferred).await
    {
        Ok(result) => Response::ok(request.id, result),
        Err(error) => Response::err(request.id, error),
    };
    (response, deferred)
}

async fn dispatch(
    state: &State,
    conn: &Conn,
    method: &str,
    params: Value,
    deferred: &mut Option<Deferred>,
) -> Result<Value, RpcError> {
    let manager = &state.sessions;
    match method {
        ServerHello::NAME => run::<ServerHello>(params, async |_| Ok(hello(state))).await,
        ProjectOpen::NAME => {
            run::<ProjectOpen>(params, async |p| projects::open(&state.db, &p.path).await).await
        }
        ProjectList::NAME => {
            run::<ProjectList>(params, async |_| Ok(state.db.list_projects().await?)).await
        }
        ProjectGet::NAME => {
            run::<ProjectGet>(params, async |p| get_project(state, p.project_id).await).await
        }
        AgentList::NAME => {
            run::<AgentList>(params, async |p| agents::list(state, p.project_id).await).await
        }
        AgentSignInMethods::NAME => {
            run::<AgentSignInMethods>(params, async |p| {
                agents::sign_in_methods(state, &p.agent).await
            })
            .await
        }
        AgentSignIn::NAME => {
            run::<AgentSignIn>(params, async |p| {
                agents::sign_in_with(state, &p.agent, &p.method_id).await?;
                Ok(forge_api::methods::Empty {})
            })
            .await
        }
        SessionCreate::NAME => {
            run::<SessionCreate>(params, async |p| sessions::create(state, p).await).await
        }
        SessionBranchPreview::NAME => {
            run::<SessionBranchPreview>(params, async |p| sessions::branch_preview(state, p).await)
                .await
        }
        SessionPrompt::NAME => {
            run::<SessionPrompt>(params, async |p| {
                let turn_id = manager
                    .prompt(&p.session_id, p.text, TurnSource::Human)
                    .await?;
                Ok(TurnResult { turn_id })
            })
            .await
        }
        SessionCancel::NAME => {
            run::<SessionCancel>(params, async |p| {
                manager.cancel(&p.session_id).await?;
                Ok(Empty {})
            })
            .await
        }
        SessionSetMode::NAME => {
            run::<SessionSetMode>(params, async |p| sessions::set_mode(state, p).await).await
        }
        SessionGet::NAME => {
            run::<SessionGet>(params, async |p| Ok(manager.get(&p.session_id).await?)).await
        }
        SessionList::NAME => {
            run::<SessionList>(params, async |p| {
                get_project(state, p.project_id.clone()).await?;
                Ok(manager.list(&p.project_id, p.states).await?)
            })
            .await
        }
        SessionClose::NAME => {
            run::<SessionClose>(params, async |p| {
                let reason = sessions::close_reason(p.outcome);
                manager.close(&p.session_id, reason).await?;
                Ok(Empty {})
            })
            .await
        }
        ApprovalList::NAME => {
            run::<ApprovalList>(params, async |p| {
                if let Some(id) = &p.project_id {
                    get_project(state, id.clone()).await?;
                }
                Ok(manager.approvals(p.project_id).await?)
            })
            .await
        }
        ApprovalDecide::NAME => {
            run::<ApprovalDecide>(params, async |p| {
                manager
                    .decide(&p.approval_id, p.decision, p.remember, p.note)
                    .await?;
                Ok(Empty {})
            })
            .await
        }
        EventsSubscribe::NAME => {
            run::<EventsSubscribe>(params, async |p| {
                let (result, replay) = subscribe(state, conn, p).await?;
                *deferred = Some(replay);
                Ok(result)
            })
            .await
        }
        EventsUnsubscribe::NAME => {
            run::<EventsUnsubscribe>(params, async |p| unsubscribe(state, conn, p)).await
        }
        EventsQuery::NAME => run::<EventsQuery>(params, async |p| query(state, p).await).await,
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

fn hello(state: &State) -> HelloResult {
    HelloResult {
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        api_version: API_VERSION.to_owned(),
        platform: Platform::current(),
        sandbox_tiers: state
            .sandbox
            .tiers()
            .into_iter()
            .filter(|t| t.is_available())
            .map(|t| t.tier)
            .collect(),
    }
}

async fn get_project(state: &State, id: forge_api::ProjectId) -> Result<Project, HandlerError> {
    match state.db.get_project(id.clone()).await? {
        Some(project) => Ok(project),
        None => Err(ApiError::not_found(format!("project {id}")).into()),
    }
}

async fn query(
    state: &State,
    params: EventsQueryParams,
) -> Result<Vec<forge_api::events::Event>, HandlerError> {
    if params.limit == 0 || params.limit > MAX_QUERY_LIMIT {
        return Err(
            ApiError::validation(format!("limit must be between 1 and {MAX_QUERY_LIMIT}")).into(),
        );
    }
    let log = state.log.clone();
    let session = params.session_id;
    let query = Query {
        kinds: params.kinds,
        text: params.text.filter(|t| !t.trim().is_empty()),
        before: params.before,
        limit: params.limit,
    };
    let events = tokio::task::spawn_blocking(move || log.query(&session, &query))
        .await
        .map_err(anyhow::Error::from)??;
    Ok(events)
}

/// Events held back from a new subscription until its response and replay are sent.
enum Gate {
    Holding(Vec<EventNotification>),
    /// Live delivery; skip anything at or before this cursor, which replay already sent.
    Open(u64),
}

fn notification_text(note: &EventNotification) -> Option<String> {
    let params = serde_json::to_value(note).ok()?;
    serde_json::to_string(&Notification {
        jsonrpc: JSONRPC_VERSION.into(),
        method: EVENT_NOTIFICATION.into(),
        params,
    })
    .ok()
}

/// Register the subscription now, holding live events back; the returned work replays events
/// after `afterSeq` and then opens the gate, so the client sees each event once, in order.
async fn subscribe(
    state: &State,
    conn: &Conn,
    params: EventsSubscribeParams,
) -> Result<(SubscribeResult, Deferred), HandlerError> {
    if params.topics.is_empty() {
        return Err(ApiError::validation("topics must not be empty").into());
    }
    for topic in &params.topics {
        match topic {
            Topic::Board | Topic::Approvals => {}
            Topic::Project(id) => {
                get_project(state, id.clone()).await?;
            }
            Topic::Session(id) => {
                state.sessions.get(id).await?;
            }
        }
    }
    let head = state.log.head();
    if let Some(after) = params.after_seq
        && after > head
    {
        return Err(ApiError::validation(format!(
            "afterSeq {after} is ahead of the event stream (latest {head}); resubscribe without afterSeq"
        ))
        .into());
    }

    let gate = Arc::new(Mutex::new(Gate::Holding(Vec::new())));
    let topics: HashSet<Topic> = params.topics.iter().cloned().collect();
    let (deliver_gate, outbox) = (gate.clone(), conn.outbox.clone());
    let id = state.log.hub().subscribe(
        params.topics,
        Box::new(move |note| {
            let mut gate = lock(&deliver_gate);
            match &mut *gate {
                Gate::Holding(held) => {
                    held.push(note);
                    true
                }
                Gate::Open(after) if note.seq <= *after => true,
                Gate::Open(_) => notification_text(&note).is_some_and(|t| outbox.send(t)),
            }
        }),
    );
    {
        let mut subs = conn.lock_subs();
        if subs.closed {
            // The connection went away while this request ran.
            state.log.hub().unsubscribe(&id);
        } else {
            subs.ids.insert(id.clone());
        }
    }

    let (log, outbox, sub_id) = (state.log.clone(), conn.outbox.clone(), id.clone());
    let after = params.after_seq;
    let replay: Deferred = Box::pin(async move {
        // Without `afterSeq` nothing is replayed, and every event held since registering is
        // new to the client, including any appended between reading `head` and registering.
        let mut last = after.unwrap_or(0);
        if let Some(after) = after {
            let replayed = tokio::task::spawn_blocking(move || log.replay(&topics, after)).await;
            let replayed = match replayed {
                Ok(Ok(events)) => events,
                Ok(Err(e)) => {
                    tracing::error!(error = %e, "replaying events");
                    outbox.drop.cancel();
                    return;
                }
                Err(e) => {
                    tracing::error!(error = %e, "replaying events");
                    outbox.drop.cancel();
                    return;
                }
            };
            for r in replayed {
                let note = EventNotification {
                    subscription_id: sub_id.clone(),
                    topic: r.topic,
                    seq: r.stream_seq,
                    event: r.event,
                };
                let Some(text) = notification_text(&note) else {
                    continue;
                };
                if !outbox.send_patiently(text).await {
                    return;
                }
                last = r.stream_seq;
            }
        }
        open_gate(&gate, last, &outbox).await;
    });
    Ok((
        SubscribeResult {
            subscription_id: id,
        },
        replay,
    ))
}

/// Send the events held while the subscription was set up, skipping any at or before `last`
/// (already replayed), then switch the gate to live delivery.
async fn open_gate(gate: &Mutex<Gate>, mut last: u64, outbox: &Outbox) {
    loop {
        let held = {
            let mut gate = lock(gate);
            match &mut *gate {
                Gate::Holding(held) if held.is_empty() => {
                    *gate = Gate::Open(last);
                    return;
                }
                Gate::Holding(held) => std::mem::take(held),
                Gate::Open(_) => return,
            }
        };
        for note in held {
            if note.seq <= last {
                continue;
            }
            let Some(text) = notification_text(&note) else {
                continue;
            };
            if !outbox.send_patiently(text).await {
                return;
            }
            last = note.seq;
        }
    }
}

fn unsubscribe(
    state: &State,
    conn: &Conn,
    params: EventsUnsubscribeParams,
) -> Result<Empty, HandlerError> {
    let id = params.subscription_id;
    if !conn.lock_subs().ids.remove(&id) {
        return Err(ApiError::not_found(format!("subscription {id} on this connection")).into());
    }
    state.log.hub().unsubscribe(&id);
    Ok(Empty {})
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use forge_api::events::{Event, EventPayload};
    use forge_api::model::Usage;
    use forge_api::{SessionId, Timestamp};

    use super::*;

    fn note(seq: u64) -> EventNotification {
        EventNotification {
            subscription_id: SubscriptionId::new(),
            topic: Topic::Board,
            seq,
            event: Event {
                seq,
                session_id: SessionId::new(),
                at: Timestamp::now(),
                parent: None,
                payload: EventPayload::Usage(Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: None,
                }),
            },
        }
    }

    fn sent(rx: &mut mpsc::Receiver<String>) -> Vec<u64> {
        let mut seqs = Vec::new();
        while let Ok(text) = rx.try_recv() {
            let value: Value = serde_json::from_str(&text).unwrap();
            seqs.push(value["params"]["seq"].as_u64().unwrap());
        }
        seqs
    }

    #[tokio::test]
    async fn opening_the_gate_sends_held_events_newer_than_the_cursor() {
        let (tx, mut rx) = mpsc::channel(16);
        let outbox = Outbox::new(tx, Duration::from_secs(1));

        // A fresh subscription starts from 0: everything held is new to the client.
        let gate = Mutex::new(Gate::Holding(vec![note(7), note(8)]));
        open_gate(&gate, 0, &outbox).await;
        assert_eq!(sent(&mut rx), [7, 8]);
        assert!(matches!(*lock(&gate), Gate::Open(8)));

        // After a replay up to 7, only what came later is sent.
        let gate = Mutex::new(Gate::Holding(vec![note(6), note(7), note(8)]));
        open_gate(&gate, 7, &outbox).await;
        assert_eq!(sent(&mut rx), [8]);

        let gate = Mutex::new(Gate::Holding(Vec::new()));
        open_gate(&gate, 5, &outbox).await;
        assert!(matches!(*lock(&gate), Gate::Open(5)));
    }
}
