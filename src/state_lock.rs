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
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use thiserror::Error;

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
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(100);

/// Resolve the retry budget from `PROTEUS_LOCK_TIMEOUT_MS` (falling back
/// to [`DEFAULT_LOCK_BUDGET_MS`]) and convert it to a retry-attempt count.
/// Pulled out so tests can exercise the parser without spawning a process.
fn retry_attempts_from_env(get: impl Fn(&str) -> Option<String>) -> u32 {
    let budget_ms = get("PROTEUS_LOCK_TIMEOUT_MS")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_LOCK_BUDGET_MS);
    let granularity_ms = RETRY_DELAY.as_millis() as u64;
    let attempts = (budget_ms.max(granularity_ms) + granularity_ms - 1) / granularity_ms;
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
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if held.is_some() {
        // Already held by an outer scope in this same process; the kernel
        // flock would deadlock if we re-acquired, so hand back a no-op guard.
        return Ok(StateLockGuard { outermost: false });
    }
    let path = lock_path_for(state_path);
    match acquire_inner(&path) {
        Ok(file) => {
            *held = Some(file);
            Ok(StateLockGuard { outermost: true })
        }
        Err(e) => Err(e),
    }
}

fn lock_path_for(state_path: &Path) -> PathBuf {
    let dir = state_path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(LOCK_FILE_NAME)
}

fn acquire_inner(path: &Path) -> Result<File, LockError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating lock dir {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("opening lock file {}", path.display()))?;

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
        let dir = std::env::temp_dir().join(format!(
            "proteus-lock-nested-drop-{}",
            std::process::id()
        ));
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
}
