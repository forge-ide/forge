//! Per-agent-instance resource sampler (F-152 Linux, F-156 macOS/Windows).
//!
//! Populates the AgentMonitor inspector's cpu / rss / fds pills (F-140).
//! F-140 shipped the pill chrome with placeholder dashes because there was
//! no backend monitor yet; this module is that backend.
//!
//! # Architecture
//!
//! - [`Sampler`] is the trait every platform probe implements. Tests use
//!   the [`FakeSampler`] below; Linux is backed by `/proc/<pid>`, macOS
//!   by `libproc`, Windows by the `GetProcessTimes` /
//!   `GetProcessMemoryInfo` / `GetProcessHandleCount` trio. Targets
//!   outside those three fail to compile at the module-level
//!   `compile_error!` — the F-152 silent `Sample::default()` stub was
//!   removed in F-156 because all-zero pills masquerading as real
//!   readings are worse than a build error.
//!
//! - [`ResourceMonitor::track`] registers a `(instance_id, pid)` pair and
//!   spawns a per-instance tokio tick task. On each tick the task asks the
//!   sampler for a fresh [`Sample`], folds it into a small rolling-average
//!   CPU history, and emits an [`Event::ResourceSample`] on the monitor's
//!   broadcast channel.
//!
//! - [`ResourceMonitor::untrack`] aborts the matching task; the event
//!   stream naturally stops for that id so the UI pills clear back to `—`.
//!   Dropping the monitor aborts every tracked task.
//!
//! # Why the instance-id is the caller's input
//!
//! `forge_agents::AgentInstance` does not carry a PID today — instances
//! are logical entities in the orchestrator registry. Rather than adding
//! `pid: Option<u32>` to the registry (which would couple logical
//! lifecycle to OS-level identity), this module takes both pieces
//! externally. The spawn site that has the PID (e.g. a future step
//! executor that forks a provider sidecar) calls `track`; the
//! orchestrator's terminal event handler calls `untrack`.
//!
//! The DoD's "no sampler thread leaks on instance drop" invariant is
//! expressed at the `ResourceMonitor::drop` boundary — dropping the
//! monitor aborts every task, so scoping the monitor to the session
//! lifetime is sufficient to guarantee no leaks when the session ends.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use forge_core::{AgentInstanceId, Event};
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;

/// Default tick cadence. 1 Hz — the low end of the DoD's 1–5 Hz range.
/// Low enough to keep `/proc` read amplification negligible on a big
/// session, fast enough that a human sees the pill value change.
pub const DEFAULT_TICK: Duration = Duration::from_secs(1);

/// Capacity of the monitor's broadcast bus. Matches
/// `forge_session::bg_agents::EVENT_BUS_CAPACITY` so a slow subscriber
/// on the merged `session:event` stream doesn't drop resource samples
/// before every other variant.
const EVENT_BUS_CAPACITY: usize = 1024;

/// Number of most-recent CPU samples to average when emitting the pill
/// value. Small — the pill is a "what is this agent doing right now?"
/// glance, not a long-horizon chart. A larger window would smooth
/// meaningful spikes away.
const CPU_WINDOW: usize = 5;

/// Raw platform sample returned by a [`Sampler`].
///
/// Each field is independently `Option` because best-effort platform
/// probes can fail on a single dimension while the rest succeed. The
/// wire contract (`Event::ResourceSample`) preserves the `None`s
/// verbatim so the UI never sees a value ghost between emissions.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sample {
    /// CPU-time the process has used since the previous sample, in
    /// **seconds of on-cpu time**. A delta, not a total — the monitor
    /// divides by the tick window to compute `cpu_pct`. `None` when the
    /// probe cannot read a fresh cumulative total.
    pub cpu_seconds: Option<f64>,
    /// Resident set size in bytes. `None` when unreadable.
    pub rss_bytes: Option<u64>,
    /// Live file-descriptor count. `None` when unreadable.
    pub fd_count: Option<u64>,
}

/// Platform-agnostic resource probe.
///
/// Implementations hold per-instance state across calls (the delta path
/// for CPU needs the previous cumulative total). The trait is `async`
/// so a real Linux probe can `tokio::fs::read` `/proc/<pid>/stat` without
/// blocking the sampler task.
#[async_trait::async_trait]
pub trait Sampler: Send + Sync + 'static {
    /// Return a fresh [`Sample`] for the given PID. Called on every tick
    /// while the instance is tracked. Implementations must return a
    /// default `Sample` (all `None`) rather than error — a transient
    /// probe failure surfaces as `—` in the UI, not as a hard stop.
    async fn sample(&self, pid: u32) -> Sample;
}

/// Owns a broadcast channel and a set of per-instance tick tasks.
pub struct ResourceMonitor {
    events: broadcast::Sender<Event>,
    sampler: Arc<dyn Sampler>,
    tick: Duration,
    tasks: Arc<Mutex<HashMap<AgentInstanceId, JoinHandle<()>>>>,
}

