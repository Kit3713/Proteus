// SPDX-License-Identifier: GPL-3.0-or-later

//! Advisory file lock around `/var/lib/proteus/.lock`.
//!
//! Issue #126: two concurrent `proteus apply` (or `rotate`, `pin`, `revert`,
//! ...) runs would race on `state.json` and could corrupt the cached
//! originals. We serialize them with an exclusive `flock(2)` on a sidecar
//! file in the state directory.
//!
//! The lock is best-effort and intentionally simple:
//!
//! - Open `<state-dir>/.lock` (creating it if needed) with O_CLOEXEC so a
//!   forked subprocess can't inherit the lock by accident.
//! - Try `LOCK_EX | LOCK_NB`; if held, retry a handful of times with a
//!   short sleep, then bail with `LockError::Busy`.
//! - The returned [`StateLockGuard`] holds the fd; `Drop` closes it, which
//!   the kernel translates to `LOCK_UN`.
//!
//! Reentrancy: a single Proteus process may legitimately call multiple
//! mutating helpers in sequence — most obviously `apply::run` calling
//! `rotate::run`, `bluetooth_cmd::apply`, ... directly. To stay safe under
//! that pattern we keep a process-wide `Mutex<Option<File>>`: the first
//! call acquires the real flock and tucks the `File` inside the option,
//! nested calls observe the option is already `Some` and return a no-op
//! guard, and the outermost `Drop` releases the lock and clears the slot.
//! Different *processes* still contend on the kernel-level flock the way
//! the design intends.
//!
//! Issue #206-B: the previous shape used a process-wide `AtomicBool` plus
//! a separate `OnceLock<File>` slot. With the Milestone 1 backend trait
//! coming in, `acquire_for_state_path` will be called from async event
//! loops (the connection-up watcher; Milestone 4c's event-driven
//! framework) where multiple tasks can land on the call simultaneously.
//! The atomic + once-lock combination interleaved badly under that
//! pattern: Task A wins the CAS, releases on drop, then Task B observes
//! the bool is `false` but the OnceLock still holds the (now-released)
//! fd. Migrating to `Mutex<Option<File>>` keeps the external contract
//! (RAII guard, nested-acquire-is-no-op) but makes the inner state safe
//! to read and mutate atomically under any scheduling.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use thiserror::Error;

use crate::state::{STATE_FILE_MODE, ensure_state_dir_secure};

/// Number of attempts the foreground command makes before giving up on a
/// busy lock. With [`RETRY_DELAY`] this caps total wait at the default
/// 5s budget below — long enough for an interactive `proteus apply` to
/// outlast a systemd-timer rotate that holds the lock for the full
/// MAC + DBus + state-write cycle.
///
/// Issue #203: total budget is `RETRY_DELAY * RETRY_ATTEMPTS`. The
/// `PROTEUS_LOCK_TIMEOUT_MS` env var overrides the budget; values below
/// 100 ms (the retry granularity) are clamped up. systemd dispatcher /
/// timer units set this to 10000 ms in their drop-in so a long
/// orchestrator run doesn't lose a contention race to a quickly-retrying
/// follow-up dispatcher invocation.
const DEFAULT_LOCK_BUDGET_MS: u64 = 5_000;
/// Issue #221: hard cap on `PROTEUS_LOCK_TIMEOUT_MS` so absurd values
/// don't overflow the budget→attempts conversion.
///
/// C8: lowered from 1h to 2 minutes. The shipped systemd drop-in sets
/// 10s and the documented foreground UX expects "blocks for at most a
/// few seconds" — a 1h cap meant a hostile or fat-fingered env value
/// could pin a `proteus apply` for an entire hour with the lock held.
/// Anything that legitimately needs more than 2 min is misusing the
/// API and should restructure rather than crank this knob.
const MAX_LOCK_BUDGET_MS: u64 = 120_000;
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(100);

