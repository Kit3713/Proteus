// SPDX-License-Identifier: GPL-3.0-or-later

//! Test-only helpers shared across `#[cfg(test)]` modules. Visible via
//! `crate::testing::*` only when building tests; never compiled into the
//! release binary.

use std::path::PathBuf;

/// Tiny tempdir helper. Avoids the `tempfile` crate dep per project policy
/// (see `dns/mod.rs` for the original pattern). Uses `getrandom` for the
/// suffix so two concurrent test runs cannot collide. The directory is
/// removed when the value is dropped.
pub struct TempRoot {
    pub path: PathBuf,
}

impl TempRoot {
    pub fn new(label: &str) -> Self {
        let mut buf = [0u8; 8];
        getrandom::getrandom(&mut buf).unwrap();
        let suffix: String = buf.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("proteus-{label}-test-{suffix}"));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