impl ResourceMonitor {
    /// Construct a monitor bound to `sampler` that ticks every `tick`.
    pub fn new(sampler: Arc<dyn Sampler>, tick: Duration) -> Self {
        let (events, _rx) = broadcast::channel(EVENT_BUS_CAPACITY);
        Self {
            events,
            sampler,
            tick,
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Convenience constructor with the default 1 Hz tick.
    pub fn with_default_tick(sampler: Arc<dyn Sampler>) -> Self {
        Self::new(sampler, DEFAULT_TICK)
    }

    /// Subscribe to the resource-sample event stream. Each subscriber
    /// gets its own receiver; late subscribers miss earlier events
    /// (bounded broadcast).
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Start sampling the process at `pid` for the agent instance
    /// identified by `id`. Idempotent on `id` — a repeated `track` with
    /// the same id replaces the previous task (used when a restarted
    /// instance reuses its id).
    ///
    /// # Daemon-PID guard (F-370)
    ///
    /// If `pid` equals the daemon's own PID this call is a deliberate
    /// no-op: it registers no task, emits no `ResourceSample`, and leaves
    /// the id **un-tracked**. The `Event::ResourceSample` wire shape is
    /// per-instance, but the daemon-wide numbers would be shifted by
    /// unrelated cross-agent work — rendered in the AgentMonitor pills
    /// as if they were per-instance, the values mislead the user. Until
    /// a real per-child PID is available (e.g. a future step executor
    /// that forks a provider sidecar), skipping emission lets the UI
    /// fall back to the `—` placeholder the pre-F-152 stub already
    /// rendered. Callers with a real child PID get full sampling.
    pub async fn track(&self, id: AgentInstanceId, pid: u32) {
        if pid == std::process::id() {
            // Drop any stale task already registered under this id so a
            // previous real-PID track doesn't keep sampling after a
            // "downgrade" to the daemon PID.
            let mut tasks = self.tasks.lock().await;
            if let Some(prev) = tasks.remove(&id) {
                prev.abort();
            }
            return;
        }
        let mut tasks = self.tasks.lock().await;
        if let Some(prev) = tasks.remove(&id) {
            prev.abort();
        }
        let sampler = Arc::clone(&self.sampler);
        let tick = self.tick;
        let events = self.events.clone();
        let task_id = id.clone();
        let handle = tokio::spawn(async move {
            run_ticker(task_id, pid, sampler, tick, events).await;
        });
        tasks.insert(id, handle);
    }

    /// Stop sampling `id`. Idempotent — unknown ids are a no-op.
    pub async fn untrack(&self, id: &AgentInstanceId) {
        let mut tasks = self.tasks.lock().await;
        if let Some(handle) = tasks.remove(id) {
            handle.abort();
        }
    }

    /// Number of currently-tracked instances. Exposed for tests that
    /// want to pin the no-leak invariant without reaching into the
    /// `tasks` map.
    pub async fn tracked_count(&self) -> usize {
        self.tasks.lock().await.len()
    }
}

impl Drop for ResourceMonitor {
    /// Aborts every outstanding tick task. This is the mechanical
    /// implementation of the DoD's "no sampler thread leaks on instance
    /// drop" invariant — callers scope the monitor to the session and
    /// every task dies with the session.
    fn drop(&mut self) {
        // `try_lock` so a panicking subscriber somewhere down the stack
        // that still holds the mutex doesn't turn drop-time abort into a
        // deadlock. On contention we fall back to draining what we can
        // see — the tokio runtime cancels the rest when the session's
        // runtime shuts down.
        if let Ok(mut tasks) = self.tasks.try_lock() {
            for (_id, handle) in tasks.drain() {
                handle.abort();
            }
        }
    }
}

async fn run_ticker(
    id: AgentInstanceId,
    pid: u32,
    sampler: Arc<dyn Sampler>,
    tick: Duration,
    events: broadcast::Sender<Event>,
) {
    let mut interval = tokio::time::interval(tick);
    // Skip the zero-duration first tick that `interval` fires
    // immediately on creation — the first sample has no delta to
    // difference against, so emitting a pill value with `cpu_pct: None`
    // at t=0 would flicker the UI. Waiting one tick guarantees the
    // first emission carries a meaningful `cpu_pct`.
    interval.tick().await;
    let mut history: VecDeque<f64> = VecDeque::with_capacity(CPU_WINDOW);
    let tick_secs = tick.as_secs_f64().max(f64::EPSILON);
    loop {
        interval.tick().await;
        let sample = sampler.sample(pid).await;
        if let Some(cpu_seconds) = sample.cpu_seconds {
            let pct = (cpu_seconds / tick_secs) * 100.0;
            if history.len() == CPU_WINDOW {
                history.pop_front();
            }
            history.push_back(pct);
        }
        let cpu_pct = if history.is_empty() {
            None
        } else {
            Some(history.iter().sum::<f64>() / history.len() as f64)
        };
        let _ = events.send(Event::ResourceSample {
            instance_id: id.clone(),
            cpu_pct,
            rss_bytes: sample.rss_bytes,
            fd_count: sample.fd_count,
            sampled_at: Utc::now(),
        });
    }
}

// ---------------------------------------------------------------------------
// Platform probes
// ---------------------------------------------------------------------------

/// Linux `/proc/<pid>` probe. Parses `stat` for cumulative CPU ticks,
/// `status` for RSS (kB), and counts entries in `fd/`.
///
/// Per-instance delta state is kept in an async mutex keyed by PID so
/// repeated `track`s for the same PID share a CPU baseline.
#[cfg(target_os = "linux")]
pub mod linux {
    use super::{Sample, Sampler};
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    /// Clock ticks per second read from `sysconf(_SC_CLK_TCK)` once at
    /// probe construction. 100 on every Linux distro I've seen, but
    /// reading it keeps the code portable across architectures that
    /// might pick 1000.
    fn clock_ticks_per_sec() -> u64 {
        // SAFETY: sysconf(3) is a thread-safe pure function taking no
        // Rust references. `_SC_CLK_TCK` is always defined on Linux.
        let raw = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if raw <= 0 {
            100
        } else {
            raw as u64
        }
    }

    /// `/proc/<pid>`-backed sampler.
    pub struct ProcSampler {
        clk_tck: u64,
        previous: Mutex<HashMap<u32, u64>>, // pid -> utime+stime ticks
    }

    impl ProcSampler {
        pub fn new() -> Self {
            Self {
                clk_tck: clock_ticks_per_sec(),
                previous: Mutex::new(HashMap::new()),
            }
        }
    }

    impl Default for ProcSampler {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait::async_trait]
    impl Sampler for ProcSampler {
        async fn sample(&self, pid: u32) -> Sample {
            let cpu_seconds = cpu_delta_seconds(self, pid).await;
            let rss_bytes = read_rss_bytes(pid).await;
            let fd_count = count_fds(pid).await;
            Sample {
                cpu_seconds,
                rss_bytes,
                fd_count,
            }
        }
    }

    async fn cpu_delta_seconds(s: &ProcSampler, pid: u32) -> Option<f64> {
        let text = tokio::fs::read_to_string(format!("/proc/{pid}/stat"))
            .await
            .ok()?;
        let total_ticks = parse_stat_utime_stime(&text)?;
        let mut prev = s.previous.lock().await;
        let delta_ticks = match prev.get(&pid).copied() {
            Some(previous) if total_ticks >= previous => total_ticks - previous,
            _ => 0,
        };
        prev.insert(pid, total_ticks);
        if s.clk_tck == 0 {
            return None;
        }
        Some(delta_ticks as f64 / s.clk_tck as f64)
    }

    /// Parse columns 14 (`utime`) and 15 (`stime`) out of a
    /// `/proc/<pid>/stat` line. The comm field may contain spaces and
    /// parentheses, so we split on the last `)` to get the post-comm
    /// tail and index from there.
    pub(crate) fn parse_stat_utime_stime(text: &str) -> Option<u64> {
        let tail_start = text.rfind(')')?;
        let tail = &text[tail_start + 1..];
        let fields: Vec<&str> = tail.split_whitespace().collect();
        // After `)` the first field is `state` (index 0 = column 3);
        // utime is column 14 = fields index 11; stime column 15 = 12.
        let utime: u64 = fields.get(11)?.parse().ok()?;
        let stime: u64 = fields.get(12)?.parse().ok()?;
        Some(utime + stime)
    }

    async fn read_rss_bytes(pid: u32) -> Option<u64> {
        let text = tokio::fs::read_to_string(format!("/proc/{pid}/status"))
            .await
            .ok()?;
        parse_status_rss_bytes(&text)
    }

    /// Parse `VmRSS:\s+N kB` out of `/proc/<pid>/status`. Returns bytes.
    pub(crate) fn parse_status_rss_bytes(text: &str) -> Option<u64> {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let rest = rest.trim();
                let kb_str = rest.split_whitespace().next()?;
                let kb: u64 = kb_str.parse().ok()?;
                return Some(kb * 1024);
            }
        }
        None
    }