/// Resolve the retry budget from `PROTEUS_LOCK_TIMEOUT_MS` (falling back
/// to [`DEFAULT_LOCK_BUDGET_MS`]) and convert it to a retry-attempt count.
/// Pulled out so tests can exercise the parser without spawning a process.
///
/// Issue #221: the budget is clamped to [granularity_ms, MAX_LOCK_BUDGET_MS]
/// before the divide, so a hostile or fat-fingered `PROTEUS_LOCK_TIMEOUT_MS`
/// like `u64::MAX` can no longer overflow the `+ granularity_ms - 1` step
/// (debug panic / release wrap to ~0 attempts → instant Busy DoS). The 1h
/// upper bound is generous: the documented systemd drop-in sets 10s.
fn retry_attempts_from_env(get: impl Fn(&str) -> Option<String>) -> u32 {
    let budget_ms = get("PROTEUS_LOCK_TIMEOUT_MS")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_LOCK_BUDGET_MS);
    let granularity_ms = RETRY_DELAY.as_millis() as u64;
    let budget_ms = budget_ms.clamp(granularity_ms, MAX_LOCK_BUDGET_MS);
    let attempts = budget_ms.div_ceil(granularity_ms);
    attempts.min(u32::MAX as u64) as u32
}

const LOCK_FILE_NAME: &str = ".lock";

