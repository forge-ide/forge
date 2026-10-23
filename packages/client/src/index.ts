// Typed daemon client. Types under ./generated/ come from forge-api via `just gen`.
export * from "./generated/api";
export { DaemonError, RpcClient, type Params, type Result } from "./rpc";