    async fn count_fds(pid: u32) -> Option<u64> {
        let mut dir = tokio::fs::read_dir(format!("/proc/{pid}/fd")).await.ok()?;
        let mut n: u64 = 0;
        while let Ok(Some(_)) = dir.next_entry().await {
            n += 1;
        }
        Some(n)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parse_status_rss_extracts_kilobytes_and_converts_to_bytes() {
            // Fixture mirrors a real `/proc/<pid>/status` fragment.
            let text = "\
Name:   forged
VmPeak:   123456 kB
VmSize:   123456 kB
VmRSS:      4096 kB
Threads:   3
";
            assert_eq!(parse_status_rss_bytes(text), Some(4096 * 1024));
        }

        #[test]
        fn parse_status_rss_returns_none_when_missing() {
            assert_eq!(parse_status_rss_bytes("Name: forged\n"), None);
        }

        #[test]
        fn parse_stat_finds_utime_stime_with_spaces_in_comm() {
            // Real fixture shape — comm (`my proc (dev)`) contains spaces
            // and an inner parenthesis. Matches the parser's "last `)` wins"
            // rule that defends against this case.
            let text = "12345 (my proc (dev)) S 1 12345 12345 0 -1 4194304 0 0 0 0 \
                        42 17 0 0 20 0 1 0 100 0 0 rest ignored";
            // 42 + 17 = 59
            assert_eq!(parse_stat_utime_stime(text), Some(59));
        }

        #[tokio::test]
        async fn proc_sampler_returns_a_non_none_rss_for_self() {
            // The test's own PID is guaranteed to have a live /proc
            // entry, so RSS must parse to Some(_). CPU is a delta, so
            // the first call is expected to report 0.0s (no baseline).
            // This is the live end-to-end hook — if /proc isn't mounted
            // the test harness itself would have failed earlier.
            let s = ProcSampler::new();
            let sample = s.sample(std::process::id()).await;
            assert!(
                sample.rss_bytes.is_some(),
                "live /proc/self/status must parse"
            );
            assert!(
                sample.fd_count.is_some(),
                "live /proc/self/fd must be readable"
            );
        }
    }
}

