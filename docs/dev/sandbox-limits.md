# Sandbox resource limits

Operator reference for the five `setrlimit(2)` caps that `crates/forge-session/src/sandbox.rs::SandboxedCommand` applies to every approved tool invocation, and for the one cap whose kernel scope differs from the rest. For the security context that motivates these caps, see [`security.md`](security.md#sandbox-resource-limits-f-055); this page is the full operator-facing reference the security page links out to.

## The five caps at a glance

Defaults come from `SandboxConfig::default`. Soft and hard limits are set to the same value, so a sandboxed child cannot raise them.

| Resource | Default | Threat mitigated | Override field |
|---|---|---|---|
| `RLIMIT_CPU` | 30 s | runaway CPU on a single tool call (SIGXCPU) | `cpu_seconds` |
| `RLIMIT_AS` | 512 MiB | address-space exhaustion / large allocations | `address_space_bytes` |
| `RLIMIT_NPROC` | 4096 | fork bombs (see caveat below) | `max_processes` |
| `RLIMIT_NOFILE` | 256 | fd-table exhaustion | `max_open_files` |
| `RLIMIT_FSIZE` | 100 MiB | cat-to-disk attacks (SIGXFSZ on overflow) | `max_file_size_bytes` |

The `rlimits_bound_child_via_setrlimit` test in `crates/forge-session/src/sandbox.rs` probes `/proc/self/limits` from inside the sandbox to confirm `pre_exec` actually applied these values — that test is the load-bearing regression for F-055.

## Scope asymmetry: NPROC is uid-wide

Four of the five caps are per-process; `RLIMIT_NPROC` is per real-uid. That asymmetry is load-bearing for how operators should tune them:

| Cap | Kernel scope | Per-sandbox? |
|---|---|---|
| `RLIMIT_CPU` | per-process | yes |
| `RLIMIT_AS` | per-process | yes |
| `RLIMIT_NOFILE` | per-process | yes |
| `RLIMIT_FSIZE` | per-process | yes |
| `RLIMIT_NPROC` | **per real-uid** | **no — see below** |

`RLIMIT_NPROC` is checked at `fork(2)` time against the count of processes already owned by the calling task's real uid, not against a per-sandbox counter. Because `forged` runs as the operator's normal uid (it shares the desktop session, or in CI it shares the test harness's uid), every other process the same uid owns counts against the 4096 default. The 4096 ceiling is therefore tuned for one job: stopping a runaway `fork()` loop within milliseconds — which saturates any cap regardless of headroom — not for budgeting a sandbox's legitimate process count.

### Operator scenarios

Two extremes make the asymmetry concrete:

- **Desktop session.** A user with a browser, IDE, terminal multiplexer, and a few language-server processes typically already owns 800–4000 uid-wide processes. A sandboxed tool inherits whatever headroom is left — perhaps 96 processes on a busy session — before `fork(2)` returns `EAGAIN`. Tools that legitimately fan out (a `make -j16` build, a `cargo test` matrix) can hit the cap without any abuse.
- **Bare CI host.** A CI runner with one job and a handful of system services owns ~50 uid-wide processes. The same sandboxed tool gets ~4046 forks before tripping the cap. A malicious or buggy tool on this host has effectively the entire 4096 budget to itself.

The same default cap therefore behaves as "tight headroom" on a desktop and "permissive ceiling" on CI. Neither end is wrong — both still stop a fork bomb — but operators tuning for legitimate fan-out should treat `SandboxConfig::max_processes` as a **whole-uid** number.

### Tuning `max_processes`

| Host class | Suggested `max_processes` | Rationale |
|---|---|---|
| Desktop / shared workstation | 4096 (default) — raise to 8192 only if `fork: Resource temporarily unavailable` shows up under normal use | uid baseline is variable; the cap exists to bound a runaway, not to budget the tool |
| Dedicated CI runner | 1024 or lower | the uid baseline is small and predictable; tightening the cap reduces the blast radius of a compromised tool without affecting realistic build fan-out |
| Container with one user per container | leave at default | container PID namespaces already provide per-container isolation; the rlimit is a defense-in-depth backstop |

Because the cap is uid-wide, two sandboxes started on the same daemon **do not** each get an independent budget — they share whatever the cap permits at the moment each `fork(2)` runs. A misbehaving tool can therefore starve a well-behaved sibling by consuming the uid's headroom first. This is the core asymmetry that the four per-process limits do not have, and it is the reason `RLIMIT_NPROC` cannot be used as a per-sandbox process budget no matter how the default is tuned.

## Cgroup-based per-sandbox PID limit (follow-up)

The fix for the scope asymmetry is to move per-sandbox process counting off the rlimit machinery and onto the cgroup v2 PID controller. The shape:

1. At sandbox spawn, create a fresh cgroup v2 leaf under the daemon's delegated cgroup (e.g. `/sys/fs/cgroup/forge.slice/sandbox-<uuid>/`). On systemd hosts this works cleanly via `systemd-run --user --scope --property=TasksMax=N`.
2. Write the desired ceiling to `pids.max` (the controller's per-cgroup task ceiling).
3. Move the freshly-spawned sandbox PID into the leaf's `cgroup.procs` before `execve(2)` — practically, this means writing the child PID from the parent right after `fork(2)` returns, gated on the cgroup mount and write succeeding.
4. On sandbox shutdown, kill any survivors via `cgroup.kill` (cgroup v2 ≥ 5.14) and then remove the leaf.

`pids.max` is checked at `fork(2)`/`clone(2)` against the cgroup's current task count only, so each sandbox gets its own independent budget regardless of what the daemon's uid is doing elsewhere. The existing `RLIMIT_NPROC` setrlimit stays as a uid-wide backstop; the cgroup controller becomes the per-sandbox enforcement primitive. This is the pattern Linux container runtimes already use for the same reason.

The integration and its regression test are tracked on [F-149](https://github.com/forge-ide/forge/issues/274).
