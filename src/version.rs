// SPDX-License-Identifier: GPL-3.0-or-later

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Phase indicator surfaced in stub messages so users know when an unimplemented
// command is expected to land. Bumped at the start of each phase.
pub const PHASE: char = 'A';