/// macOS `libproc`-backed probe. Reads `proc_taskinfo` for cumulative
/// user+system CPU nanoseconds and resident memory, and counts open file
/// descriptors via `listpidinfo::<ListFDs>`.
///
/// `proc_taskinfo`'s `pti_total_user` / `pti_total_system` are in
/// nanoseconds (see `<sys/proc_info.h>` in the macOS SDK). The Linux
/// probe returns seconds, so we convert. `pti_resident_size` is already
/// in bytes. `pbi_nfiles` on the BSD-info flavor gives the fd count
/// without enumerating them — far cheaper than listing — but the listing
/// path is the one the DoD names and it's the path that matches how the
/// Linux probe physically counts entries in `/proc/<pid>/fd`, so we use
/// it for behavioral parity.
#[cfg(target_os = "macos")]
pub mod macos {
    use super::{Sample, Sampler};
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    use libproc::bsd_info::BSDInfo;
    use libproc::file_info::{ListFDs, ProcFDType};
    use libproc::proc_pid::{listpidinfo, pidinfo};
    use libproc::task_info::TaskInfo;

    /// `libproc`-backed sampler. Holds a per-PID CPU-time baseline in
    /// nanoseconds so the public `Sample::cpu_seconds` can report a
    /// delta rather than a cumulative total — matching the Linux probe.
    pub struct LibprocSampler {
        previous_ns: Mutex<HashMap<u32, u64>>, // pid -> utime+stime ns
    }

    impl LibprocSampler {
        pub fn new() -> Self {
            Self {
                previous_ns: Mutex::new(HashMap::new()),
            }
        }
    }

    impl Default for LibprocSampler {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait::async_trait]
    impl Sampler for LibprocSampler {
        async fn sample(&self, pid: u32) -> Sample {
            // `libproc` is synchronous FFI. Calls resolve in microseconds
            // per the XNU syscall tables, so staying on the tokio worker
            // is preferable to the allocation cost of `spawn_blocking`.
            let (cpu_seconds, rss_bytes) = match pidinfo::<TaskInfo>(pid as i32, 0) {
                Ok(ti) => {
                    let total_ns = ti.pti_total_user.saturating_add(ti.pti_total_system);
                    let mut prev = self.previous_ns.lock().await;
                    let delta_ns = match prev.get(&pid).copied() {
                        Some(previous) if total_ns >= previous => total_ns - previous,
                        _ => 0,
                    };
                    prev.insert(pid, total_ns);
                    let seconds = delta_ns as f64 / 1_000_000_000.0;
                    (Some(seconds), Some(ti.pti_resident_size))
                }
                Err(_) => (None, None),
            };
            let fd_count = count_fds(pid);
            Sample {
                cpu_seconds,
                rss_bytes,
                fd_count,
            }
        }
    }

    /// Counts open file descriptors by asking the kernel to enumerate
    /// them. `listpidinfo::<ListFDs>` takes a max-entries bound; BSDInfo
    /// carries `pbi_nfiles` which is the kernel's live count. Use that
    /// as the bound, then count the returned entries — if the kernel
    /// filled the buffer we still get an accurate count.
    fn count_fds(pid: u32) -> Option<u64> {
        let bsd = pidinfo::<BSDInfo>(pid as i32, 0).ok()?;
        // `pbi_nfiles` is a u32; cast up front so overflow-on-pathological
        // process is a saturating cast rather than a wrap.
        let cap = bsd.pbi_nfiles as usize;
        // A zero cap means the kernel says "no open fds" — return it
        // straight rather than asking `listpidinfo` for a zero-size Vec.
        if cap == 0 {
            return Some(0);
        }
        let fds = listpidinfo::<ListFDs>(pid as i32, cap).ok()?;
        // Filter to the set `pbi_nfiles` really counts: regular
        // vnode/socket/pipe fds, matching what `/proc/<pid>/fd` on Linux
        // shows. Kernel-internal entries like fsevents aren't visible on
        // Linux either.
        let n = fds
            .iter()
            .filter(|fd| {
                matches!(
                    fd.proc_fdtype.into(),
                    ProcFDType::VNode
                        | ProcFDType::Socket
                        | ProcFDType::Pipe
                        | ProcFDType::PSEM
                        | ProcFDType::PSHM
                        | ProcFDType::KQueue
                )
            })
            .count() as u64;
        Some(n)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn libproc_sampler_returns_non_none_rss_and_fds_for_self() {
            // Mirrors the Linux sibling test
            // (`proc_sampler_returns_a_non_none_rss_for_self`). The DoD's
            // per-platform invariant is "monitor emits non-zero samples
            // for a self-process"; probing the real libproc sampler
            // against the test's own PID is the strongest live check we
            // can run inside `cargo test`. `cpu_seconds` is a delta, so
            // the first call reports 0.0 — hence only RSS and fd_count
            // carry the "non-None" assertion, matching Linux.
            //
            // This test ships live on macOS CI (F-158 added the
            // macos-latest runner that calls `just test-rust`) so a
            // regression on the real syscall path fails the PR.
            let s = LibprocSampler::new();
            let sample = s.sample(std::process::id()).await;
            assert!(
                sample.rss_bytes.is_some(),
                "libproc must parse live self RSS"
            );
            assert!(
                sample.fd_count.is_some(),
                "libproc must count live self fds"
            );
            let rss = sample.rss_bytes.unwrap();
            assert!(rss > 0, "test process must report non-zero RSS, got {rss}");
            let fds = sample.fd_count.unwrap();
            assert!(
                fds > 0,
                "test process must have at least one open fd, got {fds}"
            );
        }
    }
}

