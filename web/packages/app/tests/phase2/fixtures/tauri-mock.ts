// Phase 2 reuses the Phase 1 mocked-Tauri-IPC fixture as-is. The contract
// (onInvoke, emit, calls, reset) is sufficient for the deterministic-state
// induction the Phase 2 mocked-IPC specs need (UAT-09, UAT-11). Real-shell
// UATs (UAT-01/02/03/05/06/07/08/12) need `tauri-driver` and live outside
// this fixture entirely.

export { test, expect, installTauriMock, type TauriMockHandle, type InvokeHandler } from '../../phase1/fixtures/tauri-mock';
