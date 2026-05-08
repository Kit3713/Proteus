// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-kind rate limiter for the event-trigger registry. Issue #254.
//!
//! Why this exists: a hostile or buggy event source (a flapping NIC,
//! an AP that toggles BSSID every 200 ms, a regulatory-domain ping
//! pong from a misbehaving driver) can fire `RotationTrigger`s
//! faster than the rotation handler can process them. Each rotation
//! writes state, opens DBus, and emits log lines — running tens per
//! second is at best a log-flood and at worst a DoS that wedges
//! NetworkManager. The registry now consults this limiter before
//! dispatching: after the Nth trigger of the same kind inside the
//! window, further triggers of that kind are dropped (with one warn
//! line per drop event, but coalesced — see `note_overflow`).
//!
//! Design constraints we cared about:
//! - **No new top-level deps.** The instructions explicitly forbid
//!   pulling in `lru` or `linked_hash_map`. Stdlib only.
//! - **Per-kind isolation.** A flap-happy interface can't starve the
//!   regulatory-domain channel; the limiter keys on
//!   `RotationTrigger::kind()` (the four stable string tokens).
//! - **Bounded memory.** No unbounded growth even under sustained
//!   high-rate firing. `VecDeque<Instant>` per kind is naturally
//!   trimmed each call (see `prune`).
//! - **Cheap.** Every fire path takes the same lock once, prunes,
//!   compares against the cap, and either pushes or returns.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default sliding-window length. One minute is generous enough that
/// a real captive-portal authentication burst (NM reports
/// `connection-up` 2-3 times in quick succession) clears, but tight
/// enough to bound the worst case to 60 events per kind per minute
/// of CPU work.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);

/// Default per-kind cap inside the window. The four real-world
/// trigger kinds (`connection-up`, `link-flap`, `reg-domain-change`,
/// `portal-auth`) all genuinely fire <10/min in even an aggressive
/// roam scenario; capping at 10 lets a real burst through and still
/// catches a runaway source.
pub const DEFAULT_MAX_PER_WINDOW: usize = 10;

/// Per-kind rate limiter. Cheap to construct; cheap to clone via the
/// `Arc` the registry holds. Internally a single mutex over a small
/// HashMap of `&'static str → VecDeque<Instant>`. We trim each
/// kind's deque on every hit so memory stays bounded by
/// `kinds * max_per_window` (4 * 10 = 40 instants for the live
/// trigger set).
#[derive(Debug)]
pub struct RateLimiter {
    inner: Mutex<Inner>,
    window: Duration,
    max: usize,
}

#[derive(Debug, Default)]
struct Inner {
    /// `kind → recent fire instants, in ascending order`. Trimmed on
    /// every `check_and_record` call so the deque length never
    /// exceeds `max + 1` between two consecutive operations.
    seen: HashMap<&'static str, std::collections::VecDeque<Instant>>,
    /// `kind → (overflow_count_since_last_warn, last_warn_at)`. Used
    /// by `note_overflow` to coalesce the warn-line burst that
    /// happens right after we hit the cap. Without coalescing a
    /// flapping source would flood journald exactly as badly as the
    /// thing we're rate-limiting against; with it, the operator sees
    /// one warn line per kind per overflow burst plus a periodic
    /// "still dropping" reminder no more than once per window.
    overflow: HashMap<&'static str, (u64, Instant)>,
}

/// Outcome of a `check_and_record`. `Allowed` means dispatch should
/// proceed; `RateLimited` means the caller should drop the trigger
/// (and the limiter has already accounted for the drop, so a
/// subsequent `note_overflow` call merely decides whether to log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    /// Trigger should be dropped. The carried count is the running
    /// number of consecutive drops for this kind since the last
    /// `Allowed` — useful for the warn-line coalesce path. `u64`
    /// because a hostile source could in principle drive it past
    /// `u32::MAX` over a long uptime.
    RateLimited(u64),
}

