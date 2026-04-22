# ADR-003: Sandbox Resource Enforcement — cgroup v2 / rlimit Hybrid

**Status:** Accepted
**Date:** 2026-04-21

---

## Context

`forged` spawns tool subprocesses (shell, fs, LSP helpers, user-approved commands) under the sandbox wrapper in `crates/forge-session/src/sandbox.rs`. Phase 2 (F-149) replaced the historical `RLIMIT_NPROC`-only design with a cgroup v2 primary path plus an rlimit backstop. The three decisions below pin that hybrid so future contributors do not regress to rlimit-only or move enrollment back to the parent side.

Cross-references: `crate-architecture.md §3.5` (forge-session responsibilities), `isolation-model.md §8.2` (Level 1 process isolation).

---

## Decisions

### 1. Per-sandbox cgroup v2 leaf is the authoritative process cap

**Decision.** Each `SandboxedCommand::spawn` creates a fresh cgroup v2 leaf as a sibling of the daemon's own cgroup, writes `pids.max = SandboxConfig::max_processes`, and enrolls the child pid into it. Each sandbox gets its own independent task budget (default 256).

**Rationale.** `RLIMIT_NPROC` is per real-uid, not per-process-tree. On a developer machine the daemon shares a uid with the desktop session and every other process that uid owns; on CI it shares a uid with every other concurrent test. A uid-wide cap cannot give sandbox A a budget of 256 that is independent of sandbox B's budget — the two share the same counter. cgroup v2 `pids.max` is per-cgroup, so a per-sandbox leaf is the only mechanism that gives each sandbox an independent task ceiling.

Sibling-of-daemon placement is deliberate: cgroup v2 forbids a cgroup from containing both processes and child cgroups (the "no internal processes" rule), so nesting leaves under the daemon's own cgroup would violate the rule the moment `pids` is enabled on its subtree. The daemon's parent (typically a systemd `user@<uid>.service/app.slice` path) already has `pids` in `cgroup.subtree_control` thanks to systemd's default delegation, so sibling leaves inherit the controller for free.

See `crates/forge-session/src/sandbox.rs:59-82` for the `SandboxConfig` field-level docs that tie `max_processes` to this mechanism.

---

### 2. Enrollment happens in `pre_exec`, not after `spawn()` returns

**Decision.** The pid is written into `<leaf>/cgroup.procs` from inside a `pre_exec` hook — i.e. from the child process, post-fork but pre-exec — using only async-signal-safe libc calls (`open`, `write`, `close`, `getpid`). A parent-side re-enroll runs after `spawn()` returns as a belt-and-braces backstop.

**Rationale.** A naive implementation would `spawn()` the child and then write its pid into `cgroup.procs` from the parent. That has a fork-escape race: between the moment the kernel returns the child pid to the parent and the moment the parent writes the pid into the leaf, the child can execute arbitrarily many instructions — including `fork()` / `clone()` — and any descendants it spawns before enrollment lands inherit the *parent's* cgroup, not the sandbox leaf. A fork bomb in the first few microseconds of child execution would slip past the cap.

`pre_exec` runs in the child process after `fork()` but before `execve()`. Writing `getpid()` into `cgroup.procs` at that point atomically enrolls the child before it can call `execve` to run user code, let alone fork descendants. All descendants of the child then inherit the sandbox cgroup.

The hook uses only async-signal-safe calls because `pre_exec` runs in the window between `fork` and `execve` where allocator state, locks, and anything not signal-safe is undefined. `format!`, `to_string`, and even `std::fs::write` are unsafe to call there — they allocate or take locks. The hook pre-computes the C-string path outside `pre_exec`, then uses raw libc `open`/`write`/`close` and a hand-rolled `pid_to_decimal` (no allocation, no locks, no locale) inside.

The parent-side re-enroll after `spawn()` returns is a backstop for the narrow case where the pre_exec `open()` fails (e.g., EMFILE under extreme fd pressure). It catches the child but cannot catch any descendants already forked by that point — pre_exec is the load-bearing path, parent-side is a safety net.

See `crates/forge-session/src/sandbox.rs:59-82` and the full `SandboxedCommand::spawn` body for the enrollment sequence and race-analysis comments.

---

### 3. `RLIMIT_NPROC` stays as a uid-wide backstop when cgroup delegation is absent

**Decision.** The sandbox applies `setrlimit(RLIMIT_NPROC, rlimit_nproc_backstop)` — default 4096 — in `pre_exec` on every spawn. When `CgroupLeaf::create` returns `Ok(None)` (non-Linux host, cgroup v1, container without `pids` delegation, user-slice without delegation), the sandbox proceeds with rlimit-only enforcement instead of failing the spawn.

**Rationale.** The cgroup path is the primary enforcement mechanism but it is not universally available:

- Non-Linux hosts do not have cgroups at all (the `imp` submodule itself is `#[cfg(target_os = "linux")]`).
- Some Linux hosts run cgroup v1 or a hybrid layout where v2's `pids` controller is not exposed.
- Container environments (docker / podman without `--pids=host`, some Kubernetes pod configurations) expose cgroup v2 but do not delegate the `pids` controller to user slices.
- Developers running `forged` directly outside systemd's user manager may end up in the root cgroup, which has no writable subtree.

In all of these cases, failing the spawn would make `forged` unusable on environments where users reasonably expect it to work (CI containers, older distros, macOS developer machines under eventual macOS support). The rlimit backstop is not equivalent to the cgroup cap — it is uid-wide, so its numeric value (4096) is tuned to stop runaway fork bombs within milliseconds while leaving headroom for the uid's baseline process count, not to provide per-sandbox isolation. Consumers that need hard per-sandbox isolation check `SandboxedChild::cgroup_path().is_some()` and refuse to run untrusted workloads on hosts where it is `None`.

Graceful degradation is silent because the user cannot meaningfully act on a cgroup-not-available log spam on every spawn; `cgroup_path()` is the introspection hook for code that needs to know.

See `crates/forge-session/src/sandbox.rs:59-82` for the `SandboxConfig::rlimit_nproc_backstop` field doc that pins this rationale to the type.

---

## Consequences

- The regression test `cgroup_pids_max_caps_sandbox_tasks_per_f149` skips on hosts without `pids` delegation rather than silently exercising the rlimit-only path — silent success on a degraded path would let a real regression hide.
- Teardown of the leaf is best-effort: `SandboxedChild::drop` writes `cgroup.kill` + `rmdir`; orphans from crash paths are reclaimed on reboot. Non-empty leaves EBUSY on rmdir; we accept the orphan rather than busy-wait.
- The cgroup leaf naming convention `forge-sandbox-<daemon-pid>-<counter>` is unique within a daemon lifetime but not across daemon restarts. Leaves from a crashed prior daemon process are inert (no processes inside) and cleaned by the next kernel reboot.
- Any future addition to `SandboxConfig` that needs per-sandbox (not per-uid) enforcement must follow the cgroup path; adding more `setrlimit` calls will not deliver per-sandbox isolation.
- macOS / Windows support for real sandboxing is deferred; the bookkeeping types (`SandboxConfig`, `ChildRegistry`, `BASE_ENV_WHITELIST`) compile everywhere so callers can plumb config through without platform branching.
