# proteus fish completion
# Hand-written for stability and small size. Re-source-able.
#
# Install:
#   - Per-user:    cp to ~/.config/fish/completions/proteus.fish
#   - System-wide: cp to /usr/share/fish/vendor_completions.d/proteus.fish
#
# install.sh installs this to /usr/share/fish/vendor_completions.d/proteus.fish.

set -l __proteus_subs apply bluetooth completions config current dhcp diff dns \
    doctor dry-run enterprise-wifi events help hostname ipv6 kill nft ntp \
    original persona pin portal probe reset resolved resume revert rf rotate \
    rotate-if-needed session show-config show-defaults ssid stack status timer \
    uninstall unpin wiki

# True when no subcommand has been entered yet.
function __proteus_needs_subcommand
    set -l cmd (commandline -opc)
    set -l subs apply bluetooth completions config current dhcp diff dns \
        doctor dry-run enterprise-wifi events help hostname ipv6 kill nft ntp \
        original persona pin portal probe reset resolved resume revert rf \
        rotate rotate-if-needed session show-config show-defaults ssid stack \
        status timer uninstall unpin wiki
    for word in $cmd[2..]
        if contains -- $word $subs
            return 1
        end
    end
    return 0
end

# Skip virtual @-suffixed entries.
function __proteus_ifaces
    if command -q ip
        ip -o link 2>/dev/null | awk -F': ' '{print $2}' | grep -v '@'
    end
end

function __proteus_pin_targets
    __proteus_ifaces
    if command -q nmcli
        nmcli -g name connection show 2>/dev/null
    end
end

function __proteus_wiki_pages
    printf '%s\n' \
        backend bluetooth captive-portals cli concepts config dhcp discovery \
        distro-support dns doctor enterprise-wifi faq getting-started glossary \
        hostile-environments hostname-recipes internals intro ip-rotation ipv6 \
        journald-network-logs kill-switch mac-recipes network-fingerprint-checklist \
        personas per-ssid probes profiles quickstart real-world-testing recipes \
        reproducible-builds rf-fingerprinting rotation security-checklist \
        stack-fingerprint threat-model throttling-detect timer troubleshooting \
        uninstall verifying wpa-supplicant-hardening
end

# Wipe any prior bindings so this file is re-source-able.
complete -c proteus -e

