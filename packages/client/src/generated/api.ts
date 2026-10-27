// Generated from crates/forge-api by `just gen`. Do not edit.

export const API_VERSION = "0.5.0";

export const EVENT_NOTIFICATION = "event";

/** JSON-RPC error code for each application error kind. */
export const ERROR_CODES: Record<ErrorKind, number> = {
  NotFound: -32001,
  InvalidState: -32002,
  PolicyDenied: -32003,
  AgentUnavailable: -32004,
  SandboxUnavailable: -32005,
  Conflict: -32006,
  Unauthorized: -32007,
  ValidationFailed: -32008,
};

/** Standard JSON-RPC codes for protocol-level failures. */
export const RPC_CODES = {
  parseError: -32700,
  invalidRequest: -32600,
  methodNotFound: -32601,
  internalError: -32603,
} as const;

/**
 * A project.
 */
export type ProjectId = string;

/**
 * A session.
 */
export type SessionId = string;

/**
 * An agent turn.
 */
export type TurnId = string;

/**
 * A worktree checkpoint.
 */
export type CheckpointId = string;

/**
 * A pending or decided approval.
 */
export type ApprovalId = string;

/**
 * A review thread.
 */
export type ThreadId = string;

/**
 * A comment in a review thread.
 */
export type CommentId = string;

/**
 * A diff hunk.
 */
export type HunkId = string;

/**
 * An event subscription on one connection.
 */
export type SubscriptionId = string;

/**
 * A point in time, always UTC on the wire.
 */
export type Timestamp = string;

/**
 * The kind of an application error. Each maps to one JSON-RPC error code.
 */
export type ErrorKind =
  | "NotFound"
  | "InvalidState"
  | "PolicyDenied"
  | "AgentUnavailable"
  | "SandboxUnavailable"
  | "Conflict"
  | "Unauthorized"
  | "ValidationFailed";

/**
 * The structured `data` field of an application error.
 */
export type ErrorData = { kind: ErrorKind; detail: string; retryable: boolean };

/**
 * How much the agent may do without asking.
 */
export type Mode = "ask" | "accept-edits" | "auto" | "full-access";

/**
 * Isolation tier for a session's agent process.
 */
export type SandboxTier = "light" | "standard" | "container" | "unsandboxed";

/**
 * Session lifecycle state (component-architecture.md, Session manager).
 */
export type SessionState =
  "provisioning" | "ready" | "working" | "waiting" | "paused" | "review" | "done" | "failed";

/**
 * Operating system the daemon runs on.
 */
export type Platform = "linux" | "macos";

/**
 * An opened project. `root` is the one absolute path that crosses the API.
 */
export type Project = { id: ProjectId; name: string; root: string; repos: Array<Repo> };

/**
 * A repo within a project; `path` is relative to the project root.
 */
export type Repo = { name: string; path: string; baseBranch: string };

export type AgentProfile = {
  id: string;
  /**
   * Registry key: `claude-code`, `codex`, `gemini`, or another adapter.
   */
  agent: string;
  installed: boolean;
  /**
   * Every secret the profile names is in the keyring, or for a registry agent with none
   * of them there, its own subscription sign-in is present and unexpired.
   */
  authenticated: boolean;
  /**
   * The installed version, when Forge can tell.
   */
  version?: string;
  /**
   * The agent's display name, for a registry agent.
   */
  name?: string;
  /**
   * The version Forge is tested with, for a registry agent.
   */
  supportedVersion?: string;
  /**
   * The command that installs the supported version, when the agent isn't installed or
   * is at another version.
   */
  installCommand?: string;
  /**
   * How to sign in, when the agent isn't signed in: the command that stores each missing
   * secret in the keyring, or the agent's own command that signs in with a subscription.
   */
  signIn?: string;
  /**
   * Why a session with this profile can't start, when Forge can tell before starting one
   * (an invalid profile file, an unknown agent).
   */
  problem?: string;
};

/**
 * A way an agent offers to sign in, as it advertises it over ACP.
 */
export type SignInMethod = {
  id: string;
  name: string;
  description?: string;
  /**
   * For a method the agent runs in a terminal: the whole command, which the client runs
   * interactively outside the sandbox, as the user would. Without it, `agent.signIn` runs
   * the method, and the agent does the flow itself (a browser, say).
   */
  terminal?: TerminalSignIn;
};

