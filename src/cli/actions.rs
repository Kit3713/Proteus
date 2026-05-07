// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-subcommand action enums.
//!
//! Grouped here so `command.rs` only contains the top-level dispatch shape;
//! each `<Foo>Action` lives next to its peers.

use clap::{Args, Subcommand};

use super::WIKI_SEARCH_DEFAULT_LIMIT;

#[derive(Subcommand, Debug)]
pub enum TimerAction {
    /// Show all proteus-* timers, their state, and current cadence.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// List the timer types Proteus defines.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Enable + start a timer (e.g. `rotate`, `check`).
    Enable(TimerNameArgs),
    /// Disable + stop a timer.
    Disable(TimerNameArgs),
    /// Change a timer's cadence (writes a drop-in).
    Set {
        /// Timer name (`rotate`, `check`, ...).
        name: String,
        /// Cadence: `30s`, `5m`, `2h`, `1d`, or `hourly` / `daily`.
        #[arg(long)]
        interval: String,
    },
    /// Reset a timer's cadence back to its default (removes the drop-in).
    Reset(TimerNameArgs),
    /// Tail recent journald logs for a timer's unit.
    Logs {
        /// Timer name (`rotate`, `check`, ...).
        name: String,
        /// How many lines to tail.
        #[arg(long, default_value_t = 50)]
        lines: u32,
    },
}

#[derive(Args, Debug)]
pub struct TimerNameArgs {
    /// Timer name (`rotate`, `check`, `resume`, `boot`).
    pub name: String,
}

#[derive(Subcommand, Debug)]
pub enum KillAction {
    /// Show whether the kill switch is currently active.
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum NftAction {
    /// Show whether our nft table is installed plus the rendered ruleset.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Install or refresh the Proteus nft table (idempotent).
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Remove the Proteus nft table.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum HostnameAction {
    /// Show current kernel/pretty/transient + Proteus mode + cached originals.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Pick a new hostname per `[hostname] mode` and apply it.
    Rotate {
        #[arg(long)]
        yes: bool,
    },
    /// Pin to a specific hostname (validated against RFC 1123).
    Pin {
        /// Hostname to apply. Must be lowercase [a-z0-9-], no leading/trailing hyphen.
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Restore the cached original hostname.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum PortalAction {
    /// Show current portal classification (clear / portal-required / portal-authed / unknown).
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Add an SSID to the known-portal list.
    Mark {
        ssid: String,
        #[arg(long)]
        yes: bool,
    },
    /// Remove an SSID from the known-portal list.
    Unmark {
        ssid: String,
        #[arg(long)]
        yes: bool,
    },
    /// List known-portal SSIDs.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Open the captive portal page in the default browser.
    Open {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum Ipv6Action {
    /// Show current per-iface IPv6 settings + privacy mode.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply stable-privacy + temp + DUID per config.
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Restore the cached pre-Proteus IPv6 sysctl values.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum EnterpriseWifiAction {
    /// Show 802-1x.anonymous-identity for every 802.1X connection NM knows.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Set 802-1x.anonymous-identity = anonymous@<realm> on a connection.
    Enable {
        /// NM connection profile id (the human-friendly name).
        #[arg(long)]
        connection: String,
        #[arg(long)]
        yes: bool,
    },
    /// Clear 802-1x.anonymous-identity on a connection.
    Disable {
        /// NM connection profile id (the human-friendly name).
        #[arg(long)]
        connection: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum DhcpAction {
    /// Show DHCP suppression state per NM connection.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply DHCP suppression to all managed NM connections.
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Restore NM defaults on all proteus-managed connections.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum WikiAction {
    /// Full-text search across the embedded wiki.
    Search {
        /// One or more query terms (space-separated; case-insensitive).
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(long)]
        json: bool,
        /// Cap on result rows shown (default 10).
        #[arg(long, default_value_t = WIKI_SEARCH_DEFAULT_LIMIT)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum BluetoothAction {
    /// List adapters with current alias, discoverable state, and RPA status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply alias / discoverable / BLE RPA policy to all adapters.
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Restore original adapter aliases from cache.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Print the active config (alias for `proteus show-config`).
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Print a single config value (e.g. `mac.enabled`).
    Get {
        /// Dotted key, e.g. `mac.rotation_interval`.
        key: String,
        #[arg(long)]
        json: bool,
    },
    /// Set a single config value. Requires root + --yes.
    Set {
        /// Dotted key, e.g. `mac.rotation_interval`.
        key: String,
        /// New value (string, integer, bool — coerced to the existing type).
        value: String,
        #[arg(long)]
        yes: bool,
    },
    /// Enable a component (shorthand for `set <component>.enabled true`).
    Enable {
        /// Section name, e.g. `mac`, `hostname`.
        component: String,
        #[arg(long)]
        yes: bool,
    },
    /// Disable a component, optionally recording a reason as a comment.
    Disable {
        /// Section name, e.g. `dns`.
        component: String,
        /// Free-form reason; written above the section as a `# Proteus: disabled` comment.
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Open $EDITOR on /etc/proteus/config.toml; validate on save.
    Edit,
    /// Parse the current config; report errors with file context.
    Validate {
        #[arg(long)]
        json: bool,
    },
    /// Reset a section (or the whole file) to built-in defaults. Requires --yes.
    Reset {
        /// Optional section to reset; omit to reset everything.
        section: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// List every supported config key with its type and default.
    Keys {
        #[arg(long)]
        json: bool,
    },
    /// Set the active profile (off / min / low / med / high / agr).
    /// Per-knob overrides already in the config file are preserved.
    SetProfile {
        /// Profile name: `off`, `min`, `low`, `med`, `high`, or `agr`.
        profile: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum StackAction {
    /// Show current sysctl values + the drop-in we'd apply.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Write the drop-in to /etc/sysctl.d/95-proteus.conf and reload.
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Remove the drop-in and reload defaults.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum DnsAction {
    /// Show what is applied or what we deferred to and why.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply the ECS-strip drop-in (no-op if hard guard trips).
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Remove the ECS-strip drop-in.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}