/// Windows `kernel32` / `psapi`-backed probe. Reads cumulative kernel +
/// user CPU time via `GetProcessTimes` (returned as 100ns FILETIME
/// units), working-set bytes via `GetProcessMemoryInfo`, and open kernel
/// handle count via `GetProcessHandleCount`.
///
/// Windows "handles" aren't a perfect analogue of POSIX file descriptors
/// — they include window handles, event handles, mutex handles, etc. —
/// but they're the closest off-the-shelf count and the DoD explicitly
/// names `GetProcessHandleCount` as the probe, so we follow it.
#[cfg(target_os = "windows")]
pub mod windows {
    use super::{Sample, Sampler};
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessHandleCount, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_VM_READ,
    };

    /// `windows-sys`-backed sampler. Same delta-from-previous CPU pattern
    /// as the Linux and macOS probes.
    pub struct WindowsSampler {
        /// pid -> cumulative (kernel + user) CPU in 100ns FILETIME units.
        previous_100ns: Mutex<HashMap<u32, u64>>,
    }

    impl WindowsSampler {
        pub fn new() -> Self {
            Self {
                previous_100ns: Mutex::new(HashMap::new()),
            }
        }
    }

    impl Default for WindowsSampler {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Merge two 32-bit halves of a FILETIME into a single u64. Windows
    /// lays FILETIME out as a struct of `dwLowDateTime` + `dwHighDateTime`
    /// instead of a u64 for historical 16-bit-alignment reasons.
    fn filetime_to_u64(ft: FILETIME) -> u64 {
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    }

    /// RAII wrapper around a `HANDLE` so the sampler can't leak handles
    /// through a `?` / early return. `OpenProcess` returns a handle that
    /// must be `CloseHandle`'d; forgetting it is the classic long-lived
    /// Windows service leak.
    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: self.0 came from OpenProcess which returns
                // either a valid kernel handle or null. The null branch
                // is skipped above.
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    #[async_trait::async_trait]
    impl Sampler for WindowsSampler {
        async fn sample(&self, pid: u32) -> Sample {
            // Ask for the minimum access rights each probe needs.
            // PROCESS_QUERY_LIMITED_INFORMATION covers GetProcessTimes and
            // GetProcessHandleCount; PROCESS_VM_READ is required by the
            // psapi memory probe. An AV/EDR can still deny this — callers
            // degrade gracefully (all-None Sample) in that case.
            //
            // `HANDLE` is `*mut c_void` and therefore `!Send`. To keep the
            // `sample()` future `Send` (the `Sampler` trait requires it
            // through `async_trait`'s `+ Send` bound), we do every syscall
            // in a tight non-await scope, drop the handle at the end of
            // it, and only then `.await` the mutex that stores previous
            // CPU totals. Nothing in the sync block suspends, so there's
            // no behavioral downside.
            let (total_100ns_opt, rss_bytes, fd_count) = {
                let handle = unsafe {
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid)
                };
                if handle.is_null() {
                    return Sample::default();
                }
                let handle = OwnedHandle(handle);
                (
                    read_cpu_total_100ns(handle.0),
                    read_working_set_bytes(handle.0),
                    read_handle_count(handle.0),
                )
            };

            let cpu_seconds = match total_100ns_opt {
                Some(total_100ns) => {
                    let mut prev = self.previous_100ns.lock().await;
                    let delta = match prev.get(&pid).copied() {
                        Some(previous) if total_100ns >= previous => total_100ns - previous,
                        _ => 0,
                    };
                    prev.insert(pid, total_100ns);
                    // FILETIME ticks are 100ns. 10_000_000 per second.
                    Some(delta as f64 / 10_000_000.0)
                }
                None => None,
            };

            Sample {
                cpu_seconds,
                rss_bytes,
                fd_count,
            }
        }
    }

    fn read_cpu_total_100ns(h: HANDLE) -> Option<u64> {
        let mut creation = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut kernel = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut user = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        // SAFETY: `h` is a live process handle from OpenProcess; all four
        // output pointers target stack-allocated FILETIME locals that
        // outlive the call.
        let ok =
            unsafe { GetProcessTimes(h, &mut creation, &mut exit, &mut kernel, &mut user) != 0 };
        if !ok {
            return None;
        }
        Some(filetime_to_u64(kernel).saturating_add(filetime_to_u64(user)))
    }

    fn read_working_set_bytes(h: HANDLE) -> Option<u64> {
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        // SAFETY: `h` is a live process handle. `counters` is a stack
        // struct sized correctly via the `cb` field set above.
        let ok = unsafe { GetProcessMemoryInfo(h, &mut counters, counters.cb) != 0 };
        if !ok {
            return None;
        }
        Some(counters.WorkingSetSize as u64)
    }

    fn read_handle_count(h: HANDLE) -> Option<u64> {
        let mut count: u32 = 0;
        // SAFETY: `h` is a live process handle; `count` is a live stack
        // local the kernel writes through the out-pointer.
        let ok = unsafe { GetProcessHandleCount(h, &mut count) != 0 };
        if !ok {
            return None;
        }
        Some(count as u64)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // CI gap: there is no Windows GitHub Actions runner wired up to
        // forge-ide/forge today (F-158 added macOS-latest but Windows is
        // out of scope for that issue). This test compiles in-tree on
        // Windows hosts and is the self-process equivalent of the Linux
        // and macOS siblings; it must be run manually on a Windows dev
        // box to validate. The `#[cfg(target_os = "windows")]` on the
        // enclosing module keeps the build green on Linux/macOS CI by
        // excluding the whole tree from compilation there.
        #[tokio::test]
        async fn windows_sampler_returns_non_none_rss_and_fds_for_self() {
            let s = WindowsSampler::new();
            let sample = s.sample(std::process::id()).await;
            assert!(
                sample.rss_bytes.is_some(),
                "GetProcessMemoryInfo must succeed for self"
            );
            assert!(
                sample.fd_count.is_some(),
                "GetProcessHandleCount must succeed for self"
            );
            let rss = sample.rss_bytes.unwrap();
            assert!(rss > 0, "test process must report non-zero RSS, got {rss}");
            let fds = sample.fd_count.unwrap();
            assert!(
                fds > 0,
                "test process must have at least one handle, got {fds}"
            );
        }
    }
}