export type TerminalSignIn = {
  argv: Array<string>;
  /**
   * Added to the client's own environment.
   */
  env: Array<[string, string]>;
};

export type Usage = { inputTokens: number; outputTokens: number; costUsd?: number };

export type Session = {
  id: SessionId;
  projectId: ProjectId;
  title: string;
  agentProfile: string;
  state: SessionState;
  mode: Mode;
  sandbox: SandboxTier;
  branch: string;
  createdAt: Timestamp;
  updatedAt: Timestamp;
  usage: Usage;
};

export type EditKind = "create" | "modify" | "delete";

/**
 * Something an agent wants to do; every action gets exactly one policy decision.
 */
export type Action =
  | { type: "edit"; path: string; kind: EditKind }
  | { type: "exec"; argv: Array<string>; cwd: string }
  | { type: "network"; host: string; port: number }
  | { type: "mcp"; server: string; tool: string; args: unknown }
  | { type: "git"; op: string };

export type Approval = {
  id: ApprovalId;
  sessionId: SessionId;
  action: Action;
  reason: string;
  createdAt: Timestamp;
};

export type Hunk = {
  id: HunkId;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: Array<string>;
  turnId?: TurnId;
};

export type FileStatus = "added" | "modified" | "deleted" | "renamed";

export type FileChange = {
  /**
   * The repo the file is in, by its name in `forge.toml`.
   */
  repo?: string;
  path: string;
  status: FileStatus;
  oldPath?: string;
  /**
   * Each side's mode, in octal as git prints it (`100644`, `100755`, `120000` for a
   * symlink, `160000` for a submodule), when the file is on that side.
   */
  oldMode?: string;
  newMode?: string;
  additions: number;
  deletions: number;
  /**
   * A side held a NUL byte or bytes that aren't UTF-8. Its hunks and counts are still
   * there, with each such byte shown as U+FFFD; clients collapse it by default.
   */
  binary: boolean;
  /**
   * A side is over 16 MiB or 1,000,000 lines: no line counts, and `review.fileDiff`
   * refuses it. Clients show it as a warning, never as an unchanged-looking 0/0.
   */
  tooLarge: boolean;
};

export type Changeset = {
  sessionId: SessionId;
  base: string;
  head: string;
  files: Array<FileChange>;
};

/**
 * Inclusive, 1-based line range.
 */
export type LineRange = { startLine: number; endLine: number };

export type ThreadStatus = "open" | "resolved" | "outdated";

export type Thread = {
  id: ThreadId;
  sessionId: SessionId;
  path: string;
  range: LineRange;
  checkpointId: CheckpointId;
  status: ThreadStatus;
  comments: Array<Comment>;
};

export type CommentAuthor = "human" | "agent";

export type Comment = { id: CommentId; author: CommentAuthor; body: string; createdAt: Timestamp };

/**
 * One entry in a session's event log.
 */
export type Event = {
  /**
   * Per-session, gap-free.
   */
  seq: number;
  sessionId: SessionId;
  at: Timestamp;
  /**
   * Links a decision, hook run, or result to the tool call it belongs to.
   */
  parent?: number;
} & (
  | { kind: "session.state"; body: SessionStateBody }
  | { kind: "turn.started"; body: TurnStartedBody }
  | { kind: "turn.ended"; body: TurnEndedBody }
  | { kind: "prompt"; body: PromptBody }
  | { kind: "agent.message"; body: AgentMessageBody }
  | { kind: "agent.plan"; body: AgentPlanBody }
  | { kind: "tool.call"; body: ToolCallBody }
  | { kind: "tool.result"; body: ToolResultBody }
  | { kind: "policy.decision"; body: PolicyDecisionBody }
  | { kind: "hook.run"; body: HookRunBody }
  | { kind: "approval.requested"; body: ApprovalRequestedBody }
  | { kind: "approval.decided"; body: ApprovalDecidedBody }
  | { kind: "edit"; body: EditBody }
  | { kind: "terminal.output"; body: TerminalOutputBody }
  | { kind: "network"; body: NetworkBody }
  | { kind: "checkpoint"; body: CheckpointBody }
  | { kind: "session.rewound"; body: SessionRewoundBody }
  | { kind: "review.thread"; body: ReviewThreadBody }
  | { kind: "usage"; body: Usage }
  | { kind: "error"; body: ErrorData }
);

/**
 * An event's `kind` and `body`.
 */
