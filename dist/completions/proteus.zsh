#compdef proteus
# proteus zsh completion
# Hand-written for stability and small size. Re-source-able.
#
# Install:
#   - Copy to a directory in $fpath (e.g. /usr/share/zsh/site-functions/_proteus)
#     or ~/.zsh/completions/_proteus and add that dir to $fpath in ~/.zshrc.
#
# install.sh installs this to /usr/share/zsh/site-functions/_proteus.

local -a _proteus_wiki_pages
_proteus_wiki_pages=(
    intro quickstart concepts mac-recipes bluetooth probes rotation
    captive-portals dhcp ipv6 hostname-recipes enterprise-wifi dns
    discovery stack-fingerprint rf-fingerprinting threat-model cli config
    troubleshooting verifying uninstall internals faq glossary
)

# Skip virtual @-suffixed entries.
_proteus_ifaces() {
    local -a ifaces
    if (( $+commands[ip] )); then
        ifaces=( ${(f)"$(ip -o link 2>/dev/null | awk -F': ' '{print $2}' | grep -v '@')"} )
    fi
    _describe -t ifaces 'interface' ifaces
}

_proteus_pin_targets() {
    local -a targets
    if (( $+commands[ip] )); then
        targets+=( ${(f)"$(ip -o link 2>/dev/null | awk -F': ' '{print $2}' | grep -v '@')"} )
    fi
    if (( $+commands[nmcli] )); then
        targets+=( ${(f)"$(nmcli -g name connection show 2>/dev/null)"} )
    fi
    _describe -t targets 'pin target' targets
}

_proteus_wiki_page() {
    _describe -t pages 'wiki page' _proteus_wiki_pages
}

local -a _proteus_global_opts
_proteus_global_opts=(
    '(-v --verbose)'{-v,--verbose}'[Increase log verbosity]'
    '(-q --quiet)'{-q,--quiet}'[Decrease log verbosity]'
    '--config[Override config path]:config file:_files'
    '--state[Override state path]:state file:_files'
    '--no-color[Disable colored output]'
    '(-h --help)'{-h,--help}'[Show help]'
    '(-V --version)'{-V,--version}'[Show version]'
)

_proteus_commands() {
    local -a commands
    commands=(
        'apply:Apply Proteus config to the system'
        'current:List current MAC addresses'
        'diff:Show diff between config, defaults, and live state'
        'dry-run:Preview what a mutating command would do'
        'help:Show help for a feature'
        'original:Show the cached original MACs and hostname'
        'pin:Pin an interface or NM connection to a specific MAC'
        'reset:Reset config to defaults and re-apply'
        'revert:Revert Proteus changes to the cached originals'
        'rotate:Rotate MAC for one or all managed interfaces'
        'show-config:Print the active config file'
        'show-defaults:Print the built-in default config'
        'status:Show overall system + per-feature status'
        'uninstall:Remove Proteus from the system'
        'unpin:Remove a pin previously set with pin'
        'wiki:Browse the embedded wiki'
    )
    _describe -t commands 'proteus command' commands
}

# Helpers shared by subcommands with identical option surfaces.
_proteus_json_only() { _arguments $_proteus_global_opts '--json[Emit JSON]' }
_proteus_yes_only()  { _arguments $_proteus_global_opts '--yes[Skip confirmation]' }
_proteus_pin_arg()   { _arguments $_proteus_global_opts '1:target:_proteus_pin_targets' }
_proteus_wiki_arg()  { _arguments $_proteus_global_opts '1:page:_proteus_wiki_page' }

_proteus_status()        { _proteus_json_only }
_proteus_original()      { _proteus_json_only }
_proteus_show_config()   { _proteus_json_only }
_proteus_show_defaults() { _proteus_json_only }
_proteus_diff()          { _proteus_json_only }
_proteus_apply()         { _proteus_yes_only }
_proteus_revert()        { _proteus_yes_only }
_proteus_reset()         { _proteus_yes_only }
_proteus_pin()           { _proteus_pin_arg }
_proteus_unpin()         { _proteus_pin_arg }
_proteus_wiki()          { _proteus_wiki_arg }
_proteus_help()          { _proteus_wiki_arg }

_proteus_current() {
    _arguments $_proteus_global_opts \
        '--json[Emit JSON]' \
        '--iface[Limit to a single interface]:iface:_proteus_ifaces'
}

_proteus_rotate() {
    _arguments $_proteus_global_opts \
        '--iface[Limit to a single interface]:iface:_proteus_ifaces' \
        '--yes[Skip confirmation]'
}

_proteus_uninstall() {
    _arguments $_proteus_global_opts \
        '--purge[Also remove /etc/proteus and /var/lib/proteus]' \
        '--yes[Skip confirmation]'
}

_proteus() {
    local context state state_descr line
    typeset -A opt_args

    _arguments -C \
        $_proteus_global_opts \
        '1: :_proteus_commands' \
        '*::arg:->args'

    if [[ $state == args ]]; then
        local cmd="${words[1]}"
        local handler="_proteus_${cmd//-/_}"
        if (( $+functions[$handler] )); then
            $handler
        fi
    fi
}

_proteus "$@"
