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
    /// Release + renew the DHCP lease without changing the MAC.
    ///
    /// Roadmap Milestone 4c: rotates the IP from the upstream DHCP
    /// server's perspective so the L3 identity changes while the L2
    /// cover (persona MAC) stays stable. Useful when the operator
    /// wants a fresh lease but doesn't want to disturb the rest of
    /// the connection state (Wi-Fi association, 802.1X auth, etc.).
    Renew {
        /// Limit to a single interface; default is every managed
        /// wifi/ethernet device.
        #[arg(long)]
        iface: Option<String>,
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

/// `proteus resolved ...` — Milestone 4a: mDNS+LLMNR off via systemd-resolved
/// drop-in. Sibling to `proteus dns` so reverting one doesn't disturb the
/// other.
#[derive(Subcommand, Debug)]
pub enum ResolvedAction {
    /// Show what is applied or what we deferred to and why.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply the mDNS+LLMNR drop-in (no-op if hard guard trips).
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Remove the mDNS+LLMNR drop-in.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

/// `proteus ntp ...` — Milestone 4a: timesyncd NTP normalisation. Skipped if
/// `chronyd` or `ntpd` is on the system; both have their own config layers.
#[derive(Subcommand, Debug)]
pub enum NtpAction {
    /// Show what is applied or what we deferred to and why.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply the timesyncd drop-in (no-op if hard guard trips).
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Remove the timesyncd drop-in.
    Revert {
        #[arg(long)]
        yes: bool,
    },
}

/// `proteus persona ...` — roadmap Milestone 2.
///
/// The schema, catalogue, loader, and CLI all land in this PR. The
/// apply / rotate integration (MAC OUI shaping, hostname template
/// rendering, DHCP fingerprint write) is the follow-up tracked in
/// the roadmap "Integration" bullets.
#[derive(Subcommand, Debug)]
pub enum PersonaAction {
    /// List available personas (built-in + user). Filterable by kind / category.
    List {
        /// `stealth` or `randomizer`.
        #[arg(long)]
        kind: Option<String>,
        /// `phone`, `laptop`, `tv`, `iot`, `router`, `console`, `printer`, `generic`.
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Print the full schema for a single persona.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Set `[persona] active = <id>` in config. `--apply` runs `proteus apply` after.
    Use {
        id: String,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Drop back to plain randomizer mode (`active = None`).
    Clear {
        #[arg(long)]
        yes: bool,
    },
    /// Show the active persona id and which fields it would shape.
    Current {
        #[arg(long)]
        json: bool,
    },
    /// Pick a random persona id (filterable). Does NOT auto-apply.
    Random {
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Clone an existing persona to `/etc/proteus/personas/<new>.toml`.
    New {
        /// New id (kebab-case).
        id: String,
        /// Existing persona to copy from.
        #[arg(long = "from")]
        from: String,
        #[arg(long)]
        yes: bool,
    },
    /// `$EDITOR` on `/etc/proteus/personas/<id>.toml`.
    Edit {
        id: String,
    },
    /// Schema-check an arbitrary `.toml` file. Exit 0 / 1.
    Validate {
        path: std::path::PathBuf,
    },
    /// Copy `<path>` into `/etc/proteus/personas/`.
    Import {
        path: std::path::PathBuf,
        #[arg(long)]
        yes: bool,
    },
    /// Copy persona `<id>` to `<path>`.
    Export {
        id: String,
        path: std::path::PathBuf,
    },
}

/// `proteus ssid ...` — roadmap Milestone 3.
///
/// Read commands (`list`, `show`) work for any user. Mutating commands
/// (`set`, `clear`) require root because they write under
/// `/etc/proteus/`.
#[derive(Subcommand, Debug)]
pub enum SsidAction {
    /// List every per-SSID entry. With `--json`, emit the raw `[per_ssid]`
    /// table; without it, one line per SSID with the fields that are set.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show the resolved policy for one SSID, including the source trace
    /// (`per_ssid > persona > profile > defaults`).
    Show {
        /// SSID to resolve. Case-sensitive, matches `Config::per_ssid` keys.
        ssid: String,
        #[arg(long)]
        json: bool,
    },
    /// Set one field on a per-SSID block. Creates the block if absent.
    /// Keys: `persona`, `aggressiveness_profile`, `pin_mac`,
    /// `rotate_interval`, `portal_policy`.
    Set {
        ssid: String,
        /// Field name (one of the five known keys).
        key: String,
        /// New value. Validation happens in the resolver, not the writer
        /// — invalid values surface in `proteus ssid show` with a
        /// fall-through trace.
        value: String,
        #[arg(long)]
        yes: bool,
    },
    /// Drop the entire `[per_ssid."<ssid>"]` block.
    Clear {
        ssid: String,
        #[arg(long)]
        yes: bool,
    },
}

/// `proteus events ...` — Milestone 4c rotation-trigger daemon.
///
/// `run` starts the long-lived process: builds an `EventRegistry`,
/// registers a default rotation handler, and spawns every available
/// event source. The systemd unit (`dist/systemd/proteus-events.service`)
/// is the production entry point; `proteus events run` from the
/// shell is for development + the smoke-test path.
#[derive(Subcommand, Debug)]
pub enum EventsAction {
    /// Run the long-lived event daemon. Reads `[events]` from
    /// config; refuses to start when `enabled = false` unless
    /// `--force` is passed.
    Run {
        /// Run the loop even when `[events] enabled = false`. Useful
        /// for one-off smoke tests; the systemd unit never sets this.
        #[arg(long)]
        force: bool,
        /// Exit after `n` triggers (or after `--once-after-secs`,
        /// whichever comes first). `0` (the default) means run
        /// forever — the production shape for the systemd unit.
        #[arg(long, default_value_t = 0)]
        max_triggers: u64,
        /// Stop the daemon after the given number of seconds. `0`
        /// (the default) means run forever. The smoke-test path
        /// pairs this with `--max-triggers` so a CI run terminates
        /// even when no triggers fire.
        #[arg(long, default_value_t = 0)]
        once_after_secs: u64,
    },
}

#[derive(Subcommand, Debug)]
pub enum RfAction {
    /// Show Wi-Fi/Bluetooth chipset inventory + current TX-power per iface.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply the configured TX-power reduction to every Wi-Fi interface.
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Restore the cached pre-Proteus TX power per interface.
    Revert {
        #[arg(long)]
        yes: bool,
    },
    /// Report scan policy (active vs passive) and randomization capability
    /// per Wi-Fi iface — roadmap Milestone 4b.
    Scan {
        #[arg(long)]
        json: bool,
    },
    /// Firmware/driver inventory per Wi-Fi iface and Bluetooth adapter
    /// — roadmap Milestone 4b.
    Chipset {
        #[arg(long)]
        json: bool,
    },
}
