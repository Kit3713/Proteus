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
//! that pattern we keep a process-wide `AtomicBool`: the first call acquires
//! the real flock, nested calls return a no-op guard, and the outermost
//! `Drop` releases the lock and clears the flag. Different *processes* still
//! contend on the kernel-level flock the way the design intends.

use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Tracks whether the current process already owns the flock. This is *not*
/// the lock — it's how we make in-process re-entry safe so the orchestrator
/// can call the per-feature apply helpers directly without deadlocking on
/// the per-fd flock semantics on Linux.
static HELD: AtomicBool = AtomicBool::new(false);

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

/// RAII guard. Holds the fd that owns the flock; `Drop` releases it.
///
/// `inner` is `Some` only on the *outermost* acquire in this process. Nested
/// callers receive a guard with `inner = None` so dropping them is a no-op
/// and the real lock survives until the outer scope exits.
pub struct StateLockGuard {
    inner: Option<File>,
}

impl Drop for StateLockGuard {
    fn drop(&mut self) {
        if let Some(file) = self.inner.take() {
            // Best-effort explicit unlock so the lock is released even on
            // exotic platforms where close() takes its time. close() will
            // also drop the lock; we just hint to the kernel sooner.
            unsafe {
                libc::flock(file.as_raw_fd(), libc::LOCK_UN);
            }
            HELD.store(false, Ordering::SeqCst);
            // `file` is dropped here, closing the fd.
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
    if HELD
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        // Already held by an outer scope in this same process; the kernel
        // flock would deadlock if we re-acquired, so hand back a no-op guard.
        return Ok(StateLockGuard { inner: None });
    }
    let path = lock_path_for(state_path);
    match acquire_inner(&path) {
        Ok(file) => Ok(StateLockGuard { inner: Some(file) }),
        Err(e) => {
            // Roll back the HELD flag so a later retry can try again.
            HELD.store(false, Ordering::SeqCst);
            Err(e)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// All tests in this module mutate the process-wide `HELD` flag (or the
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
        }
        // After drop, HELD is clear and a fresh acquire works.
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
        assert!(HELD.load(Ordering::SeqCst));
        {
            let inner = acquire_for_state_path(&state).expect("nested acquire");
            // Inner guard should be a no-op (no fd owned).
            assert!(inner.inner.is_none());
            drop(inner);
            // Outer lock must still be held.
            assert!(HELD.load(Ordering::SeqCst));
        }
        drop(outer);
        assert!(!HELD.load(Ordering::SeqCst));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn busy_when_external_process_holds_lock() {
        let _serial = serial_guard();
        // Simulate the cross-process race: open the lock file in a separate
        // fd, take the flock, then try to acquire in this process. The
        // in-process HELD flag is per-process, so we have to reach the
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
}