export type EventPayload =
  | { kind: "session.state"; body: SessionStateBody }
  | { kind: "turn.started"; body: TurnStartedBody }
  | { kind: "turn.ended"; body: TurnEndedBody }
  | { kind: "prompt"; body: PromptBody }
  | { kind: "agent.message"; body: AgentMessageBody }
  | { kind: "agent.plan"; body: AgentPlanBody }
  | { kind: "tool.call"; body: ToolCallBody }
  | { kind: "tool.result"; body: ToolResultBody }
  | { kind: "policy.decision"; body: PolicyDecisionBody }
  | { kind: "hook.run"; body: HookRunBody }
  | { kind: "approval.requested"; body: ApprovalRequestedBody }
  | { kind: "approval.decided"; body: ApprovalDecidedBody }
  | { kind: "edit"; body: EditBody }
  | { kind: "terminal.output"; body: TerminalOutputBody }
  | { kind: "network"; body: NetworkBody }
  | { kind: "checkpoint"; body: CheckpointBody }
  | { kind: "session.rewound"; body: SessionRewoundBody }
  | { kind: "review.thread"; body: ReviewThreadBody }
  | { kind: "usage"; body: Usage }
  | { kind: "error"; body: ErrorData };

/**
 * Event kinds without bodies, for filters such as `events.query { kinds }`.
 */
export type EventKind =
  | "session.state"
  | "turn.started"
  | "turn.ended"
  | "prompt"
  | "agent.message"
  | "agent.plan"
  | "tool.call"
  | "tool.result"
  | "policy.decision"
  | "hook.run"
  | "approval.requested"
  | "approval.decided"
  | "edit"
  | "terminal.output"
  | "network"
  | "checkpoint"
  | "session.rewound"
  | "review.thread"
  | "usage"
  | "error";

export type SessionStateBody = { from: SessionState; to: SessionState; reason?: string };

export type TurnSource = "human" | "review" | "hook";

export type TurnStartedBody = { turnId: TurnId; source: TurnSource };

export type TurnEndedBody = { turnId: TurnId; stopReason: string };

export type PromptBody = { turnId: TurnId; text: string };

export type AgentMessageBody = { turnId: TurnId; messageId: string; delta: string };

export type PlanEntry = { content: string; status: string };

export type AgentPlanBody = { entries: Array<PlanEntry> };

export type ToolCallBody = {
  toolCallId: string;
  action: Action;
  title: string;
  /**
   * Who decided the call. Events recorded before it existed read as Forge.
   */
  source: ToolCallSource;
};

/**
 * Who decided a tool call.
 */
export type ToolCallSource = "forge" | "agent";

export type ToolResultBody = {
  toolCallId: string;
  status: string;
  /**
   * Truncated and redacted.
   */
  output?: string;
};

export type Verdict = "allow" | "deny" | "ask";

export type PolicyDecisionBody = {
  toolCallId: string;
  verdict: Verdict;
  reason: string;
  source: string;
};

export type HookRunBody = {
  hook: string;
  event: string;
  verdict?: Verdict;
  durationMs: number;
  exitCode: number;
};

export type ApprovalRequestedBody = { approvalId: ApprovalId; action: Action; reason: string };

export type ApprovalDecision = "allow" | "deny";

/**
 * How long an approval decision is remembered.
 */
export type Remember = "session" | "project";

export type ApprovalDecidedBody = {
  approvalId: ApprovalId;
  decision: ApprovalDecision;
  remember?: Remember;
};

export type EditBody = { path: string; kind: EditKind; additions: number; deletions: number };

export type TerminalOutputBody = { terminalId: string; chunk: string };

/**
 * One connection through the session's egress proxy, logged when it closes, or at once when
 * it is denied.
 */
export type NetworkBody = {
  host: string;
  port: number;
  /**
   * `allow` or `deny`; the proxy never asks.
   */
  verdict: Verdict;
  reason?: string;
  bytesSent: number;
  bytesReceived: number;
  durationMs: number;
};

export type CheckpointBody = {
  checkpointId: CheckpointId;
  /**
   * The turn it was taken after; absent for one Forge took between turns (before a
   * rewind, say).
   */
  turnId?: TurnId;
  ref: string;
};

/**
 * The worktree went back to a checkpoint; the turns after it were discarded.
 */
export type SessionRewoundBody = {
  checkpointId: CheckpointId;
  /**
   * The turns whose changes are gone, oldest first.
   */
  discardedTurns: Array<TurnId>;
};

