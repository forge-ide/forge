export type { AgentId } from './generated/AgentId';
export type { AgentInstanceId } from './generated/AgentInstanceId';
export type { ApprovalConfig } from './generated/ApprovalConfig';
export type { BgAgentStateDto } from './generated/BgAgentStateDto';
export type { BgAgentSummary } from './generated/BgAgentSummary';
export type { ApprovalEntry } from './generated/ApprovalEntry';
export type { ApprovalLevel } from './generated/ApprovalLevel';
export type { ApprovalScope } from './generated/ApprovalScope';
export type { CompactTrigger } from './generated/CompactTrigger';
export type { FileContent } from './generated/FileContent';
export type { Layout } from './generated/Layout';
export type { Layouts } from './generated/Layouts';
export type { LayoutTree } from './generated/LayoutTree';
export type { MessageId } from './generated/MessageId';
export type { PaneState } from './generated/PaneState';
export type { PaneType } from './generated/PaneType';
export type { PersistentApprovalEntry } from './generated/PersistentApprovalEntry';
export type { ProviderId } from './generated/ProviderId';
export type { RerunVariant } from './generated/RerunVariant';
export type { RosterScope } from './generated/RosterScope';
export type { SessionId } from './generated/SessionId';
export type { SessionPersistence } from './generated/SessionPersistence';
export type { SessionState } from './generated/SessionState';
export type { SplitDirection } from './generated/SplitDirection';
export type { TerminalBytesEvent } from './generated/TerminalBytesEvent';
export type { TerminalExitEvent } from './generated/TerminalExitEvent';
export type { TerminalId } from './generated/TerminalId';
export type { TerminalSpawnArgs } from './generated/TerminalSpawnArgs';
export type { ToolCallId } from './generated/ToolCallId';
export type { TreeKindDto } from './generated/TreeKindDto';
export type { TreeNodeDto } from './generated/TreeNodeDto';
export type { WorkspaceId } from './generated/WorkspaceId';

// F-142: context-adapter lives in the IPC package so both the app's send-time
// wiring and any future shell-side compactor share a single serialization
// contract for `ContextBlock[]`.
export {
  adaptContextBlocks,
  providerFlavour,
  toAnthropicXml,
  toOpenAiFunctionContext,
} from './context-adapter';
export type {
  ContextBlock,
  ContextBlockType,
  ProviderFlavour,
} from './context-adapter';
