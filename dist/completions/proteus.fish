# proteus fish completion
# Hand-written for stability and small size. Re-source-able.
#
# Install:
#   - Per-user:    cp to ~/.config/fish/completions/proteus.fish
#   - System-wide: cp to /usr/share/fish/vendor_completions.d/proteus.fish
#
# install.sh installs this to /usr/share/fish/vendor_completions.d/proteus.fish.

# True when no subcommand has been entered yet.
function __proteus_needs_subcommand
    set -l cmd (commandline -opc)
    set -l subs apply current diff dry-run help original pin reset revert \
        rotate show-config show-defaults status uninstall unpin wiki
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
        intro quickstart concepts mac-recipes bluetooth probes rotation \
        captive-portals dhcp ipv6 hostname-recipes enterprise-wifi dns \
        discovery stack-fingerprint rf-fingerprinting threat-model cli config \
        troubleshooting verifying uninstall internals faq glossary
end

# Wipe any prior bindings so this file is re-source-able.
complete -c proteus -e

# Subcommands.
complete -c proteus -n __proteus_needs_subcommand -a apply         -d 'Apply Proteus config to the system'
complete -c proteus -n __proteus_needs_subcommand -a current       -d 'List current MAC addresses'
complete -c proteus -n __proteus_needs_subcommand -a diff          -d 'Show diff between config, defaults, and live state'
complete -c proteus -n __proteus_needs_subcommand -a dry-run       -d 'Preview what a mutating command would do'
complete -c proteus -n __proteus_needs_subcommand -a help          -d 'Show help for a feature'
complete -c proteus -n __proteus_needs_subcommand -a original      -d 'Show the cached original MACs and hostname'
complete -c proteus -n __proteus_needs_subcommand -a pin           -d 'Pin an interface or NM connection to a specific MAC'
complete -c proteus -n __proteus_needs_subcommand -a reset         -d 'Reset config to defaults and re-apply'
complete -c proteus -n __proteus_needs_subcommand -a revert        -d 'Revert Proteus changes to the cached originals'
complete -c proteus -n __proteus_needs_subcommand -a rotate        -d 'Rotate MAC for one or all managed interfaces'
complete -c proteus -n __proteus_needs_subcommand -a show-config   -d 'Print the active config file'
complete -c proteus -n __proteus_needs_subcommand -a show-defaults -d 'Print the built-in default config'
complete -c proteus -n __proteus_needs_subcommand -a status        -d 'Show overall system + per-feature status'
complete -c proteus -n __proteus_needs_subcommand -a uninstall     -d 'Remove Proteus from the system'
complete -c proteus -n __proteus_needs_subcommand -a unpin         -d 'Remove a pin previously set with pin'
complete -c proteus -n __proteus_needs_subcommand -a wiki          -d 'Browse the embedded wiki'

# Global flags.
complete -c proteus -s v -l verbose  -d 'Increase log verbosity'
complete -c proteus -s q -l quiet    -d 'Decrease log verbosity'
complete -c proteus -l config        -d 'Override config path' -r -F
complete -c proteus -l state         -d 'Override state path'  -r -F
complete -c proteus -l no-color      -d 'Disable colored output'
complete -c proteus -s h -l help     -d 'Show help'
complete -c proteus -s V -l version  -d 'Show version'

# Per-subcommand flags. __fish_seen_subcommand_from gates them by context.
complete -c proteus -n '__fish_seen_subcommand_from status current original show-config show-defaults diff' \
    -l json -d 'Emit JSON'
complete -c proteus -n '__fish_seen_subcommand_from apply revert reset rotate uninstall' \
    -l yes -d 'Skip confirmation'

complete -c proteus -n '__fish_seen_subcommand_from current rotate' \
    -l iface -d 'Limit to a single interface' -x -a '(__proteus_ifaces)'
complete -c proteus -n '__fish_seen_subcommand_from uninstall' \
    -l purge -d 'Also remove /etc/proteus and /var/lib/proteus'

# Positionals.
complete -c proteus -n '__fish_seen_subcommand_from pin unpin' -f -a '(__proteus_pin_targets)'
complete -c proteus -n '__fish_seen_subcommand_from wiki help' -f -a '(__proteus_wiki_pages)'
