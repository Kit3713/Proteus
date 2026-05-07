// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub original_macs: BTreeMap<String, String>,
    pub original_hostname: Option<String>,
    pub captured_by_version: Option<String>,
    pub captured_at: Option<String>,
}

impl State {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(e).with_context(|| format!("reading state file {}", path.display()));
            }
        };
        let state: State = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing state file {}", path.display()))?;
        Ok(Some(state))
    }

    // Atomic write: temp file + rename. Used by phase B+ apply/rotate.
    #[allow(dead_code)]
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("state path has no parent directory")?;
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self)?;
        {
            let mut f =
                fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        Ok(())
    }
}