# Subcommands.
complete -c proteus -n __proteus_needs_subcommand -a apply            -d 'Apply Proteus config to the system'
complete -c proteus -n __proteus_needs_subcommand -a bluetooth        -d 'Bluetooth alias / discoverable / BLE RPA management'
complete -c proteus -n __proteus_needs_subcommand -a completions      -d 'Print embedded shell-completion script'
complete -c proteus -n __proteus_needs_subcommand -a config           -d 'Manage Proteus configuration'
complete -c proteus -n __proteus_needs_subcommand -a current          -d 'List current MAC addresses'
complete -c proteus -n __proteus_needs_subcommand -a dhcp             -d 'DHCP option suppression'
complete -c proteus -n __proteus_needs_subcommand -a diff             -d 'Show diff between config, defaults, and live state'
complete -c proteus -n __proteus_needs_subcommand -a dns              -d 'DNS ECS-strip drop-in on systemd-resolved'
complete -c proteus -n __proteus_needs_subcommand -a doctor           -d 'Run a battery of self-diagnostic checks'
complete -c proteus -n __proteus_needs_subcommand -a dry-run          -d 'Preview what a mutating command would do'
complete -c proteus -n __proteus_needs_subcommand -a enterprise-wifi  -d '802.1X enterprise Wi-Fi anonymous outer identity'
complete -c proteus -n __proteus_needs_subcommand -a events           -d 'Event-driven rotation framework'
complete -c proteus -n __proteus_needs_subcommand -a help             -d 'Show help for a feature'
complete -c proteus -n __proteus_needs_subcommand -a hostname         -d 'Hostname management via systemd hostnamed'
complete -c proteus -n __proteus_needs_subcommand -a ipv6             -d 'IPv6 stable-privacy + temp addresses + DUID rotation'
complete -c proteus -n __proteus_needs_subcommand -a kill             -d 'Emergency network kill switch'
complete -c proteus -n __proteus_needs_subcommand -a nft              -d 'Manage the Proteus nftables table'
complete -c proteus -n __proteus_needs_subcommand -a ntp              -d 'systemd-timesyncd NTP normalisation drop-in'
complete -c proteus -n __proteus_needs_subcommand -a original         -d 'Show the cached original MACs and hostname'
complete -c proteus -n __proteus_needs_subcommand -a persona          -d 'Device-persona / randomizer-recipe management'
complete -c proteus -n __proteus_needs_subcommand -a pin              -d 'Pin an interface or NM connection to a specific MAC'
complete -c proteus -n __proteus_needs_subcommand -a portal           -d 'Captive portal detection + known-portal SSID list'
complete -c proteus -n __proteus_needs_subcommand -a probe            -d 'Run a manual probe round'
complete -c proteus -n __proteus_needs_subcommand -a reset            -d 'Reset config to defaults'
complete -c proteus -n __proteus_needs_subcommand -a resolved         -d 'systemd-resolved mDNS + LLMNR off drop-in'
complete -c proteus -n __proteus_needs_subcommand -a resume           -d 'Restore network connectivity after kill'
complete -c proteus -n __proteus_needs_subcommand -a revert           -d 'Revert Proteus changes to the cached originals'
complete -c proteus -n __proteus_needs_subcommand -a rf               -d 'RF surface (TX-power, scan policy, chipset)'
complete -c proteus -n __proteus_needs_subcommand -a rotate           -d 'Rotate MAC for one or all managed interfaces'
complete -c proteus -n __proteus_needs_subcommand -a rotate-if-needed -d 'Rotate the MAC iff cooldown elapsed'
complete -c proteus -n __proteus_needs_subcommand -a session          -d 'Show the current network session at a glance'
complete -c proteus -n __proteus_needs_subcommand -a show-config      -d 'Print the active config file'
complete -c proteus -n __proteus_needs_subcommand -a show-defaults    -d 'Print the built-in default config'
complete -c proteus -n __proteus_needs_subcommand -a ssid             -d 'Per-SSID profile policies'
complete -c proteus -n __proteus_needs_subcommand -a stack            -d 'Stack-fingerprint sysctl drop-in'
complete -c proteus -n __proteus_needs_subcommand -a status           -d 'Show overall system + per-feature status'
complete -c proteus -n __proteus_needs_subcommand -a timer            -d 'Manage Proteus systemd timers'
complete -c proteus -n __proteus_needs_subcommand -a uninstall        -d 'Remove Proteus from the system'
complete -c proteus -n __proteus_needs_subcommand -a unpin            -d 'Remove a pin previously set with pin'
complete -c proteus -n __proteus_needs_subcommand -a wiki             -d 'Browse the embedded wiki'

# Global flags.
complete -c proteus -s v -l verbose  -d 'Increase log verbosity'
complete -c proteus -s q -l quiet    -d 'Decrease log verbosity'
complete -c proteus -l config        -d 'Override config path' -r -F
complete -c proteus -l state         -d 'Override state path'  -r -F
complete -c proteus -l no-color      -d 'Disable colored output'
complete -c proteus -l format        -d 'Output format' -x -a 'table json yaml'
complete -c proteus -s h -l help     -d 'Show help'
complete -c proteus -s V -l version  -d 'Show version'

# Per-subcommand flags. __fish_seen_subcommand_from gates them by context.
complete -c proteus -n '__fish_seen_subcommand_from status session current original show-config show-defaults diff doctor probe' \
    -l json -d 'Emit JSON'
complete -c proteus -n '__fish_seen_subcommand_from apply revert reset rotate rotate-if-needed uninstall resume kill' \
    -l yes -d 'Skip confirmation'
complete -c proteus -n '__fish_seen_subcommand_from status session current' \
    -l watch -d 'Re-run on a fixed interval'
complete -c proteus -n '__fish_seen_subcommand_from status session current' \
    -l interval -d 'Refresh cadence' -x

complete -c proteus -n '__fish_seen_subcommand_from current rotate rotate-if-needed dhcp' \
    -l iface -d 'Limit to a single interface' -x -a '(__proteus_ifaces)'
