// SPDX-License-Identifier: GPL-3.0-or-later

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Phase indicator surfaced in stub messages so users know when an unimplemented
// command is expected to land. Bumped at the start of each phase. Phase G is
// the post-build-out maintenance phase per `docs/ROADMAP.md`; the
// roadmap-version cycles (M2 personas, M3 per-SSID, M4 events, M5 ARM, M6
// completions) all run on top of it.
pub const PHASE: char = 'G';