// Any target that isn't one of the three we support fails to compile
// with a loud error. The F-152 stub's Sample::default() silently produced
// all-zero pill values on unsupported platforms, which is worse than a
// hard stop — consumers would think the monitor was working.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!(
    "forge-session resource monitor is not implemented on this target. \
     Supported platforms: linux, macos, windows."
);

/// Return the compile-time "best available" sampler for the current
/// platform. One arm per supported OS; unsupported targets fail to
/// compile at the module-level `compile_error!` above, so this function
/// is a single `cfg` dispatch with no fallback.
pub fn default_sampler() -> std::sync::Arc<dyn Sampler> {
    #[cfg(target_os = "linux")]
    {
        std::sync::Arc::new(linux::ProcSampler::new())
    }
    #[cfg(target_os = "macos")]
    {
        std::sync::Arc::new(macos::LibprocSampler::new())
    }
    #[cfg(target_os = "windows")]
    {
        std::sync::Arc::new(windows::WindowsSampler::new())
    }
}

// ---------------------------------------------------------------------------
// Test harness — the fake sampler is part of the public API so integration
// tests in other crates can use it too.
// ---------------------------------------------------------------------------

/// Test sampler that returns a scripted sequence of samples and counts
/// how many times it was called. Public so integration tests outside
/// this crate can drive the monitor deterministically.
pub struct FakeSampler {
    calls: std::sync::Mutex<u64>,
    queue: std::sync::Mutex<std::collections::VecDeque<Sample>>,
    fallback: Sample,
}

impl FakeSampler {
    /// New sampler that returns `fallback` once the scripted queue is
    /// drained. Useful for tests that want "N scripted samples, then
    /// steady state".
    pub fn new(fallback: Sample) -> Self {
        Self {
            calls: std::sync::Mutex::new(0),
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            fallback,
        }
    }

    /// Enqueue a specific sample to be returned on the next `sample()`.
    pub fn enqueue(&self, s: Sample) {
        self.queue.lock().unwrap().push_back(s);
    }

