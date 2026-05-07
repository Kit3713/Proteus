// SPDX-License-Identifier: GPL-3.0-or-later

//! Top-level `Command` enum. One variant per `proteus <subcommand>`.

use clap::Subcommand;

use super::actions::{
    BluetoothAction, ConfigAction, DhcpAction, DnsAction, EnterpriseWifiAction, HostnameAction,
    Ipv6Action, KillAction, NftAction, PortalAction, RfAction, StackAction, TimerAction,
    WikiAction,
};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show overall system + per-feature status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Show the current network session at a glance (read-only).
    Session {
        #[arg(long)]
        json: bool,
    },
    /// List current MAC addresses (per interface).
    Current {
        #[arg(long)]
        json: bool,
        /// Limit to a single interface.
        #[arg(long)]
        iface: Option<String>,
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
    Rotate {
        #[arg(long)]
        iface: Option<String>,
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
}