export type ThreadChange = "created" | "reply" | "resolved" | "outdated";

export type ReviewThreadBody = { threadId: ThreadId; change: ThreadChange };

/**
 * A subscription topic: `board`, `approvals`, `session/<id>`, or `project/<id>`.
 */
export type Topic = "board" | "approvals" | `session/${string}` | `project/${string}`;

/**
 * Params of the `event` notification the daemon pushes to subscribers.
 *
 * Each event reaches a subscription at most once, even when several of its topics match, so
 * `seq` strictly increases within a subscription.
 */
export type EventNotification = {
  subscriptionId: SubscriptionId;
  /**
   * The first of the event's topics that the subscription includes.
   */
  topic: Topic;
  /**
   * The daemon-wide stream cursor: increases across every session and topic and survives
   * restarts. `events.subscribe { afterSeq }` resumes after it. Not the event's own `seq`,
   * which counts within its session.
   */
  seq: number;
  event: Event;
};

/**
 * Params or result written as `{}` on the wire.
 */
export type Empty = Record<string, never>;

export type HelloParams = {
  /**
   * Free-form client name, for logs: `forge-cli`, `forge-desktop`.
   */
  client: string;
  /**
   * The API version the client was built against.
   */
  apiVersion: string;
};

export type HelloResult = {
  daemonVersion: string;
  apiVersion: string;
  platform: Platform;
  sandboxTiers: Array<SandboxTier>;
};

export type ProjectOpenParams = {
  /**
   * Absolute path to the project root. Clients resolve relative paths first.
   */
  path: string;
};

export type ProjectGetParams = { projectId: ProjectId };

export type AgentListParams = {
  /**
   * List this project's profiles: the built-in ones and its `.forge/agents/` files. Without
   * it, only the built-in profiles.
   */
  projectId?: ProjectId;
};

/**
 * A registry agent, for signing in. Only registry agents sign in: signing in runs the agent
 * outside the sandbox, so never a command from a project's profile file.
 */
export type AgentRefParams = {
  /**
   * Registry key: `claude-code`, `codex`, `gemini`, or another adapter.
   */
  agent: string;
};

export type AgentSignInParams = {
  agent: string;
  /**
   * One of the agent's non-terminal sign-in methods, from `agent.signInMethods`.
   */
  methodId: string;
};

export type SessionCreateParams = {
  projectId: ProjectId;
  agentProfile: string;
  title: string;
  prompt: string;
  mode: Mode;
  sandbox?: SandboxTier;
  /**
   * Full branch name; derived from `title` and the configured format when omitted.
   */
  branch?: string;
  /**
   * What to do when the branch or worktree name is already taken. Without it, a collision
   * fails with `Conflict` so the client can ask the user.
   */
  onCollision?: CollisionResolution;
};

/**
 * The user's answer to a branch or worktree name collision.
 */
export type CollisionResolution = "reuse-branch" | "adopt-worktree";

export type BranchPreviewParams = { projectId: ProjectId; title: string; branch?: string };

/**
 * The branch a new session would use, and what already holds that name.
 */
export type BranchPreview = { branch: string; collisions: Array<BranchCollision> };

export type BranchCollision = {
  /**
   * Repo name from `forge.toml`.
   */
  repo: string;
  kind: CollisionKind;
  /**
   * The live session that owns the worktree, for `owned-by-live-session`.
   */
  sessionId?: SessionId;
};

export type CollisionKind = "branch-exists" | "worktree-exists" | "owned-by-live-session";

export type SessionPromptParams = { sessionId: SessionId; text: string };

export type TurnResult = { turnId: TurnId };

export type SessionIdParams = { sessionId: SessionId };

export type SessionSetModeParams = { sessionId: SessionId; mode: Mode; sandbox?: SandboxTier };

export type SessionRewindParams = { sessionId: SessionId; checkpointId: CheckpointId };

export type SessionListParams = { projectId: ProjectId; states?: Array<SessionState> };

export type SessionOutcome = "shipped" | "abandoned";

export type SessionCloseParams = { sessionId: SessionId; outcome: SessionOutcome };

export type ApprovalListParams = { projectId?: ProjectId };

export type ApprovalDecideParams = {
  approvalId: ApprovalId;
  decision: ApprovalDecision;
  remember?: Remember;
  note?: string;
};

export type ReviewChangesetParams = { sessionId: SessionId; fromCheckpoint?: CheckpointId };

