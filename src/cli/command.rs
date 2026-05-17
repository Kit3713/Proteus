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
        /// Issue #343: emit a single-line JSON summary
        /// (`{ command, components: [...], exit_code }`) instead of the
        /// human-readable per-component lines. CI / Ansible consumers
        /// can grep `.exit_code` and inspect `.components[].status`
        /// without parsing the rendered table.
        #[arg(long)]
        json: bool,
    },
    /// Revert Proteus changes to the cached originals.
    Revert {
        #[arg(long)]
        yes: bool,
        /// Issue #343: emit a single-line JSON summary
        /// (`{ command, components: [...], exit_code }`) instead of
        /// the per-step removal lines + trailing warnings. Same
        /// envelope as `apply --json`.
        #[arg(long)]
        json: bool,
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
        /// Emit a `{"results": [ ... ]}` JSON envelope summarising the
        /// rotation (issue #395). One entry per interface touched, each
        /// carrying `iface`, `old_mac`, `new_mac`, `outcome`, and
        /// (under `--explain`) the candidate trace. Mirrors the
        /// `--json` shape on other readers so dispatchers can stop
        /// screen-scraping the human-readable lines.
        #[arg(long)]
        json: bool,
    },
    /// Rotate the MAC iff the cooldown window has elapsed.
    ///
    /// Designed to be called by the NetworkManager dispatcher so it
    /// stops sed-parsing `proteus current --json`. Returns a typed
    /// [`crate::backend::RotateOutcome`] as a single stdout line plus
    /// a deterministic exit code (`0` for rotated/skipped/no-factory,
    /// `70` for backend-unavailable).
    #[command(name = "rotate-if-needed")]
    RotateIfNeeded {
        /// Interface name. The NM dispatcher always passes one; the
        /// CLI defaults to the first managed wifi/ethernet.
        #[arg(long)]
        iface: Option<String>,
        /// Cooldown budget in seconds (0 = always rotate; 0..=86400).
        // N12.12: bound to 0..=86_400 because 0 is a legitimate
        // "always rotate" shape (the dispatcher's hot path may pass
        // it for forced rotates) and 86_400s = 1 day is the realistic
        // upper bound for a per-SSID stickiness window.
        #[arg(
            long,
            default_value_t = 60,
            value_parser = clap::value_parser!(u64).range(0..=86_400),
        )]
        cooldown: u64,
        /// SSID being joined, when known. Roadmap Milestone 3: the
        /// dispatcher passes this so per-SSID policies (`pin_mac`,
        /// `rotate_interval`) can short-circuit or extend the rotate.
        /// The plain CLI shape (no SSID) keeps the existing behaviour.
        #[arg(long)]
        ssid: Option<String>,
        #[arg(long)]
        yes: bool,
        /// Issue #378: print the policy + cooldown math the dispatcher
        /// hot path is making (effective cooldown, per-SSID pin/interval
        /// overrides, the configured backend driver) before invoking
        /// the backend. Triage tool — does NOT mutate; pairs with
        /// `--cooldown` to surface the actual effective budget.
        #[arg(long)]
        explain: bool,
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
    ///
    /// Issue #392: bulk-clear flags `--all` and `--scope <type>` mirror
    /// the symmetric shape of `pin`. Exactly one of `target`, `--all`,
    /// or `--scope` must be supplied; clap rejects every other combo
    /// at parse time. Both bulk modes require `--yes` because they
    /// rewrite the pin registry wholesale.
    Unpin {
        /// Interface name or NM connection profile. Omit when using
        /// `--all` or `--scope`.
        #[arg(
            required_unless_present_any = ["all", "scope"],
            conflicts_with_all = ["all", "scope"],
        )]
        target: Option<String>,
        /// Remove every pin in the registry (requires `--yes`).
        #[arg(long, conflicts_with = "scope")]
        all: bool,
        /// Remove every pin of the given scope: `iface` or
        /// `nm-connection` (requires `--yes`).
        #[arg(long, value_name = "TYPE")]
        scope: Option<String>,
        /// Confirm this mutating change (issue #391 / N12.1):
        /// `unpin` clears the persisted pin so the next rotation
        /// drops the operator-chosen MAC. Without `--yes` the
        /// command exits with `CONFIRMATION_REQUIRED`.
        #[arg(long)]
        yes: bool,
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
        /// CL6: emit machine-readable JSON instead of the rendered
        /// markdown / TOC. With a page name the payload is
        /// `{ "page": ..., "content": ... }`; without one it's
        /// `{ "pages": [...] }`.
        #[arg(long)]
        json: bool,
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
    /// Tail journald across every Proteus systemd unit + the NM dispatcher.
    ///
    /// Composes `journalctl -u <unit> ... -t proteus-dispatcher` so a
    /// single command surfaces every line Proteus emits (boot, rotate,
    /// check, resume, events service, and the dispatcher script). Defaults
    /// to printing the last 50 lines and exiting; pair with `--follow` for
    /// a live tail.
    Logs {
        /// Tail-follow (don't exit after the initial batch).
        #[arg(long, short = 'f')]
        follow: bool,
        /// How many lines to tail (1..=100000). Same bound as `timer logs`.
        #[arg(
            long,
            short = 'n',
            default_value_t = 50,
            value_parser = clap::value_parser!(u32).range(1..=100_000),
        )]
        lines: u32,
        /// Passthrough to `journalctl --since` (e.g. `1h ago`, `09:00`,
        /// `2025-05-17`).
        #[arg(long)]
        since: Option<String>,
        /// Emit structured journal entries (`journalctl --output=json`).
        #[arg(long)]
        json: bool,
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
        /// Emit a single-line JSON summary (`{ "resumed": [...], "warnings": [...] }`)
        /// instead of the human-readable per-iface lines. CL6: parity with
        /// `kill --status --json` so wrappers don't have to grep the
        /// human output.
        #[arg(long)]
        json: bool,
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
    /// Print build provenance (package version, git sha, rustc, target,
    /// build time, state schema version).
    ///
    /// Issue #376: CI and GUI wrappers need a stable, machine-readable
    /// shape for "which proteus is this?". Pass `--json` for the wrapper
    /// surface; the bare form is human-readable. `proteus about` is an
    /// alias for the bare form.
    Version {
        /// Emit JSON instead of human-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Friendly alias for `proteus version` (human-readable).
    About,
}
