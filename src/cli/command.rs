// SPDX-License-Identifier: GPL-3.0-or-later

//! Top-level `Command` enum. One variant per `proteus <subcommand>`.

use clap::Subcommand;

use super::actions::{
    BluetoothAction, ConfigAction, DhcpAction, DnsAction, EnterpriseWifiAction, EventsAction,
    HostnameAction, Ipv6Action, KillAction, NftAction, NtpAction, PersonaAction, PortalAction,
    ResolvedAction, RfAction, SsidAction, StackAction, TimerAction, WikiAction,
};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show overall system + per-feature status.
    #[command(alias = "s")]
    Status {
        #[arg(long)]
        json: bool,
        /// Re-run on a fixed interval, clearing the screen between renders.
        /// Roadmap Milestone 6 (CLI ergonomics). Pair with `--interval`
        /// (default 2s); accepts `2s`, `500ms`, `1m`.
        #[arg(long)]
        watch: bool,
        /// Refresh cadence for `--watch`. Ignored without `--watch`.
        #[arg(long, default_value = "2s")]
        interval: String,
    },
    /// Show the current network session at a glance (read-only).
    Session {
        #[arg(long)]
        json: bool,
        /// Re-run on a fixed interval. See `proteus status --watch`.
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value = "2s")]
        interval: String,
    },
    /// List current MAC addresses (per interface).
    Current {
        #[arg(long)]
        json: bool,
        /// Limit to a single interface.
        #[arg(long)]
        iface: Option<String>,
        /// Re-run on a fixed interval. See `proteus status --watch`.
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value = "2s")]
        interval: String,
    },
    /// Show the cached original MACs and hostname.
    Original {
        #[arg(long)]
        json: bool,
    },
    /// Print the active config file (or note that defaults are in use).
    ShowConfig {
        #[arg(long)]
        json: bool,
    },
    /// Print the built-in default config.
    ShowDefaults {
        #[arg(long)]
        json: bool,
    },
    /// Apply Proteus config to the system.
    #[command(alias = "a")]
    Apply {
        #[arg(long)]
        yes: bool,
    },
    /// Revert Proteus changes to the cached originals.
    Revert {
        #[arg(long)]
        yes: bool,
    },
    /// Rotate MAC for one or all managed interfaces.
    #[command(alias = "r")]
    Rotate {
        #[arg(long)]
        iface: Option<String>,
        #[arg(long)]
        yes: bool,
        /// Print every candidate considered + the reason it was rejected
        /// (collision, forbidden, gateway, avoid). Roadmap M2.
        #[arg(long)]
        explain: bool,
    },
    /// Rotate the MAC iff the cooldown window has elapsed
    /// (Roadmap Milestone 1, issue #206-C). Designed to be called by
    /// the NetworkManager dispatcher so it stops sed-parsing
    /// `proteus current --json`. Returns a typed
    /// [`crate::backend::RotateOutcome`] as a single stdout line plus
    /// a deterministic exit code (`0` for rotated/skipped/no-factory,
    /// `70` for backend-unavailable).
    #[command(name = "rotate-if-needed")]
    RotateIfNeeded {
        /// Interface name. The NM dispatcher always passes one; the
        /// CLI defaults to the first managed wifi/ethernet.
        #[arg(long)]
        iface: Option<String>,
        /// Cooldown budget in seconds.
        #[arg(long, default_value_t = 60)]
        cooldown: u64,
        #[arg(long)]
        yes: bool,
    },
    /// Pin an interface or NM connection to a specific MAC.
    Pin {
        /// Interface name or NM connection profile.
        target: String,
        /// Specific MAC to pin (defaults to current cloned MAC).
        #[arg(long)]
        mac: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Remove a pin previously set with `pin`.
    Unpin {
        /// Interface name or NM connection profile.
        target: String,
    },
    /// Show diff between config, defaults, and live state.
    Diff {
        #[arg(long)]
        json: bool,
    },
    /// Preview what a mutating command would do.
    DryRun {
        /// The command (and args) to preview.
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },
    /// Reset config to built-in defaults (sacred originals untouched).
    Reset {
        #[arg(long)]
        yes: bool,
        /// Print what would happen without writing.
        #[arg(long = "dry-run")]
        dry_run: bool,
    },
    /// Remove Proteus from the system.
    Uninstall {
        /// Also remove /etc/proteus and /var/lib/proteus.
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Bluetooth alias / discoverable / BLE RPA management.
    Bluetooth {
        #[command(subcommand)]
        action: BluetoothAction,
    },
    /// Hostname (kernel/pretty/transient) management via systemd hostnamed.
    Hostname {
        #[command(subcommand)]
        action: HostnameAction,
    },
    /// IPv6 stable-privacy + temp addresses + DUID rotation.
    Ipv6 {
        #[command(subcommand)]
        action: Ipv6Action,
    },
    /// 802.1X enterprise Wi-Fi anonymous outer identity (opt-in).
    EnterpriseWifi {
        #[command(subcommand)]
        action: EnterpriseWifiAction,
    },
    /// Stack-fingerprint sysctl drop-in (TCP/ICMP/NDP hardening).
    Stack {
        #[command(subcommand)]
        action: StackAction,
    },
    /// DNS ECS-strip drop-in on systemd-resolved (one knob, hard guard).
    Dns {
        #[command(subcommand)]
        action: DnsAction,
    },
    /// systemd-resolved mDNS + LLMNR off drop-in (Milestone 4a).
    Resolved {
        #[command(subcommand)]
        action: ResolvedAction,
    },
    /// systemd-timesyncd NTP normalisation drop-in (Milestone 4a).
    Ntp {
        #[command(subcommand)]
        action: NtpAction,
    },
    /// DHCP option suppression (12/60/61/81 + DHCPv6 DUID/IAID).
    Dhcp {
        #[command(subcommand)]
        action: DhcpAction,
    },
    /// Browse the embedded wiki (or search it with `wiki search <query>`).
    #[command(args_conflicts_with_subcommands = true)]
    Wiki {
        #[command(subcommand)]
        action: Option<WikiAction>,
        /// Page name (e.g. `intro`); omit to list pages.
        page: Option<String>,
    },
    /// Show help for a feature (alias for `wiki <feature>` with friendly fallback).
    Help {
        /// Feature or wiki page name.
        feature: Option<String>,
    },
    /// Manage Proteus systemd timers (status, enable, set cadence, etc.).
    Timer {
        #[command(subcommand)]
        action: TimerAction,
    },
    /// Manage Proteus configuration without hand-editing config.toml.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Run a battery of self-diagnostic checks (read-only).
    Doctor {
        /// Emit JSON instead of human-readable output.
        #[arg(long)]
        json: bool,
        /// Skip the slower checks (DBus probes, filesystem walks).
        #[arg(long)]
        quick: bool,
    },
    /// Run a manual probe round against the configured endpoints.
    Probe {
        /// Machine-readable JSON output.
        #[arg(long)]
        json: bool,
        /// Single endpoint, fast.
        #[arg(long)]
        quick: bool,
    },
    /// Emergency network kill switch — bring all interfaces down + radios off.
    Kill {
        #[command(subcommand)]
        action: Option<KillAction>,
        /// Confirm the destructive action when omitting the subcommand.
        #[arg(long, global = true)]
        yes: bool,
    },
    /// Restore network connectivity after `proteus kill`.
    Resume {
        #[arg(long)]
        yes: bool,
    },
    /// Manage the Proteus nftables table (ICMP info-drops + optional discovery blocks).
    Nft {
        #[command(subcommand)]
        action: NftAction,
    },
    /// Captive portal detection + known-portal SSID list.
    Portal {
        #[command(subcommand)]
        action: PortalAction,
    },
    /// RF surface — Wi-Fi chipset inventory + opt-in TX-power reduction.
    Rf {
        #[command(subcommand)]
        action: RfAction,
    },
    /// Device-persona / randomizer-recipe management (roadmap Milestone 2).
    Persona {
        #[command(subcommand)]
        action: PersonaAction,
    },
    /// Per-SSID profile policies (roadmap Milestone 3).
    Ssid {
        #[command(subcommand)]
        action: SsidAction,
    },
    /// Event-driven rotation framework (roadmap Milestone 4c).
    ///
    /// The long-lived `events run` subcommand subscribes to four
    /// reactive trigger sources (NM connection-up, link-flap,
    /// regulatory-domain change, captive-portal auth) and routes
    /// detected events through an in-process `EventRegistry`. The
    /// default handler invokes the same rotation entry point as
    /// `proteus rotate`; the daemon is opt-in via `[events] enabled
    /// = true` plus the `proteus-events.service` systemd unit.
    Events {
        #[command(subcommand)]
        action: EventsAction,
    },
    /// Print the embedded shell-completion script for this binary's CLI.
    ///
    /// Roadmap Milestone 6: ergonomics. The completion files are hand-written
    /// and live under `dist/completions/`; this subcommand prints them on
    /// stdout so users can install without sudo:
    ///
    /// ```sh
    /// proteus completions bash > ~/.local/share/bash-completion/completions/proteus
    /// proteus completions zsh  > "$(brew --prefix)/share/zsh/site-functions/_proteus"
    /// proteus completions fish > ~/.config/fish/completions/proteus.fish
    /// ```
    Completions {
        /// Shell to print completions for: bash, zsh, or fish.
        shell: String,
    },
}