#[derive(Debug, Error)]
pub enum LockError {
    #[error("another proteus process holds the state lock at {path}; retry shortly")]
    Busy { path: PathBuf },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Process-wide owner of the kernel-level flock. `None` means no
/// process scope holds the lock; `Some(file)` means the outermost
/// scope holds it through `file`'s fd. Wrapped in a `Mutex` so the
/// trait's async callers can race on `acquire_for_state_path` without
/// the previous AtomicBool/OnceLock interleaving (issue #206-B).
static HELD: Mutex<Option<File>> = Mutex::new(None);

/// Test-only serialization mutex. Cross-module tests (e.g. in
/// `commands::tests`) that touch `HELD` must acquire this so they don't
/// race with the lock-module's own tests when cargo runs tests in parallel.
#[cfg(test)]
pub(crate) static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Re-acquire `TEST_SERIAL` even after a poisoned lock so a panic in one
/// test doesn't take down the rest of the suite.
#[cfg(test)]
pub(crate) fn test_serial_guard() -> std::sync::MutexGuard<'static, ()> {
    TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// RAII guard. Carries an `outermost` flag set on the first acquire in
/// this process; nested acquires receive `outermost = false` and `Drop`
/// is a no-op for them. The real lock is owned by the static `HELD`
/// slot — the outermost guard's `Drop` clears it.
pub struct StateLockGuard {
    outermost: bool,
}

impl Drop for StateLockGuard {
    fn drop(&mut self) {
        if !self.outermost {
            return;
        }
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = held.take() {
            // Best-effort explicit unlock so the lock is released even on
            // exotic platforms where close() takes its time. close() will
            // also drop the lock; we just hint to the kernel sooner.
            unsafe {
                libc::flock(file.as_raw_fd(), libc::LOCK_UN);
            }
            // `file` drops here, closing the fd.
            drop(file);
        }
    }
}

/// Acquire the advisory lock co-located with `state_path`. The lock file is
/// `<state-dir>/.lock`; we never lock `state.json` itself so we don't have
/// to coordinate with `write_atomic`'s rename-over-target step.
///
/// Returns a no-op guard if this process already holds the lock — the
/// orchestrator pattern relies on this.
pub fn acquire_for_state_path(state_path: &Path) -> Result<StateLockGuard, LockError> {
    // Issue C1 / N12.13: do NOT hold the `HELD` mutex across the retry-sleep
    // loop. The previous shape took the guard at the top of this function
    // and dropped it only after `acquire_inner` returned, which meant a
    // contended kernel flock pinned the in-process Mutex for the full 5 s
    // budget — every other tokio task or thread asking for the lock
    // serialized behind it instead of contending on the kernel flock that
    // is the actual contention point. We now take the mutex only for the
    // brief windows where we read or write the slot.
    {
        let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        if held.is_some() {
            // Already held by an outer scope in this same process; the
            // kernel flock would deadlock if we re-acquired, so hand back
            // a no-op guard.
            return Ok(StateLockGuard { outermost: false });
        }
    }
    let path = lock_path_for(state_path);
    let file = acquire_inner(&path)?;
    // Re-check the slot under the mutex in case another caller in this
    // process won the race to the kernel flock between our read above and
    // our write here. If so, drop our freshly-acquired file (which
    // releases the kernel flock via close()) and surface a no-op guard so
    // the caller still observes the reentrant contract.
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if held.is_some() {
        // Best-effort explicit unlock so the kernel hears about it before
        // the close() that's about to drop the file.
        unsafe {
            libc::flock(file.as_raw_fd(), libc::LOCK_UN);
        }
        drop(file);
        return Ok(StateLockGuard { outermost: false });
    }
    *held = Some(file);
    Ok(StateLockGuard { outermost: true })
}

fn lock_path_for(state_path: &Path) -> PathBuf {
    // N12.15: a bare filename like `proteus apply --state state.json`
    // has `parent() == Some("")` (an empty path), which `join` collapses
    // to `.lock` in $CWD — so a second invocation from a different cwd
    // does not see the lock as held. Canonicalize the parent to the
    // current directory so the lock-file path is absolute regardless of
    // how the caller spelled the state path. Best-effort: if
    // canonicalize fails we fall back to the previous behaviour rather
    // than crashing the lock acquire.
    let parent = state_path.parent();
    let dir: PathBuf = match parent {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    dir.join(LOCK_FILE_NAME)
}

fn acquire_inner(path: &Path) -> Result<File, LockError> {
    if let Some(parent) = path.parent() {
        // Issue #275: tighten the state-dir mode regardless of umask so a
        // pre-existing 0o755 dir cannot leave .lock world-writable.
        ensure_state_dir_secure(parent)
            .with_context(|| format!("securing lock dir {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        // Issue #275: don't fall back to umask for the lock-file mode.
        // 0o600 matches state.json (the file the lock guards) — no other
        // user has any reason to read or take this flock.
        .mode(STATE_FILE_MODE)
        // Security audit N-3: `O_NOFOLLOW` so a symlink planted at the
        // lock-file path errors out instead of being followed. The
        // state directory is 0o700 (root-only), but a stale symlink
        // could be left behind by a buggy revert or test fixture; if a
        // local attacker had any window where the dir was world-
        // writable (or a different user briefly owned it), a symlink
        // there would otherwise let them steer where Proteus took its
        // flock. `errno = ELOOP` if the path resolves to a symlink at
        // the final component; the surrounding `with_context` carries
        // that through to the operator.
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("opening lock file {}", path.display()))?;
    // OpenOptions::mode() only takes effect when the file is freshly
    // created. If `.lock` already exists with a wider mode (left over
    // from a pre-#275 install), tighten it now.
    //
    // GH #370: tighten via `fchmod` on the open fd rather than `chmod`
    // on the path so an attacker cannot swap the path between open()
    // and chmod() (TOCTOU). With `O_NOFOLLOW` plus `fchmod` on the
    // already-open fd, both halves of audit N-3 are closed.
    let rc = unsafe { libc::fchmod(file.as_raw_fd(), STATE_FILE_MODE as libc::mode_t) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        return Err(LockError::Other(anyhow::anyhow!(
            "fchmod 0{STATE_FILE_MODE:o} on lock file {}: {err}",
            path.display(),
        )));
    }

    let retry_attempts = retry_attempts_from_env(|k| std::env::var(k).ok());
    // Try LOCK_EX|LOCK_NB up to retry_attempts times with a small sleep so a
    // contender that's about to release doesn't force an immediate failure.
    for _ in 0..retry_attempts {
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            return Ok(file);
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EWOULDBLOCK) => {
                std::thread::sleep(RETRY_DELAY);
                continue;
            }
            _ => return Err(LockError::Io(err)),
        }
    }
    Err(LockError::Busy {
        path: path.to_path_buf(),
    })
}

/// True iff this process currently holds the state lock at the
/// outermost scope. Test-only so the guard's invariants stay visible
/// to the test suite.
#[cfg(test)]
pub(crate) fn is_held_in_process() -> bool {
    HELD.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// All tests in this module mutate the process-wide `HELD` slot (or the
    /// kernel-level lock at the same path). Cargo runs tests in parallel by
    /// default, so we serialize them with [`TEST_SERIAL`] to avoid one
    /// test's acquire stomping on another's assertions.
    fn serial_guard() -> std::sync::MutexGuard<'static, ()> {
        test_serial_guard()
    }

    #[test]
    fn lock_path_lives_in_state_dir() {
        let _serial = serial_guard();
        let state = Path::new("/var/lib/proteus/state.json");
        assert_eq!(
            lock_path_for(state),
            Path::new("/var/lib/proteus/.lock").to_path_buf()
        );
    }

    #[test]
    fn acquire_then_drop_releases_for_next_acquire() {
        let _serial = serial_guard();
        let dir =
            std::env::temp_dir().join(format!("proteus-lock-roundtrip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("state.json");
        {
            let _g = acquire_for_state_path(&state).expect("first acquire");
            assert!(is_held_in_process());
        }
        // After drop, the slot is clear and a fresh acquire works.
        assert!(!is_held_in_process());
        let _g = acquire_for_state_path(&state).expect("second acquire after drop");
        drop(_g);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_acquire_in_same_process_is_a_no_op() {
        let _serial = serial_guard();
        // The orchestrator (apply::run) acquires once and then calls
        // sub-commands that also try to acquire. Reentrant calls must not
        // deadlock and must not release the outer lock when they drop.
        let dir = std::env::temp_dir().join(format!("proteus-lock-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("state.json");

        let outer = acquire_for_state_path(&state).expect("outer acquire");
        assert!(is_held_in_process());
        {
            let inner = acquire_for_state_path(&state).expect("nested acquire");
            // Inner guard should be a no-op (no fd owned).
            assert!(!inner.outermost);
            drop(inner);
            // Outer lock must still be held.
            assert!(is_held_in_process());
        }
        drop(outer);
        assert!(!is_held_in_process());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn busy_when_external_process_holds_lock() {
        let _serial = serial_guard();
        // Simulate the cross-process race: open the lock file in a separate
        // fd, take the flock, then try to acquire in this process. The
        // in-process HELD slot is per-process, so we have to reach the
        // kernel flock to hit the contention path. We verify by manually
        // calling the inner helper, which bypasses the HELD shortcut.
        let dir = std::env::temp_dir().join(format!("proteus-lock-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("state.json");
        let lock = lock_path_for(&state);

        // Hold the lock via a "foreign" fd (different File, same path).
        let foreign = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock)
            .unwrap();
        let rc = unsafe { libc::flock(foreign.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(rc, 0, "test setup: should acquire foreign flock");

        let result = acquire_inner(&lock);
        assert!(
            matches!(result, Err(LockError::Busy { .. })),
            "expected LockError::Busy when contending with foreign fd, got {result:?}"
        );

        // Release foreign lock, drop file.
        unsafe {
            libc::flock(foreign.as_raw_fd(), libc::LOCK_UN);
        }
        drop(foreign);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #203: `PROTEUS_LOCK_TIMEOUT_MS` overrides the default 5-second
    /// retry budget. Below the 100 ms granularity, the parser clamps up so
    /// at least one attempt always runs. Garbage input falls back to default.
    #[test]
    fn retry_attempts_from_env_honours_override() {
        // Default budget = 5_000 ms / 100 ms granularity = 50 attempts.
        let attempts = retry_attempts_from_env(|_| None);
        assert_eq!(attempts, 50);

        // 10s override → 100 attempts (matches what the systemd timer drop-in sets).
        let attempts = retry_attempts_from_env(|_| Some("10000".to_string()));
        assert_eq!(attempts, 100);

        // Below 100 ms clamps up to 1 attempt.
        let attempts = retry_attempts_from_env(|_| Some("50".to_string()));
        assert_eq!(attempts, 1);

        // Garbage input falls back to default.
        let attempts = retry_attempts_from_env(|_| Some("not-a-number".to_string()));
        assert_eq!(attempts, 50);
    }

    /// Issue #221 / C8: hostile or fat-fingered `PROTEUS_LOCK_TIMEOUT_MS`
    /// values must not overflow the budget→attempts conversion AND must
    /// not pin Proteus for absurd durations. C8 lowered the cap from 1h
    /// to 2 min so a typo can no longer block apply for an entire hour.
    #[test]
    fn retry_attempts_from_env_clamps_oversized_budget() {
        // 2 min budget = 120_000 ms / 100 ms granularity = 1200 attempts.
        const CAP_ATTEMPTS: u32 = 1_200;

        // u64::MAX → clamped to MAX_LOCK_BUDGET_MS (2 min).
        let attempts = retry_attempts_from_env(|_| Some(u64::MAX.to_string()));
        assert_eq!(attempts, CAP_ATTEMPTS);

        // Exactly at the cap.
        let attempts = retry_attempts_from_env(|_| Some("120000".to_string()));
        assert_eq!(attempts, CAP_ATTEMPTS);

        // One past the cap stays at the cap.
        let attempts = retry_attempts_from_env(|_| Some("120001".to_string()));
        assert_eq!(attempts, CAP_ATTEMPTS);

        // 1h request (which the previous cap allowed) is now also
        // clamped down to the 2-min cap.
        let attempts = retry_attempts_from_env(|_| Some("3600000".to_string()));
        assert_eq!(attempts, CAP_ATTEMPTS);
    }

    /// Issue #206-B regression: a sequence of acquire-drop cycles must
    /// never wedge the in-process slot. The previous AtomicBool +
    /// OnceLock pair could leave the OnceLock populated with a
    /// released fd; the new `Mutex<Option<File>>` must take/replace
    /// the file together so this round-trip stays clean.
    #[test]
    fn round_trip_acquire_drop_repeats_cleanly() {
        let _serial = serial_guard();
        let dir = std::env::temp_dir().join(format!(
            "proteus-lock-roundtrip-multi-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("state.json");
        for _ in 0..3 {
            let g = acquire_for_state_path(&state).expect("acquire");
            assert!(is_held_in_process());
            drop(g);
            assert!(!is_held_in_process());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Inner-lock-then-outer-drop ordering: dropping the inner (no-op)
    /// guard before the outer must keep the kernel flock held. The
    /// AtomicBool design got this right; the new shape must too.
    #[test]
    fn dropping_inner_does_not_release_outer_lock() {
        let _serial = serial_guard();
        let dir =
            std::env::temp_dir().join(format!("proteus-lock-nested-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("state.json");
        let outer = acquire_for_state_path(&state).expect("outer");
        let inner = acquire_for_state_path(&state).expect("nested");
        drop(inner);
        assert!(is_held_in_process(), "outer lock must still hold");
        drop(outer);
        assert!(!is_held_in_process());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Issue #275: the lock file lands at 0o600 when Proteus creates the
    /// parent dir. GH #354 / #363: a pre-existing operator-supplied dir
    /// is left alone (no chmod) so `--state /tmp/x` cannot brick /tmp.
    /// The lock file itself is still tightened to 0o600 — that file is
    /// always Proteus-owned.
    #[test]
    fn lock_file_lands_at_0600_and_dir_left_alone_when_foreign() {
        let _serial = serial_guard();
        let parent = std::env::temp_dir().join(format!(
            "proteus-lock-mode-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();
        // Pre-create the inner dir at 0o755 to mimic an operator's
        // pre-existing temp dir. With GH #354 / #363 we must NOT chmod
        // it.
        let dir = parent.join("state");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        let state = dir.join("state.json");
        let lock = lock_path_for(&state);
        // Pre-create the lock file world-readable to cover the
        // tighten-existing-file branch (the lock file itself IS
        // Proteus-owned and is always re-chmodded to 0o600).
        std::fs::write(&lock, b"").unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644)).unwrap();

        let g = acquire_for_state_path(&state).expect("acquire");
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            dir_mode, 0o755,
            "operator-supplied lock-dir must NOT be re-chmodded; got 0o{dir_mode:o}"
        );
        let lock_mode = std::fs::metadata(&lock).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            lock_mode, 0o600,
            "lock file must be 0o600, got 0o{lock_mode:o}"
        );
        drop(g);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Stream 5 acceptance test: stress with N concurrent
    /// `acquire_state_lock` callers and assert no thread blocks longer
    /// than the 5 s budget, and that every acquire eventually succeeds
    /// (no `panic = abort` is triggered, no deadlock). C1 / N12.13:
    /// the in-process `HELD` mutex must NOT be held across the kernel-
    /// flock retry sleep; if it were, this stress test would serialize
    /// on the mutex and the wall-clock per-thread runtime would scale
    /// linearly with N rather than staying within the 5 s budget.
    #[test]
    fn stress_concurrent_acquires_stay_within_budget() {
        let _serial = serial_guard();
        let dir = std::env::temp_dir().join(format!(
            "proteus-lock-stress-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = std::sync::Arc::new(dir.join("state.json"));

        const N: usize = 16;
        let mut handles = Vec::with_capacity(N);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(N));
        for _ in 0..N {
            let state = std::sync::Arc::clone(&state);
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                let started = std::time::Instant::now();
                let g = acquire_for_state_path(&state).expect("acquire under stress");
                // Simulate a short critical section so other threads
                // actually contend. The 5 s budget must hold even with
                // a 50 ms hold time × 16 threads.
                std::thread::sleep(std::time::Duration::from_millis(50));
                drop(g);
                started.elapsed()
            }));
        }

        for (i, h) in handles.into_iter().enumerate() {
            let waited = h.join().expect("worker did not panic");
            // Per the stream-5 acceptance contract: no thread blocks
            // longer than the 5 s budget.
            assert!(
                waited < std::time::Duration::from_secs(5),
                "thread {i} waited {waited:?}, exceeding the 5s lock budget"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// GH #354 / #363 regression: a freshly-created lock dir lands at
    /// 0o700 (Proteus owns it). Distinct from the foreign-dir test
    /// above — this is the cold-install path.
    #[test]
    fn lock_dir_chmodded_to_0700_when_proteus_creates_it() {
        let _serial = serial_guard();
        let parent = std::env::temp_dir().join(format!(
            "proteus-lock-fresh-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();
        // Inner dir does NOT exist; acquire_for_state_path must create
        // it and tighten to 0o700.
        let dir = parent.join("fresh-state");
        let state = dir.join("state.json");

        let g = acquire_for_state_path(&state).expect("acquire");
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            dir_mode, 0o700,
            "freshly-created lock dir must be 0o700, got 0o{dir_mode:o}"
        );
        drop(g);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Security audit N-3: a symlink planted at the lock-file path must
    /// cause the open to fail with `ELOOP` rather than be followed. We
    /// pre-create the lock path as a symlink to a sibling file and
    /// verify `acquire_inner` returns an IO error instead of locking
    /// the symlink target.
    #[test]
    fn open_refuses_to_follow_symlink_at_lock_path() {
        let _serial = serial_guard();
        let parent = std::env::temp_dir().join(format!(
            "proteus-lock-nofollow-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();
        let dir = parent.join("state");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let state = dir.join("state.json");
        let lock = lock_path_for(&state);
        let target = parent.join("attacker-target");
        std::fs::write(&target, b"steered").unwrap();
        // Replace the (potential) lock file with a symlink to the
        // attacker-controlled target; without `O_NOFOLLOW` the open
        // would land on `target` and Proteus would happily flock it.
        let _ = std::fs::remove_file(&lock);
        std::os::unix::fs::symlink(&target, &lock).unwrap();
        let result = acquire_inner(&lock);
        assert!(
            matches!(result, Err(LockError::Other(_)) | Err(LockError::Io(_))),
            "expected open() to fail with ELOOP; got {result:?}"
        );
        // And the target file must NOT have been touched (still the
        // sentinel content we wrote).
        let target_bytes = std::fs::read(&target).unwrap();
        assert_eq!(
            target_bytes, b"steered",
            "open should not have followed the symlink"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }
}
