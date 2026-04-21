# Sandbox resource limits

Operator reference for the process-isolation caps that `crates/forge-session/src/sandbox.rs::SandboxedCommand` applies to every approved tool invocation. Four caps come from `setrlimit(2)`, a fifth rlimit is a uid-wide defense-in-depth backstop, and the authoritative **per-sandbox** process budget lives on the cgroup v2 `pids` controller (F-149). For the security context that motivates these caps, see [`security.md`](security.md#sandbox-resource-limits-f-055); this page is the full operator-facing reference the security page links out to.

## The caps at a glance

Defaults come from `SandboxConfig::default`. rlimit soft and hard limits are set to the same value, so a sandboxed child cannot raise them.

| Resource | Mechanism | Default | Threat mitigated | Override field |
|---|---|---|---|---|
| CPU time | `RLIMIT_CPU` | 30 s | runaway CPU on a single tool call (SIGXCPU) | `cpu_seconds` |
| Address space | `RLIMIT_AS` | 512 MiB | address-space exhaustion / large allocations | `address_space_bytes` |
| **Per-sandbox tasks** | **cgroup v2 `pids.max`** | **256** | **fork bombs / tool fan-out abuse (per-sandbox scope)** | **`max_processes`** |
| Uid-wide processes | `RLIMIT_NPROC` | 4096 | backstop for non-cgroup hosts (uid-wide scope) | `rlimit_nproc_backstop` |
| Open files | `RLIMIT_NOFILE` | 256 | fd-table exhaustion | `max_open_files` |
| File size | `RLIMIT_FSIZE` | 100 MiB | cat-to-disk attacks (SIGXFSZ on overflow) | `max_file_size_bytes` |

The `rlimits_bound_child_via_setrlimit` test in `crates/forge-session/src/sandbox.rs` probes `/proc/self/limits` from inside the sandbox to confirm `pre_exec` actually applied the rlimit values — that test is the load-bearing regression for F-055. The `cgroup_pids_max_caps_sandbox_tasks_per_f149` test reads `pids.current` / `pids.max` directly from the leaf to confirm per-sandbox accounting — the F-149 regression.

## Scope summary: per-process vs per-sandbox vs uid-wide

Every cap except the uid-wide backstop is per-process or per-sandbox. The `RLIMIT_NPROC` backstop is per real-uid and exists solely for hosts where the cgroup path is unavailable:

| Cap | Kernel scope | Per-sandbox? |
|---|---|---|
| `RLIMIT_CPU` | per-process | yes |
| `RLIMIT_AS` | per-process | yes |
| `RLIMIT_NOFILE` | per-process | yes |
| `RLIMIT_FSIZE` | per-process | yes |
| cgroup v2 `pids.max` | **per-cgroup** | **yes** |
| `RLIMIT_NPROC` (backstop) | **per real-uid** | **no — degraded-host only** |

`RLIMIT_NPROC` is checked at fork time against the count of processes already owned by the calling task's real uid, not against a per-sandbox counter. On hosts that delegate the cgroup v2 `pids` controller to the daemon's slice (the default on systemd with user sessions), the authoritative per-sandbox budget is `pids.max` written into a fresh leaf created at spawn. When the host does not delegate (cgroup v1, containers without delegation, non-Linux), cgroup setup is skipped silently and `RLIMIT_NPROC` becomes the only ceiling.

### Tuning `max_processes` (per-sandbox cgroup limit)

`SandboxConfig::max_processes` writes to `pids.max` on the per-sandbox cgroup v2 leaf — the number applies **per sandbox**, not per uid. Two sandboxes each get their own independent budget of N tasks; a misbehaving tool cannot starve a well-behaved sibling.

| Host class | Suggested `max_processes` | Rationale |
|---|---|---|
| Desktop / shared workstation | 256 (default) | room for a `make -j8` or similar fan-out without giving a compromised tool a large budget |
| Dedicated CI runner | 256–512 | CI runners typically have more headroom than desktops; raise only if legitimate build fan-out hits the cap |
| Tight single-tool workloads | 64 | sufficient for single-command tool invocations; shrinks blast radius aggressively |
| Container with one user per container | leave at default | container PID namespaces already provide per-container isolation; the cgroup cap is defense-in-depth |

### Tuning `rlimit_nproc_backstop` (uid-wide fallback)

`SandboxConfig::rlimit_nproc_backstop` writes to `RLIMIT_NPROC` in `pre_exec`. The kernel checks this at fork time against the uid-wide process count, so the cap is shared across every process the daemon's uid owns. It exists for hosts where the cgroup v2 `pids` controller is not delegated to the daemon's slice (cgroup v1, containers without delegation, non-systemd inits) — on those hosts it becomes the only process ceiling.

| Host class | Suggested `rlimit_nproc_backstop` | Rationale |
|---|---|---|
| Desktop / shared workstation | 4096 (default) | enough headroom for the uid baseline (800–4000 processes on a busy session) while still bounding a runaway fork loop |
| Dedicated CI runner | 4096 (default) | CI-runner uids are small; the default is permissive enough |
| Tight-budget host | 1024 | only if the cgroup path is guaranteed available; otherwise degraded hosts can starve |

Because the backstop is uid-wide, it is **not** a substitute for `max_processes` on hosts that lack cgroup delegation — it is strictly a lower bound. Prefer keeping the cgroup path available.

## Cgroup-based per-sandbox PID limit (implemented in F-149)

Per-sandbox process accounting runs on the cgroup v2 `pids` controller rather than on `RLIMIT_NPROC`. The shape:

1. At sandbox spawn, resolve the daemon's own cgroup via `/proc/self/cgroup` (expects a `0::<path>` v2 entry) and create a fresh leaf as a **sibling** of the daemon's cgroup. Sibling-of-daemon placement is mandatory: cgroup v2 forbids a cgroup from containing both processes and child cgroups, so the daemon's own cgroup cannot be a parent.
2. Write `SandboxConfig::max_processes` to `pids.max` in the fresh leaf.
3. Install a `pre_exec` hook that, inside the forked child and before `execve`, writes `getpid()` into the leaf's `cgroup.procs`. Doing the enrollment post-fork / pre-exec closes the fork-escape race: a parent-side write can only happen after the kernel has scheduled the child, which leaves a window in which the child may already have forked descendants that never enter the leaf.
4. On sandbox teardown, write `"1"` to `cgroup.kill` (cgroup v2 ≥ 5.14) to SIGKILL every task still in the leaf, then `rmdir` the leaf.

The implementation lives in `crates/forge-session/src/sandbox.rs`, alongside the `CgroupLeaf` helper and the `cgroup_pids_max_caps_sandbox_tasks_per_f149` regression test. The regression test polls `pids.current` directly from the leaf from Rust and takes a running max rather than counting forked PIDs in shell — the original F-078 attempt hung because the shell retry on EAGAIN saturated the cap in a tight loop and the test harness never reaped. Rust-side kernel probes have no such behavior. The test avoids the post-6.1 `pids.peak` file for compatibility with Ubuntu 22.04 LTS CI runners.

The test gates on `/sys/fs/cgroup/cgroup.controllers` containing `pids` and on `SandboxedChild::cgroup_path` returning `Some(_)` — hosts without cgroup v2 delegation skip with a clear message rather than silently exercising the rlimit-only fallback.

### Orphan reaping

Sandbox teardown follows two paths that differ in who owns the child afterwards:

- **Drop on `SandboxedChild`** — the common case. Drop sends `SIGKILL` to the process group via `killpg`, then writes `cgroup.kill` and `rmdir`s the leaf immediately. No orphan possible on this path.
- **`SandboxedChild::into_child`** — caller takes ownership of the underlying `tokio::process::Child` and will `wait().await` for natural exit. Killing the leaf here would SIGKILL the very child the caller is about to wait on, so Drop's aggressive teardown is not usable. Instead, `into_child` schedules a background tokio task that polls the leaf's `cgroup.events` for `populated 0` at 50 ms intervals and `rmdir`s the leaf once the kernel reports it empty. The reaper caps its total wait at ~10 minutes so a stuck child does not pin the task indefinitely.
- **Process crash / runtime shutdown** — if forge-session exits before the reaper runs, or before Drop fires, the leaf is left empty-but-present. Systemd reclaims empty leaves on host reboot, so this is self-healing; an on-startup sweep of `forge-sandbox-*` leaves whose owning pid is gone is feasible follow-up work but not currently implemented because the leak is bounded and visible (under `/sys/fs/cgroup/.../app.slice/`) to operators who need to audit.

All three branches are best-effort: the user-visible promise is that the per-sandbox `pids.max` is enforced while the sandbox is alive, not that every leaf is rmdir'd synchronously when the sandbox exits.
