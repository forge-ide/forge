//! MCP hub and Forge's own MCP server.
//!
//! For now only Forge's server, with `git_commit` and the review tools `review_threads` and
//! `review_reply`: a session's agent reaches it
//! over Streamable HTTP on the session's egress proxy (`/_forge/<token>/mcp`), which hands each
//! request to [`ForgeServer`]. Responses are plain JSON; the server never opens an event
//! stream, so `GET` is refused by the proxy.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use forge_api::ThreadId;
use forge_api::model::{Thread, ThreadStatus};
use forge_sandbox::proxy::{LocalRequest, LocalResponse, LocalService};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// This crate's package name.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The name agents know Forge's server by; their tools show as `forge.git_commit`,
/// `mcp__forge__git_commit`, or `mcp_forge_git_commit`.
pub const SERVER_NAME: &str = "forge";

/// The `git_commit` tool's name.
pub const GIT_COMMIT: &str = "git_commit";

/// The `review_threads` tool's name.
pub const REVIEW_THREADS: &str = "review_threads";

/// The `review_reply` tool's name.
pub const REVIEW_REPLY: &str = "review_reply";

/// Longest reply accepted.
const MAX_REPLY: usize = 16 * 1024;

/// MCP protocol versions the server speaks, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Longest commit message accepted.
const MAX_MESSAGE: usize = 64 * 1024;

/// `git_commit`'s input.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitCommitArgs {
    pub message: String,
    /// Paths relative to the session's working directory; every change when absent.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
}

/// `git_commit`'s output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GitCommitResult {
    pub commit: String,
    pub branch: String,
}

/// `review_threads`' input.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewThreadsArgs {
    /// Only threads with this status; every thread when absent.
    #[serde(default)]
    pub status: Option<ThreadStatus>,
}

/// `review_reply`'s input.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewReplyArgs {
    pub thread_id: ThreadId,
    pub body: String,
}

/// The tools behind the server; the session supplies them. An `Err` is the reason a tool
/// didn't do what it was asked, for the agent.
pub trait ForgeTools: Send + Sync + 'static {
    /// Commit the session's changes.
    fn git_commit(&self, args: GitCommitArgs) -> BoxFuture<'_, Result<GitCommitResult, String>>;
    /// The session's review threads.
    fn review_threads(&self, args: ReviewThreadsArgs)
    -> BoxFuture<'_, Result<Vec<Thread>, String>>;
    /// Reply to one of the session's review threads, as the agent.
    fn review_reply(&self, args: ReviewReplyArgs) -> BoxFuture<'_, Result<(), String>>;
}

/// Forge's MCP server for one session.
pub struct ForgeServer {
    tools: Arc<dyn ForgeTools>,
}

impl ForgeServer {
    pub fn new(tools: Arc<dyn ForgeTools>) -> Self {
        Self { tools }
    }

    /// Answer one JSON-RPC message; `None` for a notification, which gets no answer.
    pub async fn handle(&self, message: &[u8]) -> Option<Value> {
        let Ok(message) = serde_json::from_slice::<Value>(message) else {
            return Some(error(Value::Null, -32700, "parse error"));
        };
        if message.is_array() {
            return Some(error(Value::Null, -32600, "batches are not supported"));
        }
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response to a request the server never sent.
            return id.map(|id| error(id, -32600, "not a request"));
        };
        let id = id?;
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .filter(|v| PROTOCOL_VERSIONS.contains(v))
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                result(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
                        "instructions": "Commit your work with git_commit; running git commit yourself is refused. When the human requests changes, answer each review thread with review_reply; review_threads lists them.",
                    }),
                )
            }
            "ping" => result(id, json!({})),
            "tools/list" => result(
                id,
                json!({ "tools": [git_commit_tool(), review_threads_tool(), review_reply_tool()] }),
            ),
            "tools/call" => self.call(id, params).await,
            _ => error(id, -32601, &format!("method {method} not found")),
        })
    }

    async fn call(&self, id: Value, params: Value) -> Value {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
        match name {
            GIT_COMMIT => self.commit(id, arguments).await,
            REVIEW_THREADS => self.threads(id, arguments).await,
            REVIEW_REPLY => self.reply(id, arguments).await,
            _ => error(id, -32602, &format!("no tool named {name:?}")),
        }
    }

    async fn threads(&self, id: Value, arguments: Value) -> Value {
        let args = match serde_json::from_value::<ReviewThreadsArgs>(arguments) {
            Ok(args) => args,
            Err(e) => return tool_error(id, &format!("invalid arguments: {e}")),
        };
        match self.tools.review_threads(args).await {
            Ok(threads) => {
                let text = serde_json::to_string_pretty(&threads).unwrap_or_default();
                result(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": text }],
                        "structuredContent": { "threads": threads },
                        "isError": false,
                    }),
                )
            }
            Err(reason) => tool_error(id, &reason),
        }
    }

    async fn reply(&self, id: Value, arguments: Value) -> Value {
        let args = match serde_json::from_value::<ReviewReplyArgs>(arguments) {
            Ok(args) if args.body.trim().is_empty() => {
                return tool_error(id, "body must not be empty");
            }
            Ok(args) if args.body.len() > MAX_REPLY => {
                return tool_error(id, "body is too long; replies are up to 16 KiB");
            }
            Ok(args) => args,
            Err(e) => return tool_error(id, &format!("invalid arguments: {e}")),
        };
        match self.tools.review_reply(args).await {
            Ok(()) => result(
                id,
                json!({
                    "content": [{ "type": "text", "text": "Replied." }],
                    "structuredContent": {},
                    "isError": false,
                }),
            ),
            Err(reason) => tool_error(id, &reason),
        }
    }

    async fn commit(&self, id: Value, arguments: Value) -> Value {
        let args = match serde_json::from_value::<GitCommitArgs>(arguments) {
            Ok(args) if args.message.trim().is_empty() => {
                return tool_error(id, "message must not be empty");
            }
            Ok(args) if args.message.len() > MAX_MESSAGE => {
                return tool_error(id, "message is too long");
            }
            Ok(args) => args,
            Err(e) => return tool_error(id, &format!("invalid arguments: {e}")),
        };
        match self.tools.git_commit(args).await {
            Ok(done) => result(
                id,
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Committed {} on {}.", done.commit, done.branch),
                    }],
                    "structuredContent": done,
                    "isError": false,
                }),
            ),
            Err(reason) => tool_error(id, &reason),
        }
    }
}

