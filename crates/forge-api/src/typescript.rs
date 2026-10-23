//! Generates `packages/client/src/generated/api.ts` from the Rust types.

use ts_rs::{Config, TS};

use crate::error::{ErrorData, ErrorKind};
use crate::events::*;
use crate::ids::*;
use crate::methods::*;
use crate::model::*;
use crate::rpc::codes;
use crate::{API_VERSION, methods};

/// Collects `export type ...` declarations in a fixed order.
struct Decls<'a> {
    cfg: &'a Config,
    out: Vec<String>,
}

impl Decls<'_> {
    fn add<T: TS + ?Sized>(&mut self) -> &mut Self {
        let mut decl = String::new();
        if let Some(docs) = T::docs() {
            decl.push_str(&docs);
        }
        decl.push_str("export ");
        decl.push_str(&T::decl(self.cfg));
        self.out.push(decl);
        self
    }
}

/// The full contents of the generated TypeScript module.
pub fn generate() -> String {
    let cfg = Config::new().with_large_int("number");
    let mut d = Decls {
        cfg: &cfg,
        out: Vec::new(),
    };

    d.add::<ProjectId>()
        .add::<SessionId>()
        .add::<TurnId>()
        .add::<CheckpointId>()
        .add::<ApprovalId>()
        .add::<ThreadId>()
        .add::<CommentId>()
        .add::<HunkId>()
        .add::<SubscriptionId>()
        .add::<crate::Timestamp>();

    d.add::<ErrorKind>().add::<ErrorData>();

    d.add::<Mode>()
        .add::<SandboxTier>()
        .add::<SessionState>()
        .add::<Platform>()
        .add::<Project>()
        .add::<Repo>()
        .add::<AgentProfile>()
        .add::<Usage>()
        .add::<Session>()
        .add::<EditKind>()
        .add::<Action>()
        .add::<Approval>()
        .add::<Hunk>()
        .add::<FileStatus>()
        .add::<FileChange>()
        .add::<Changeset>()
        .add::<LineRange>()
        .add::<ThreadStatus>()
        .add::<Thread>()
        .add::<CommentAuthor>()
        .add::<Comment>();

    d.add::<Event>()
        .add::<EventPayload>()
        .add::<EventKind>()
        .add::<SessionStateBody>()
        .add::<TurnSource>()
        .add::<TurnStartedBody>()
        .add::<TurnEndedBody>()
        .add::<PromptBody>()
        .add::<AgentMessageBody>()
        .add::<PlanEntry>()
        .add::<AgentPlanBody>()
        .add::<ToolCallBody>()
        .add::<ToolResultBody>()
        .add::<Verdict>()
        .add::<PolicyDecisionBody>()
        .add::<HookRunBody>()
        .add::<ApprovalRequestedBody>()
        .add::<ApprovalDecision>()
        .add::<Remember>()
        .add::<ApprovalDecidedBody>()
        .add::<EditBody>()
        .add::<TerminalOutputBody>()
        .add::<CheckpointBody>()
        .add::<ThreadChange>()
        .add::<ReviewThreadBody>()
        .add::<Topic>()
        .add::<EventNotification>();

    d.add::<Empty>()
        .add::<HelloParams>()
        .add::<HelloResult>()
        .add::<ProjectOpenParams>()
        .add::<ProjectGetParams>()
        .add::<SessionCreateParams>()
        .add::<CollisionResolution>()
        .add::<BranchPreviewParams>()
        .add::<BranchPreview>()
        .add::<BranchCollision>()
        .add::<CollisionKind>()
        .add::<SessionPromptParams>()
        .add::<TurnResult>()
        .add::<SessionIdParams>()
        .add::<SessionSetModeParams>()
        .add::<SessionRewindParams>()
        .add::<SessionListParams>()
        .add::<SessionOutcome>()
        .add::<SessionCloseParams>()
        .add::<ApprovalListParams>()
        .add::<ApprovalDecideParams>()
        .add::<ReviewChangesetParams>()
        .add::<ReviewFileDiffParams>()
        .add::<FileDiffResult>()
        .add::<ReviewCommentParams>()
        .add::<ReviewReplyParams>()
        .add::<ThreadIdParams>()
        .add::<ReviewThreadsParams>()
        .add::<ReviewRevertHunkParams>()
        .add::<ShipStrategy>()
        .add::<ReviewShipParams>()
        .add::<ShipResult>()
        .add::<FileReadParams>()
        .add::<FileReadResult>()
        .add::<FileWriteParams>()
        .add::<FileWriteResult>()
        .add::<LspRequestParams>()
        .add::<LspResult>()
        .add::<EventsSubscribeParams>()
        .add::<SubscribeResult>()
        .add::<EventsUnsubscribeParams>()
        .add::<EventsQueryParams>();

    let mut out = String::new();
    out.push_str("// Generated from crates/forge-api by `just gen`. Do not edit.\n\n");
    out.push_str(&format!("export const API_VERSION = {API_VERSION:?};\n\n"));
    out.push_str(&format!(
        "export const EVENT_NOTIFICATION = {EVENT_NOTIFICATION:?};\n\n"
    ));

    out.push_str("/** JSON-RPC error code for each application error kind. */\n");
    out.push_str("export const ERROR_CODES: Record<ErrorKind, number> = {\n");
    for kind in ErrorKind::ALL {
        out.push_str(&format!("  {}: {},\n", kind.name(), kind.code()));
    }
    out.push_str("};\n\n");

    out.push_str("/** Standard JSON-RPC codes for protocol-level failures. */\n");
    out.push_str("export const RPC_CODES = {\n");
    out.push_str(&format!("  parseError: {},\n", codes::PARSE_ERROR));
    out.push_str(&format!("  invalidRequest: {},\n", codes::INVALID_REQUEST));
    out.push_str(&format!("  methodNotFound: {},\n", codes::METHOD_NOT_FOUND));
    out.push_str(&format!("  internalError: {},\n", codes::INTERNAL_ERROR));
    out.push_str("} as const;\n\n");

    for decl in &d.out {
        out.push_str(decl);
        out.push_str("\n\n");
    }

    out.push_str("/** Params and result of every daemon method, keyed by method name. */\n");
    out.push_str("export type Methods = {\n");
    for entry in methods::method_map_entries(&cfg) {
        out.push_str(&entry);
        out.push('\n');
    }
    out.push_str("};\n\n");
    out.push_str("export type MethodName = keyof Methods;\n");
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn declares_every_method() {
        let ts = super::generate();
        for name in crate::methods::METHOD_NAMES {
            assert!(
                ts.contains(&format!("{name:?}: {{ params:")),
                "missing {name}"
            );
        }
        assert!(ts.contains("export type Project = "));
        assert!(ts.contains("NotFound: -32001"));
    }
}
