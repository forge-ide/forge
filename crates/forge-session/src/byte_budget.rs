//! Per-session aggregate byte budget (F-077).
//!
//! Per-op caps in `forge-fs` (10 MiB) and `forge-providers`
//! (1 MiB / 4 MiB per-line) bound the size of a single tool invocation
//! but do not compose into a session ceiling: a tool-chained adversary
//! can issue 1000 within-cap calls and exhaust host memory without
//! tripping any per-op limit. `ByteBudget` is the missing aggregate —
//! a monotonically-increasing counter shared across every tool
//! invocation in a session, refusing further ops once the configured
//! limit is reached.
//!
//! # Semantics
//!
//! Enforcement is **post-decrement**: the dispatcher executes the tool,
//! charges the budget by the bytes the result actually consumed
//! (content / stdout / stderr), then refuses the *next* call if the
//! budget is exhausted. A single op that overshoots the cap is allowed
//! to complete — the next call is refused. This matches the brief's
//! "refuses further ops when exhausted" wording and avoids forcing
//! tools to pre-declare their output size (`shell.exec` cannot know
//! its stdout volume until the child exits).
//!
//! Refusal happens at the `ToolDispatcher` boundary so every tool routes
//! through the same gate. The tool itself never runs after exhaustion;
//! the dispatcher returns `{"error": "session byte budget exceeded:
//! <consumed>/<limit> bytes"}` directly.
//!
//! # Default
//!
//! [`ByteBudget::default`] is **500 MiB** per session. The number is
//! large enough that a normal session of fs.read / fs.write / shell.exec
//! calls never trips it, and small enough that a runaway loop cannot
//! exhaust desktop or CI memory before the daemon refuses. Tests
//! configure smaller budgets (1 MiB-class) to exercise the boundary
//! without paying the memory cost of the production default.

use std::sync::atomic::{AtomicU64, Ordering};

/// Default aggregate byte budget per session: 500 MiB. See module docs.
pub const DEFAULT_BUDGET_BYTES: u64 = 500 * 1024 * 1024;

/// Monotonic per-session counter of bytes consumed by tool results.
///
/// `Ordering::Relaxed` is sufficient for both the load and the
/// fetch-add: the budget enforces a *cumulative* ceiling, not
/// happens-before ordering between writes. A single op may race past
/// the limit by the size of one in-flight tool call — that is the
/// "single op overshoot" already documented in the module preamble,
/// not a correctness gap.
#[derive(Debug)]
pub struct ByteBudget {
    consumed: AtomicU64,
    limit: u64,
}

impl ByteBudget {
    /// Construct a budget with `limit` bytes of headroom.
    pub fn new(limit: u64) -> Self {
        Self {
            consumed: AtomicU64::new(0),
            limit,
        }
    }

    /// Bytes consumed so far.
    pub fn consumed(&self) -> u64 {
        self.consumed.load(Ordering::Relaxed)
    }

    /// Configured ceiling.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// True iff `consumed() >= limit()` — subsequent dispatch calls
    /// will be refused.
    pub fn is_exhausted(&self) -> bool {
        self.consumed() >= self.limit
    }

    /// Record `bytes` against the budget. Saturating add prevents
    /// counter wrap on a pathologically long-lived session: once
    /// the counter reaches `u64::MAX` the budget stays exhausted.
    pub fn charge(&self, bytes: u64) {
        let mut current = self.consumed.load(Ordering::Relaxed);
        loop {
            let next = current.saturating_add(bytes);
            match self.consumed.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }
}

impl Default for ByteBudget {
    /// 500 MiB default budget (`DEFAULT_BUDGET_BYTES`).
    fn default() -> Self {
        Self::new(DEFAULT_BUDGET_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_at_zero_consumed() {
        let b = ByteBudget::new(1024);
        assert_eq!(b.consumed(), 0);
        assert_eq!(b.limit(), 1024);
        assert!(!b.is_exhausted());
    }

    #[test]
    fn charge_accumulates() {
        let b = ByteBudget::new(1024);
        b.charge(100);
        b.charge(200);
        assert_eq!(b.consumed(), 300);
        assert!(!b.is_exhausted());
    }

    #[test]
    fn is_exhausted_at_or_above_limit() {
        let b = ByteBudget::new(1024);
        b.charge(1024);
        assert!(b.is_exhausted());
        b.charge(1);
        assert!(b.is_exhausted());
    }

    #[test]
    fn charge_saturates_on_overflow() {
        let b = ByteBudget::new(u64::MAX);
        b.charge(u64::MAX - 10);
        b.charge(100); // would overflow without saturating_add
        assert_eq!(b.consumed(), u64::MAX);
    }

    #[test]
    fn default_is_500_mib() {
        assert_eq!(ByteBudget::default().limit(), 500 * 1024 * 1024);
        assert_eq!(DEFAULT_BUDGET_BYTES, 500 * 1024 * 1024);
    }
}