impl LocalService for ForgeServer {
    fn call(
        &self,
        request: LocalRequest,
    ) -> Pin<Box<dyn Future<Output = LocalResponse> + Send + '_>> {
        Box::pin(async move {
            match self.handle(&request.body).await {
                Some(answer) => LocalResponse {
                    status: 200,
                    content_type: "application/json",
                    body: serde_json::to_vec(&answer).unwrap_or_default(),
                },
                None => LocalResponse {
                    status: 202,
                    content_type: "application/json",
                    body: Vec::new(),
                },
            }
        })
    }
}

fn git_commit_tool() -> Value {
    json!({
        "name": GIT_COMMIT,
        "description": "Commit the session's changes on its branch. Forge checks the commit \
            against its policy and runs the project's commit.pre hooks first, and may ask a \
            human. Commits every change, including new files, unless paths names some.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "The commit message." },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only these paths, relative to the working directory.",
                },
            },
            "required": ["message"],
            "additionalProperties": false,
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "commit": { "type": "string" },
                "branch": { "type": "string" },
            },
            "required": ["commit", "branch"],
        },
    })
}

fn review_threads_tool() -> Value {
    json!({
        "name": REVIEW_THREADS,
        "description": "List this session's review threads: the file, the lines each was \
            written on and where they are now, the code it quotes, its status, and its \
            comments. Each has the ID review_reply takes.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["open", "resolved", "outdated"],
                    "description": "Only threads with this status.",
                },
            },
            "additionalProperties": false,
        },
        "outputSchema": {
            "type": "object",
            "properties": { "threads": { "type": "array", "items": { "type": "object" } } },
            "required": ["threads"],
        },
    })
}