export type ReviewFileDiffParams = {
  sessionId: SessionId;
  /**
   * The repo the file is in; may be left out when the session has only one.
   */
  repo?: string;
  path: string;
  fromCheckpoint?: CheckpointId;
};

export type FileDiffResult = { hunks: Array<Hunk> };

export type ReviewCommentParams = {
  sessionId: SessionId;
  path: string;
  range: LineRange;
  body: string;
};

export type ReviewReplyParams = { threadId: ThreadId; body: string };

export type ThreadIdParams = { threadId: ThreadId };

export type ReviewThreadsParams = { sessionId: SessionId; status?: ThreadStatus };

export type ReviewRevertHunkParams = { sessionId: SessionId; path: string; hunkId: HunkId };

export type ShipStrategy = "push-branch" | "merge-local";

export type ReviewShipParams = { sessionId: SessionId; strategy: ShipStrategy; branch?: string };

export type ShipResult = { ref: string };

export type FileReadParams = { projectId: ProjectId; path: string; sessionId?: SessionId };

export type FileReadResult = { content: string; hash: string };

export type FileWriteParams = {
  projectId: ProjectId;
  path: string;
  content: string;
  baseHash: string;
  sessionId?: SessionId;
};

export type FileWriteResult = { hash: string };

export type LspRequestParams = {
  projectId: ProjectId;
  sessionId?: SessionId;
  method: string;
  params: unknown;
};

/**
 * A raw LSP result, passed through unchanged.
 */
export type LspResult = unknown;

export type EventsSubscribeParams = { topics: Array<Topic>; afterSeq?: number };

export type SubscribeResult = { subscriptionId: SubscriptionId };

export type EventsUnsubscribeParams = { subscriptionId: SubscriptionId };

export type EventsQueryParams = {
  sessionId: SessionId;
  kinds?: Array<EventKind>;
  text?: string;
  /**
   * Only events with a lower `seq`.
   */
  before?: number;
  limit: number;
};

/** Params and result of every daemon method, keyed by method name. */
export type Methods = {
  "server.hello": { params: HelloParams; result: HelloResult };
  "project.open": { params: ProjectOpenParams; result: Project };
  "project.list": { params: Empty; result: Array<Project> };
  "project.get": { params: ProjectGetParams; result: Project };
  "agent.list": { params: AgentListParams; result: Array<AgentProfile> };
  "agent.signInMethods": { params: AgentRefParams; result: Array<SignInMethod> };
  "agent.signIn": { params: AgentSignInParams; result: Empty };
  "session.create": { params: SessionCreateParams; result: Session };
  "session.prompt": { params: SessionPromptParams; result: TurnResult };
  "session.cancel": { params: SessionIdParams; result: Empty };
  "session.setMode": { params: SessionSetModeParams; result: Session };
  "session.rewind": { params: SessionRewindParams; result: Session };
  "session.get": { params: SessionIdParams; result: Session };
  "session.list": { params: SessionListParams; result: Array<Session> };
  "session.close": { params: SessionCloseParams; result: Empty };
  "session.branchPreview": { params: BranchPreviewParams; result: BranchPreview };
  "approval.list": { params: ApprovalListParams; result: Array<Approval> };
  "approval.decide": { params: ApprovalDecideParams; result: Empty };
  "review.changeset": { params: ReviewChangesetParams; result: Changeset };
  "review.fileDiff": { params: ReviewFileDiffParams; result: FileDiffResult };
  "review.comment": { params: ReviewCommentParams; result: Thread };
  "review.reply": { params: ReviewReplyParams; result: Thread };
  "review.resolve": { params: ThreadIdParams; result: Thread };
  "review.threads": { params: ReviewThreadsParams; result: Array<Thread> };
  "review.requestChanges": { params: SessionIdParams; result: TurnResult };
  "review.revertHunk": { params: ReviewRevertHunkParams; result: Empty };
  "review.ship": { params: ReviewShipParams; result: ShipResult };
  "file.read": { params: FileReadParams; result: FileReadResult };
  "file.write": { params: FileWriteParams; result: FileWriteResult };
  "lsp.request": { params: LspRequestParams; result: LspResult };
  "events.subscribe": { params: EventsSubscribeParams; result: SubscribeResult };
  "events.unsubscribe": { params: EventsUnsubscribeParams; result: Empty };
  "events.query": { params: EventsQueryParams; result: Array<Event> };
};

export type MethodName = keyof Methods;