impl RateLimiter {
    /// Build a limiter with the default cadence (10 triggers per
    /// kind per minute).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_PER_WINDOW, DEFAULT_WINDOW)
    }

    /// Build a limiter with explicit knobs. Mostly for tests; the
    /// default ctor is what the orchestrator uses.
    pub fn with_capacity(max: usize, window: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            window,
            max,
        }
    }

    /// Decide whether a fresh trigger of `kind` should be dispatched.
    /// On `Allowed`, the trigger is recorded against the limiter's
    /// budget. On `RateLimited`, no recording happens (the caller is
    /// expected to drop) and the carried count tracks consecutive
    /// drops for warn-coalescing.
    ///
    /// `now` is injected so tests don't have to sleep. Production
    /// callers pass `Instant::now()`.
    pub fn check_and_record(&self, kind: &'static str, now: Instant) -> Decision {
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        let entry = inner.seen.entry(kind).or_default();
        // Trim: drop entries strictly older than the cutoff.
        while let Some(front) = entry.front()
            && *front < cutoff
        {
            entry.pop_front();
        }
        if entry.len() >= self.max {
            // Bump the consecutive-drops counter for this kind.
            let slot = inner.overflow.entry(kind).or_insert((0, now));
            slot.0 = slot.0.saturating_add(1);
            return Decision::RateLimited(slot.0);
        }
        entry.push_back(now);
        // A successful dispatch resets the consecutive-drops counter.
        if let Some(slot) = inner.overflow.get_mut(kind) {
            slot.0 = 0;
            slot.1 = now;
        }
        Decision::Allowed
    }

    /// Decide whether to emit a warn line for the most recent drop
    /// of `kind`. Returns `Some(consecutive_drops)` if the caller
    /// should log; `None` if a previous warn already covered this
    /// burst within the window.
    ///
    /// Coalesce policy: the first drop in a streak always logs, then
    /// we wait until the window elapses since the last warn before
    /// logging again. That keeps the journal honest about ongoing
    /// drops without flooding it.
    pub fn note_overflow(&self, kind: &'static str, now: Instant) -> Option<u64> {
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let entry = inner.overflow.entry(kind).or_insert((0, now));
        // First overflow in the streak (overflow count went from 0→1
        // in the same fire that called us, so the count here is
        // exactly 1 the first time around): log.
        if entry.0 == 1 {
            entry.1 = now;
            return Some(entry.0);
        }
        // Subsequent overflows: rate-limit the warn itself to once
        // per window so log volume is bounded.
        if now.duration_since(entry.1) >= self.window {
            entry.1 = now;
            return Some(entry.0);
        }
        None
    }

    /// Test/inspection helper: number of recorded entries for a kind
    /// in the current window. Pruned at the time of inspection.
    pub fn recorded(&self, kind: &'static str, now: Instant) -> usize {
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        let entry = inner.seen.entry(kind).or_default();
        while let Some(front) = entry.front()
            && *front < cutoff
        {
            entry.pop_front();
        }
        entry.len()
    }

    /// Configured cap. Pinned so config-wiring tests can assert.
    pub fn max(&self) -> usize {
        self.max
    }

    /// Configured window. Pinned for tests.
    pub fn window(&self) -> Duration {
        self.window
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// First N triggers of a kind go through; the (N+1)th and
    /// beyond are rate-limited.
    #[test]
    fn nth_trigger_in_window_is_dropped() {
        let lim = RateLimiter::with_capacity(3, Duration::from_secs(60));
        let t0 = Instant::now();
        for i in 0..3 {
            assert_eq!(
                lim.check_and_record("link-flap", t0 + Duration::from_millis(i * 10)),
                Decision::Allowed,
                "first 3 must be allowed"
            );
        }
        // 4th: dropped.
        match lim.check_and_record("link-flap", t0 + Duration::from_millis(40)) {
            Decision::RateLimited(n) => assert_eq!(n, 1),
            other => panic!("expected RateLimited(1), got {other:?}"),
        }
        // 5th: dropped, count incremented.
        match lim.check_and_record("link-flap", t0 + Duration::from_millis(50)) {
            Decision::RateLimited(n) => assert_eq!(n, 2),
            other => panic!("expected RateLimited(2), got {other:?}"),
        }
    }

    /// Once entries age out of the window the limiter accepts new
    /// triggers again — no permanent ban.
    #[test]
    fn entries_outside_window_are_pruned() {
        let lim = RateLimiter::with_capacity(2, Duration::from_secs(1));
        let t0 = Instant::now();
        assert_eq!(lim.check_and_record("connection-up", t0), Decision::Allowed);
        assert_eq!(
            lim.check_and_record("connection-up", t0 + Duration::from_millis(100)),
            Decision::Allowed
        );
        // Cap reached.
        assert!(matches!(
            lim.check_and_record("connection-up", t0 + Duration::from_millis(200)),
            Decision::RateLimited(_)
        ));
        // Wait past the window — both entries age out.
        let t_far = t0 + Duration::from_secs(5);
        assert_eq!(
            lim.check_and_record("connection-up", t_far),
            Decision::Allowed
        );
    }

    /// Per-kind isolation: filling up one kind doesn't block others.
    #[test]
    fn kinds_have_independent_budgets() {
        let lim = RateLimiter::with_capacity(2, Duration::from_secs(60));
        let t = Instant::now();
        assert_eq!(lim.check_and_record("link-flap", t), Decision::Allowed);
        assert_eq!(lim.check_and_record("link-flap", t), Decision::Allowed);
        assert!(matches!(
            lim.check_and_record("link-flap", t),
            Decision::RateLimited(_)
        ));
        // A different kind sees a fresh budget.
        assert_eq!(lim.check_and_record("portal-auth", t), Decision::Allowed);
        assert_eq!(lim.check_and_record("portal-auth", t), Decision::Allowed);
        assert!(matches!(
            lim.check_and_record("portal-auth", t),
            Decision::RateLimited(_)
        ));
        // First kind still rate-limited.
        assert!(matches!(
            lim.check_and_record("link-flap", t),
            Decision::RateLimited(_)
        ));
    }

    /// First overflow logs; subsequent overflows in the same window
    /// don't (coalesce). After the window elapses the next overflow
    /// logs again.
    #[test]
    fn note_overflow_coalesces_within_window() {
        let lim = RateLimiter::with_capacity(1, Duration::from_secs(10));
        let t = Instant::now();
        // Saturate.
        assert_eq!(
            lim.check_and_record("reg-domain-change", t),
            Decision::Allowed
        );
        // First overflow.
        assert!(matches!(
            lim.check_and_record("reg-domain-change", t),
            Decision::RateLimited(1)
        ));
        assert_eq!(lim.note_overflow("reg-domain-change", t), Some(1));
        // Second overflow inside the window: coalesced.
        assert!(matches!(
            lim.check_and_record("reg-domain-change", t + Duration::from_millis(100)),
            Decision::RateLimited(2)
        ));
        assert_eq!(
            lim.note_overflow("reg-domain-change", t + Duration::from_millis(100)),
            None
        );
        // After the window: the original entry ages out, so the next
        // dispatch goes through. To re-enter overflow we must
        // saturate again, then drop. The warn-line cooldown was the
        // last warn at `t`; we're now well past `t + window`, so the
        // re-entered overflow logs again.
        let later = t + Duration::from_secs(15);
        // First trigger past the window: allowed (entry aged out).
        assert_eq!(
            lim.check_and_record("reg-domain-change", later),
            Decision::Allowed
        );
        // Second trigger immediately after: dropped.
        let later2 = later + Duration::from_millis(50);
        assert!(matches!(
            lim.check_and_record("reg-domain-change", later2),
            Decision::RateLimited(_)
        ));
        // The warn cooldown has elapsed since the last `note_overflow`
        // at `t + 100ms` (we're at `t + 15s + 50ms`), so the new burst
        // logs again. Note: a successful `Allowed` reset the streak
        // counter, so the next overflow is once again `1`.
        assert!(lim.note_overflow("reg-domain-change", later2).is_some());
    }

    /// `recorded(kind)` reports the live (post-prune) count.
    #[test]
    fn recorded_reflects_live_count() {
        let lim = RateLimiter::with_capacity(5, Duration::from_secs(1));
        let t = Instant::now();
        for _ in 0..3 {
            lim.check_and_record("link-flap", t);
        }
        assert_eq!(lim.recorded("link-flap", t), 3);
        let t_far = t + Duration::from_secs(5);
        assert_eq!(
            lim.recorded("link-flap", t_far),
            0,
            "all entries should age out past the window"
        );
    }

    /// Default knobs match the documented contract — pin these so a
    /// future refactor can't silently change the operational budget.
    #[test]
    fn defaults_pin_to_documented_values() {
        let lim = RateLimiter::new();
        assert_eq!(lim.max(), 10);
        assert_eq!(lim.window(), Duration::from_secs(60));
    }

    /// Successful dispatch resets the consecutive-overflow counter so
    /// a future overflow streak starts from 1 (and thus logs again).
    #[test]
    fn successful_dispatch_resets_overflow_counter() {
        let lim = RateLimiter::with_capacity(2, Duration::from_secs(60));
        let t = Instant::now();
        lim.check_and_record("link-flap", t);
        lim.check_and_record("link-flap", t);
        // First overflow.
        assert!(matches!(
            lim.check_and_record("link-flap", t),
            Decision::RateLimited(1)
        ));
        assert_eq!(lim.note_overflow("link-flap", t), Some(1));
        // Long after — entries age out, dispatch succeeds, counter
        // resets.
        let later = t + Duration::from_secs(120);
        assert_eq!(lim.check_and_record("link-flap", later), Decision::Allowed);
        // Saturate again to force another overflow.
        lim.check_and_record("link-flap", later);
        // Now a fresh overflow streak begins.
        match lim.check_and_record("link-flap", later) {
            Decision::RateLimited(n) => assert_eq!(n, 1, "counter should have reset"),
            other => panic!("expected RateLimited(1), got {other:?}"),
        }
    }
}