fn review_reply_tool() -> Value {
    json!({
        "name": REVIEW_REPLY,
        "description": "Reply to one of this session's review threads, for the human to read: \
            what you changed for it, or why you didn't. Only the human resolves threads.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "threadId": { "type": "string", "description": "The thread's ID (thr_...)." },
                "body": { "type": "string", "description": "The reply, up to 16 KiB." },
            },
            "required": ["threadId", "body"],
            "additionalProperties": false,
        },
    })
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A tool call that ran but didn't succeed: the agent sees the reason.
fn tool_error(id: Value, reason: &str) -> Value {
    result(
        id,
        json!({ "content": [{ "type": "text", "text": reason }], "isError": true }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Tools(Mutex<Vec<GitCommitArgs>>, Mutex<Vec<ReviewReplyArgs>>);

    impl ForgeTools for Tools {
        fn git_commit(
            &self,
            args: GitCommitArgs,
        ) -> BoxFuture<'_, Result<GitCommitResult, String>> {
            let refuse = args.message == "refuse";
            self.0.lock().unwrap().push(args);
            Box::pin(async move {
                if refuse {
                    Err("policy denied it".into())
                } else {
                    Ok(GitCommitResult {
                        commit: "abc123".into(),
                        branch: "feat/x".into(),
                    })
                }
            })
        }

        fn review_threads(
            &self,
            args: ReviewThreadsArgs,
        ) -> BoxFuture<'_, Result<Vec<Thread>, String>> {
            Box::pin(async move {
                match args.status {
                    Some(ThreadStatus::Resolved) => Err("no".into()),
                    _ => Ok(Vec::new()),
                }
            })
        }

        fn review_reply(&self, args: ReviewReplyArgs) -> BoxFuture<'_, Result<(), String>> {
            let theirs = args.body == "theirs";
            self.1.lock().unwrap().push(args);
            Box::pin(async move {
                if theirs {
                    Err("no thread of this session has that ID".into())
                } else {
                    Ok(())
                }
            })
        }
    }

    async fn ask(server: &ForgeServer, message: Value) -> Option<Value> {
        server.handle(message.to_string().as_bytes()).await
    }

    #[tokio::test]
    async fn speaks_enough_mcp_to_list_and_call_git_commit() {
        let tools = Arc::new(Tools::default());
        let server = ForgeServer::new(tools.clone());
        let init = ask(
            &server,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}),
        )
        .await
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "forge");
        let unknown = ask(
            &server,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": "1999-01-01"}}),
        )
        .await
        .unwrap();
        assert_eq!(unknown["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        assert_eq!(
            ask(
                &server,
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
            )
            .await,
            None
        );
        let list = ask(
            &server,
            json!({"jsonrpc": "2.0", "id": "a", "method": "tools/list"}),
        )
        .await
        .unwrap();
        let names: Vec<_> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["git_commit", "review_threads", "review_reply"]);

        let call = |args: Value| {
            json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                   "params": {"name": "git_commit", "arguments": args}})
        };
        let done = ask(
            &server,
            call(json!({"message": "add f", "paths": ["src/f.rs"]})),
        )
        .await
        .unwrap();
        assert_eq!(done["id"], 7);
        assert_eq!(done["result"]["isError"], false);
        assert_eq!(done["result"]["structuredContent"]["commit"], "abc123");
        assert_eq!(
            tools.0.lock().unwrap()[0],
            GitCommitArgs {
                message: "add f".into(),
                paths: Some(vec!["src/f.rs".into()]),
            }
        );
        let refused = ask(&server, call(json!({"message": "refuse"})))
            .await
            .unwrap();
        assert_eq!(refused["result"]["isError"], true);
        assert_eq!(refused["result"]["content"][0]["text"], "policy denied it");
        // Bad arguments never reach the tool.
        for args in [
            json!({}),
            json!({"message": " "}),
            json!({"message": "m", "all": true}),
        ] {
            let bad = ask(&server, call(args.clone())).await.unwrap();
            assert_eq!(bad["result"]["isError"], true, "{args}");
        }
        assert_eq!(tools.0.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn review_tools_list_threads_and_take_replies_within_limits() {
        let tools = Arc::new(Tools::default());
        let server = ForgeServer::new(tools.clone());
        let call = |name: &str, args: Value| {
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                   "params": {"name": name, "arguments": args}})
        };
        let listed = ask(&server, call("review_threads", json!({"status": "open"})))
            .await
            .unwrap();
        assert_eq!(listed["result"]["isError"], false);
        assert_eq!(listed["result"]["structuredContent"]["threads"], json!([]));
        let refused = ask(
            &server,
            call("review_threads", json!({"status": "resolved"})),
        )
        .await
        .unwrap();
        assert_eq!(refused["result"]["isError"], true);
        let thread = ThreadId::new().to_string();
        let done = ask(
            &server,
            call(
                "review_reply",
                json!({"threadId": thread, "body": "Fixed."}),
            ),
        )
        .await
        .unwrap();
        assert_eq!(done["result"]["isError"], false);
        assert_eq!(tools.1.lock().unwrap()[0].thread_id.as_str(), thread);
        let other = ask(
            &server,
            call(
                "review_reply",
                json!({"threadId": thread, "body": "theirs"}),
            ),
        )
        .await
        .unwrap();
        assert_eq!(other["result"]["isError"], true);
        // Bad arguments never reach the tool.
        for args in [
            json!({"threadId": thread}),
            json!({"threadId": thread, "body": "  "}),
            json!({"threadId": thread, "body": "x".repeat(16 * 1024 + 1)}),
            json!({"threadId": "not-an-id", "body": "x"}),
            json!({"threadId": thread, "body": "x", "resolve": true}),
        ] {
            let bad = ask(&server, call("review_reply", args.clone()))
                .await
                .unwrap();
            assert_eq!(bad["result"]["isError"], true, "{args}");
        }
        for args in [json!({"status": "closed"}), json!({"all": true})] {
            let bad = ask(&server, call("review_threads", args.clone()))
                .await
                .unwrap();
            assert_eq!(bad["result"]["isError"], true, "{args}");
        }
        assert_eq!(tools.1.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn anything_else_is_a_json_rpc_error() {
        let server = ForgeServer::new(Arc::new(Tools::default()));
        let other = ask(
            &server,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "shell"}}),
        )
        .await
        .unwrap();
        assert_eq!(other["error"]["code"], -32602);
        let missing = ask(
            &server,
            json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}),
        )
        .await
        .unwrap();
        assert_eq!(missing["error"]["code"], -32601);
        assert_eq!(
            server.handle(b"{not json").await.unwrap()["error"]["code"],
            -32700
        );
        assert_eq!(server.handle(b"[]").await.unwrap()["error"]["code"], -32600);
    }
}