complete -c proteus -n '__fish_seen_subcommand_from rotate' \
    -l explain -d 'Print every candidate considered'
complete -c proteus -n '__fish_seen_subcommand_from rotate-if-needed' \
    -l cooldown -d 'Cooldown budget in seconds' -x
complete -c proteus -n '__fish_seen_subcommand_from rotate-if-needed' \
    -l ssid -d 'SSID being joined' -x
complete -c proteus -n '__fish_seen_subcommand_from doctor probe' \
    -l quick -d 'Skip slower checks'
complete -c proteus -n '__fish_seen_subcommand_from uninstall' \
    -l purge -d 'Also remove /etc/proteus and /var/lib/proteus'
complete -c proteus -n '__fish_seen_subcommand_from reset' \
    -l dry-run -d 'Print what would happen without writing'
complete -c proteus -n '__fish_seen_subcommand_from pin' \
    -l mac -d 'Specific MAC to pin' -x
complete -c proteus -n '__fish_seen_subcommand_from unpin' \
    -l all -d 'Remove every pin (requires --yes)'
complete -c proteus -n '__fish_seen_subcommand_from unpin' \
    -l scope -d 'Restrict bulk clear to scope' -x -a 'iface nm-connection'

# Action sub-subcommands. True when the parent has been seen but no
# action from its known action list has been entered yet.
function __proteus_action_for
    set -l cmd (commandline -opc)
    set -l parent $argv[1]
    set -l actions $argv[2..]
    set -l seen_parent 0
    for word in $cmd[2..]
        if test $seen_parent -eq 0
            if test "$word" = "$parent"
                set seen_parent 1
            end
        else if contains -- $word $actions
            return 1
        end
    end
    if test $seen_parent -eq 1
        return 0
    end
    return 1
end

# Action surface per parent subcommand. Each line: parent + action list.
complete -c proteus -n '__proteus_action_for bluetooth status apply revert' -f -a 'status apply revert'
complete -c proteus -n '__proteus_action_for hostname status rotate pin revert' -f -a 'status rotate pin revert'
complete -c proteus -n '__proteus_action_for ipv6 status apply revert' -f -a 'status apply revert'
complete -c proteus -n '__proteus_action_for enterprise-wifi status enable disable' -f -a 'status enable disable'
complete -c proteus -n '__proteus_action_for stack status apply revert' -f -a 'status apply revert'
complete -c proteus -n '__proteus_action_for dns status apply revert' -f -a 'status apply revert'
complete -c proteus -n '__proteus_action_for resolved status apply revert' -f -a 'status apply revert'
complete -c proteus -n '__proteus_action_for ntp status apply revert' -f -a 'status apply revert'
complete -c proteus -n '__proteus_action_for dhcp status apply revert renew' -f -a 'status apply revert renew'
complete -c proteus -n '__proteus_action_for nft status apply revert' -f -a 'status apply revert'
complete -c proteus -n '__proteus_action_for portal status mark unmark list open' -f -a 'status mark unmark list open'
complete -c proteus -n '__proteus_action_for rf status apply revert scan chipset' -f -a 'status apply revert scan chipset'
complete -c proteus -n '__proteus_action_for kill status' -f -a 'status'
complete -c proteus -n '__proteus_action_for timer status list enable disable set reset logs' -f -a 'status list enable disable set reset logs'
complete -c proteus -n '__proteus_action_for config show get set enable disable edit validate reset keys set-profile' -f -a 'show get set enable disable edit validate reset keys set-profile'
complete -c proteus -n '__proteus_action_for persona list show use clear current random new edit validate import export' -f -a 'list show use clear current random new edit validate import export'
complete -c proteus -n '__proteus_action_for ssid list show set clear' -f -a 'list show set clear'
complete -c proteus -n '__proteus_action_for events run' -f -a 'run'
complete -c proteus -n '__proteus_action_for wiki search' -f -a 'search'

# Positionals.
complete -c proteus -n '__fish_seen_subcommand_from pin unpin' -f -a '(__proteus_pin_targets)'
complete -c proteus -n '__fish_seen_subcommand_from wiki help' -f -a '(__proteus_wiki_pages)'
complete -c proteus -n '__fish_seen_subcommand_from completions' -f -a 'bash zsh fish'