    /// How many times `sample()` has been invoked. Lets a test probe
    /// "did the tick task keep running past drop?" by reading this
    /// number before and after `advance` / drop.
    pub fn calls(&self) -> u64 {
        *self.calls.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl Sampler for FakeSampler {
    async fn sample(&self, _pid: u32) -> Sample {
        *self.calls.lock().unwrap() += 1;
        self.queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(self.fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst() -> AgentInstanceId {
        AgentInstanceId::new()
    }

    fn sample(cpu: f64, rss: u64, fds: u64) -> Sample {
        Sample {
            cpu_seconds: Some(cpu),
            rss_bytes: Some(rss),
            fd_count: Some(fds),
        }
    }

    // Tests use a small real-time tick (20ms) so they complete quickly and
    // don't depend on paused-time cooperating with multi-task broadcast
    // channels. `cpu_seconds` fields are scaled proportionally so the
    // computed `cpu_pct` equals the pre-scale value. That is: with a 20ms
    // tick, `cpu_seconds: 0.002` yields 10% — the same intent as 0.1 with
    // a 1s tick, but without waiting a real second.
    const TEST_TICK: Duration = Duration::from_millis(20);
    /// Scale factor `tick / 1s`: multiply a "what cpu_pct do I want"
    /// fraction by this to get the `cpu_seconds` delta that produces it.
    const TICK_SCALE: f64 = 0.020;
    const RECV_TIMEOUT: Duration = Duration::from_secs(2);

    async fn recv_one(rx: &mut broadcast::Receiver<Event>) -> Event {
        tokio::time::timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("receiver should see a sample within RECV_TIMEOUT")
            .expect("broadcast must not lag or close")
    }

    #[tokio::test]
    async fn track_emits_a_resource_sample_after_one_tick() {
        // RED test for the core DoD item: "spawn a mock instance, advance
        // time, assert pill values update".
        let fake = Arc::new(FakeSampler::new(sample(0.1 * TICK_SCALE, 4096, 7)));
        let mon = ResourceMonitor::new(Arc::clone(&fake) as Arc<dyn Sampler>, TEST_TICK);
        let mut rx = mon.events();
        let id = inst();
        mon.track(id.clone(), 12345).await;

        let ev = recv_one(&mut rx).await;
        match ev {
            Event::ResourceSample {
                instance_id,
                cpu_pct,
                rss_bytes,
                fd_count,
                ..
            } => {
                assert_eq!(instance_id, id);
                assert_eq!(rss_bytes, Some(4096));
                assert_eq!(fd_count, Some(7));
                // 0.1 * TICK_SCALE seconds on-cpu in a TEST_TICK window →
                // 10% rolling avg (single sample in history).
                let pct = cpu_pct.expect("cpu_pct populated when cpu_seconds is Some");
                assert!((pct - 10.0).abs() < 0.5, "expected ~10% cpu_pct, got {pct}");
            }
            other => panic!("expected ResourceSample, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn track_respects_custom_tick() {
        // 10ms tick — first emission must arrive well before the 2s
        // receive timeout.
        let fake = Arc::new(FakeSampler::new(sample(0.0, 1, 1)));
        let mon = ResourceMonitor::new(
            Arc::clone(&fake) as Arc<dyn Sampler>,
            Duration::from_millis(10),
        );
        let mut rx = mon.events();
        let id = inst();
        mon.track(id, 12345).await;

        let _ = recv_one(&mut rx).await;
        assert!(fake.calls() >= 1, "fake sampler must have been called");
    }

    #[tokio::test]
    async fn untrack_stops_future_samples_for_that_id() {
        let fake = Arc::new(FakeSampler::new(sample(0.1 * TICK_SCALE, 1, 1)));
        let mon = ResourceMonitor::new(Arc::clone(&fake) as Arc<dyn Sampler>, TEST_TICK);
        let mut rx = mon.events();
        let id_a = inst();
        let id_b = inst();
        mon.track(id_a.clone(), 1).await;
        mon.track(id_b.clone(), 2).await;

        // Observe at least one sample from each tracked instance before
        // untracking A.
        let mut seen_a = false;
        let mut seen_b = false;
        for _ in 0..32 {
            let ev = recv_one(&mut rx).await;
            if let Event::ResourceSample { instance_id, .. } = ev {
                if instance_id == id_a {
                    seen_a = true;
                } else if instance_id == id_b {
                    seen_b = true;
                }
            }
            if seen_a && seen_b {
                break;
            }
        }
        assert!(seen_a && seen_b, "both tracked instances must emit");

        mon.untrack(&id_a).await;
        // Drain any samples already queued before `untrack` landed.
        while rx.try_recv().is_ok() {}
        // New ticks arriving after untrack must only produce B samples.
        let mut saw_b_after = false;
        for _ in 0..32 {
            let ev = recv_one(&mut rx).await;
            if let Event::ResourceSample { instance_id, .. } = ev {
                assert_ne!(
                    instance_id, id_a,
                    "untracked id must not emit further samples"
                );
                if instance_id == id_b {
                    saw_b_after = true;
                    break;
                }
            }
        }
        assert!(saw_b_after, "still-tracked id must continue emitting");
        assert_eq!(
            mon.tracked_count().await,
            1,
            "tasks map drops the untracked id"
        );
    }

    #[tokio::test]
    async fn dropping_monitor_aborts_outstanding_tasks() {
        // The DoD's second test invariant: "no sampler thread leaks on
        // instance drop". Probe via the fake sampler's call counter — if
        // the task survived drop, the counter would keep ticking.
        let fake = Arc::new(FakeSampler::new(sample(0.0, 1, 1)));
        let calls_at_drop = {
            let mon = ResourceMonitor::new(Arc::clone(&fake) as Arc<dyn Sampler>, TEST_TICK);
            mon.track(inst(), 1).await;
            mon.track(inst(), 2).await;
            // Wait for at least one sample from each task so we know the
            // ticker loops are actually running.
            let mut rx = mon.events();
            let mut seen = 0;
            while seen < 2 {
                if recv_one(&mut rx).await.is_resource_sample() {
                    seen += 1;
                }
            }
            fake.calls()
            // `mon` drops here; `Drop` aborts every task.
        };

        // Give any stray task several full tick windows to run — if drop
        // didn't actually abort them, the counter would increase.
        tokio::time::sleep(TEST_TICK * 10).await;
        let calls_after_drop = fake.calls();

        assert_eq!(
            calls_after_drop, calls_at_drop,
            "tick task must not survive ResourceMonitor::drop"
        );
    }

    #[tokio::test]
    async fn track_replaces_previous_task_for_same_id() {
        // Idempotency invariant: `track(id, pid_a)` followed by
        // `track(id, pid_b)` leaves exactly one task for `id`.
        let fake = Arc::new(FakeSampler::new(sample(0.0, 1, 1)));
        let mon = ResourceMonitor::new(Arc::clone(&fake) as Arc<dyn Sampler>, TEST_TICK);
        let id = inst();
        mon.track(id.clone(), 1).await;
        mon.track(id.clone(), 2).await;
        assert_eq!(
            mon.tracked_count().await,
            1,
            "re-tracking the same id must replace, not duplicate"
        );
    }

    #[tokio::test]
    async fn track_is_a_noop_when_pid_equals_daemon_pid() {
        // F-370: sampling the daemon's own PID yields session-wide numbers
        // shifted by unrelated cross-agent work, rendered by the UI as if
        // they were per-instance. Until a real per-child PID is available
        // (e.g. a future step executor that forks a provider sidecar),
        // `track(id, daemon_pid)` is a deliberate no-op — it registers no
        // task and emits no `ResourceSample`, so the AgentMonitor pills
        // stay at the `—` placeholder rather than streaming misleading
        // values.
        let fake = Arc::new(FakeSampler::new(sample(0.1 * TICK_SCALE, 4096, 7)));
        let mon = ResourceMonitor::new(Arc::clone(&fake) as Arc<dyn Sampler>, TEST_TICK);
        let mut rx = mon.events();
        let id = inst();

        mon.track(id.clone(), std::process::id()).await;

        assert_eq!(
            mon.tracked_count().await,
            0,
            "tracking the daemon PID must not register a task"
        );

        // Wait well past a tick to be certain no sample emits.
        let waited = tokio::time::timeout(TEST_TICK * 5, rx.recv()).await;
        assert!(
            waited.is_err(),
            "no ResourceSample should fire for a daemon-PID track, got {waited:?}"
        );
        assert_eq!(
            fake.calls(),
            0,
            "sampler must never be invoked for the daemon PID"
        );
    }

    #[tokio::test]
    async fn track_with_real_pid_after_daemon_pid_still_works() {
        // Regression guard: the daemon-PID no-op must not poison the
        // monitor for subsequent real-PID calls on the same id.
        let fake = Arc::new(FakeSampler::new(sample(0.1 * TICK_SCALE, 4096, 7)));
        let mon = ResourceMonitor::new(Arc::clone(&fake) as Arc<dyn Sampler>, TEST_TICK);
        let mut rx = mon.events();
        let id = inst();

        mon.track(id.clone(), std::process::id()).await;
        mon.track(id.clone(), std::process::id().wrapping_add(1))
            .await;

        let ev = recv_one(&mut rx).await;
        assert!(
            matches!(ev, Event::ResourceSample { instance_id, .. } if instance_id == id),
            "a subsequent real-PID track must start emitting samples"
        );
    }

    #[tokio::test]
    async fn cpu_pct_is_rolling_average_over_recent_samples() {
        // Two scripted samples so the first two emissions exercise the
        // rolling-average fold, then fall through to the fallback.
        let fake = Arc::new(FakeSampler::new(sample(0.0, 1, 1)));
        fake.enqueue(sample(0.1 * TICK_SCALE, 1, 1)); // 10% first tick
        fake.enqueue(sample(0.3 * TICK_SCALE, 1, 1)); // 30% second tick
        let mon = ResourceMonitor::new(Arc::clone(&fake) as Arc<dyn Sampler>, TEST_TICK);
        let mut rx = mon.events();
        let id = inst();
        mon.track(id, 1).await;

        let first = recv_one(&mut rx).await;
        let Event::ResourceSample { cpu_pct: pct1, .. } = first else {
            panic!("wrong variant")
        };
        let pct1 = pct1.unwrap();
        assert!(
            (pct1 - 10.0).abs() < 0.5,
            "first tick → 10% only, got {pct1}"
        );

        let second = recv_one(&mut rx).await;
        let Event::ResourceSample { cpu_pct: pct2, .. } = second else {
            panic!("wrong variant")
        };
        let pct2 = pct2.unwrap();
        assert!(
            (pct2 - 20.0).abs() < 0.5,
            "second tick is avg(10, 30) = 20%, got {pct2}"
        );
    }

    #[tokio::test]
    async fn missing_cpu_seconds_produces_none_cpu_pct_on_first_sample() {
        // When the platform probe can't read CPU (all None), the rolling
        // history stays empty and the emitted `cpu_pct` is None. RSS/fds
        // are preserved verbatim.
        let fake = Arc::new(FakeSampler::new(Sample {
            cpu_seconds: None,
            rss_bytes: Some(4096),
            fd_count: Some(3),
        }));
        let mon = ResourceMonitor::new(Arc::clone(&fake) as Arc<dyn Sampler>, TEST_TICK);
        let mut rx = mon.events();
        mon.track(inst(), 1).await;

        let ev = recv_one(&mut rx).await;
        match ev {
            Event::ResourceSample {
                cpu_pct,
                rss_bytes,
                fd_count,
                ..
            } => {
                assert_eq!(cpu_pct, None);
                assert_eq!(rss_bytes, Some(4096));
                assert_eq!(fd_count, Some(3));
            }
            other => panic!("expected ResourceSample, got {other:?}"),
        }
    }

    // Small helper keeps the drop test readable.
    trait IsResourceSample {
        fn is_resource_sample(&self) -> bool;
    }
    impl IsResourceSample for Event {
        fn is_resource_sample(&self) -> bool {
            matches!(self, Event::ResourceSample { .. })
        }
    }
}
