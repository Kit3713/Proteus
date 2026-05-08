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
    backend bluetooth captive-portals cli concepts config dhcp discovery
    distro-support dns doctor enterprise-wifi faq getting-started glossary
    hostile-environments hostname-recipes internals intro ip-rotation ipv6
    journald-network-logs kill-switch mac-recipes network-fingerprint-checklist
    personas per-ssid probes profiles quickstart real-world-testing recipes
    reproducible-builds rf-fingerprinting rotation security-checklist
    stack-fingerprint threat-model throttling-detect timer troubleshooting
    uninstall verifying wpa-supplicant-hardening
)

local -a _proteus_shells
_proteus_shells=(bash zsh fish)

local -a _proteus_formats
_proteus_formats=(table json yaml)

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

_proteus_shell_arg()  { _describe -t shells 'shell' _proteus_shells }
_proteus_format_arg() { _describe -t formats 'format' _proteus_formats }

local -a _proteus_global_opts
_proteus_global_opts=(
    '(-v --verbose)'{-v,--verbose}'[Increase log verbosity]'
    '(-q --quiet)'{-q,--quiet}'[Decrease log verbosity]'
    '--config[Override config path]:config file:_files'
    '--state[Override state path]:state file:_files'
    '--no-color[Disable colored output]'
    '--format[Output format]:format:(table json yaml)'
    '(-h --help)'{-h,--help}'[Show help]'
    '(-V --version)'{-V,--version}'[Show version]'
)

_proteus_commands() {
    local -a commands
    commands=(
        'apply:Apply Proteus config to the system'
        'bluetooth:Bluetooth alias / discoverable / BLE RPA management'
        'completions:Print embedded shell-completion script'
        'config:Manage Proteus configuration'
        'current:List current MAC addresses'
        'dhcp:DHCP option suppression'
        'diff:Show diff between config, defaults, and live state'
        'dns:DNS ECS-strip drop-in on systemd-resolved'
        'doctor:Run a battery of self-diagnostic checks'
        'dry-run:Preview what a mutating command would do'
        'enterprise-wifi:802.1X enterprise Wi-Fi anonymous outer identity'
        'events:Event-driven rotation framework'
        'help:Show help for a feature'
        'hostname:Hostname management via systemd hostnamed'
        'ipv6:IPv6 stable-privacy + temp addresses + DUID rotation'
        'kill:Emergency network kill switch'
        'nft:Manage the Proteus nftables table'
        'ntp:systemd-timesyncd NTP normalisation drop-in'
        'original:Show the cached original MACs and hostname'
        'persona:Device-persona / randomizer-recipe management'
        'pin:Pin an interface or NM connection to a specific MAC'
        'portal:Captive portal detection + known-portal SSID list'
        'probe:Run a manual probe round'
        'reset:Reset config to defaults'
        'resolved:systemd-resolved mDNS + LLMNR off drop-in'
        'resume:Restore network connectivity after kill'
        'revert:Revert Proteus changes to the cached originals'
        'rf:RF surface — Wi-Fi chipset inventory + TX-power reduction'
        'rotate:Rotate MAC for one or all managed interfaces'
        'rotate-if-needed:Rotate the MAC iff cooldown elapsed'
        'session:Show the current network session at a glance'
        'show-config:Print the active config file'
        'show-defaults:Print the built-in default config'
        'ssid:Per-SSID profile policies'
        'stack:Stack-fingerprint sysctl drop-in'
        'status:Show overall system + per-feature status'
        'timer:Manage Proteus systemd timers'
        'uninstall:Remove Proteus from the system'
        'unpin:Remove a pin previously set with pin'
        'wiki:Browse the embedded wiki'
    )
    _describe -t commands 'proteus command' commands
}

# Helpers shared by subcommands with identical option surfaces.
_proteus_json_only()      { _arguments $_proteus_global_opts '--json[Emit JSON]' }
_proteus_yes_only()       { _arguments $_proteus_global_opts '--yes[Skip confirmation]' }
_proteus_pin_arg()        { _arguments $_proteus_global_opts '--mac[Specific MAC]' '--yes[Skip confirmation]' '1:target:_proteus_pin_targets' }
_proteus_unpin_arg()      { _arguments $_proteus_global_opts '1:target:_proteus_pin_targets' }
_proteus_completions_arg(){ _arguments $_proteus_global_opts '1:shell:_proteus_shell_arg' }

_proteus_status() {
    _arguments $_proteus_global_opts \
        '--json[Emit JSON]' \
        '--watch[Re-run on a fixed interval]' \
        '--interval[Refresh cadence]:interval:'
}
_proteus_session() { _proteus_status }

_proteus_original()      { _proteus_json_only }
_proteus_show_config()   { _proteus_json_only }
_proteus_show_defaults() { _proteus_json_only }
_proteus_diff()          { _proteus_json_only }
_proteus_apply()         { _proteus_yes_only }
_proteus_revert()        { _proteus_yes_only }
_proteus_resume()        { _proteus_yes_only }
_proteus_pin()           { _proteus_pin_arg }
_proteus_unpin()         { _proteus_unpin_arg }
_proteus_completions()   { _proteus_completions_arg }

_proteus_reset() {
    _arguments $_proteus_global_opts \
        '--yes[Skip confirmation]' \
        '--dry-run[Print what would happen without writing]'
}

_proteus_doctor() {
    _arguments $_proteus_global_opts \
        '--json[Emit JSON]' \
        '--quick[Skip slower checks]'
}
_proteus_probe() {
    _arguments $_proteus_global_opts \
        '--json[Emit JSON]' \
        '--quick[Single endpoint, fast]'
}

_proteus_wiki() {
    _arguments $_proteus_global_opts \
        '1: :->wiki_first' \
        '*::arg:->wiki_args'
    case $state in
        wiki_first)
            local -a wiki_choices
            wiki_choices=( 'search:Full-text search' )
            _describe -t actions 'wiki action' wiki_choices
            _describe -t pages 'wiki page' _proteus_wiki_pages
            ;;
        wiki_args)
            if [[ ${words[1]} == search ]]; then
                _arguments $_proteus_global_opts \
                    '--json[Emit JSON]' \
                    '--limit[Cap on results]:limit:' \
                    '*:query:'
            fi
            ;;
    esac
}

_proteus_help() {
    _arguments $_proteus_global_opts \
        '1:feature:_proteus_wiki_page'
}

_proteus_current() {
    _arguments $_proteus_global_opts \
        '--json[Emit JSON]' \
        '--watch[Re-run on a fixed interval]' \
        '--interval[Refresh cadence]:interval:' \
        '--iface[Limit to a single interface]:iface:_proteus_ifaces'
}

_proteus_rotate() {
    _arguments $_proteus_global_opts \
        '--iface[Limit to a single interface]:iface:_proteus_ifaces' \
        '--yes[Skip confirmation]' \
        '--explain[Print every candidate considered]'
}

_proteus_rotate_if_needed() {
    _arguments $_proteus_global_opts \
        '--iface[Interface name]:iface:_proteus_ifaces' \
        '--cooldown[Cooldown budget in seconds]:secs:' \
        '--ssid[SSID being joined]:ssid:' \
        '--yes[Skip confirmation]'
}

_proteus_uninstall() {
    _arguments $_proteus_global_opts \
        '--purge[Also remove /etc/proteus and /var/lib/proteus]' \
        '--yes[Skip confirmation]'
}

# Generic action dispatcher: takes an array of "name:desc" entries.
_proteus_actions_dispatch() {
    local -a actions
    actions=("${(@P)1}")
    _arguments $_proteus_global_opts \
        '1: :->action' \
        '*::arg:->action_args'
    case $state in
        action) _describe -t actions 'action' actions ;;
        action_args)
            _arguments $_proteus_global_opts \
                '--json[Emit JSON]' \
                '--yes[Skip confirmation]'
            ;;
    esac
}

_proteus_kill() {
    local -a kill_actions
    kill_actions=( 'status:Show kill switch state' )
    _proteus_actions_dispatch kill_actions
}

_proteus_nft() {
    local -a nft_actions
    nft_actions=( 'status:Show nft state' 'apply:Install table' 'revert:Remove table' )
    _proteus_actions_dispatch nft_actions
}

_proteus_bluetooth() {
    local -a bt_actions
    bt_actions=( 'status:Adapter status' 'apply:Apply policy' 'revert:Restore' )
    _proteus_actions_dispatch bt_actions
}

_proteus_hostname() {
    local -a hn_actions
    hn_actions=( 'status:Show hostname state' 'rotate:Pick new hostname' 'pin:Pin specific name' 'revert:Restore original' )
    _proteus_actions_dispatch hn_actions
}

_proteus_ipv6() {
    local -a v6_actions
    v6_actions=( 'status:Show IPv6 state' 'apply:Apply privacy' 'revert:Restore sysctl' )
    _proteus_actions_dispatch v6_actions
}

_proteus_enterprise_wifi() {
    local -a ew_actions
    ew_actions=( 'status:Show 802.1X identity' 'enable:Set anonymous identity' 'disable:Clear anonymous identity' )
    _proteus_actions_dispatch ew_actions
}

_proteus_stack() {
    local -a st_actions
    st_actions=( 'status:Show sysctl' 'apply:Apply drop-in' 'revert:Remove drop-in' )
    _proteus_actions_dispatch st_actions
}

_proteus_dns() {
    local -a dns_actions
    dns_actions=( 'status:Show ECS-strip state' 'apply:Apply drop-in' 'revert:Remove drop-in' )
    _proteus_actions_dispatch dns_actions
}

_proteus_resolved() {
    local -a res_actions
    res_actions=( 'status:Show mDNS+LLMNR state' 'apply:Apply drop-in' 'revert:Remove drop-in' )
    _proteus_actions_dispatch res_actions
}

_proteus_ntp() {
    local -a ntp_actions
    ntp_actions=( 'status:Show NTP state' 'apply:Apply drop-in' 'revert:Remove drop-in' )
    _proteus_actions_dispatch ntp_actions
}

_proteus_dhcp() {
    local -a dhcp_actions
    dhcp_actions=( 'status:Show DHCP suppression' 'apply:Apply' 'revert:Restore' 'renew:Release+renew lease' )
    _proteus_actions_dispatch dhcp_actions
}

_proteus_rf() {
    local -a rf_actions
    rf_actions=( 'status:Show TX-power' 'apply:Apply reduction' 'revert:Restore' 'scan:Scan policy' 'chipset:Firmware/driver inventory' )
    _proteus_actions_dispatch rf_actions
}

_proteus_portal() {
    local -a p_actions
    p_actions=( 'status:Show portal state' 'mark:Add SSID' 'unmark:Remove SSID' 'list:List SSIDs' 'open:Open portal page' )
    _proteus_actions_dispatch p_actions
}

_proteus_timer() {
    local -a t_actions
    t_actions=( 'status:Show timer state' 'list:List timer types' 'enable:Enable+start' 'disable:Disable+stop' 'set:Change cadence' 'reset:Reset cadence' 'logs:Tail journald' )
    _proteus_actions_dispatch t_actions
}

_proteus_config() {
    local -a c_actions
    c_actions=( 'show:Print active config' 'get:Print one value' 'set:Set one value' 'enable:Enable component' 'disable:Disable component' 'edit:Open in $EDITOR' 'validate:Parse and report' 'reset:Reset section' 'keys:List supported keys' 'set-profile:Set active profile' )
    _proteus_actions_dispatch c_actions
}

_proteus_persona() {
    local -a per_actions
    per_actions=( 'list:List personas' 'show:Print schema' 'use:Set active persona' 'clear:Drop to randomizer' 'current:Show active' 'random:Pick random id' 'new:Clone existing' 'edit:Open in $EDITOR' 'validate:Schema-check toml' 'import:Copy into store' 'export:Copy to path' )
    _proteus_actions_dispatch per_actions
}

_proteus_ssid() {
    local -a s_actions
    s_actions=( 'list:List per-SSID entries' 'show:Show resolved policy' 'set:Set field' 'clear:Drop block' )
    _proteus_actions_dispatch s_actions
}

_proteus_events() {
    local -a e_actions
    e_actions=( 'run:Run event daemon' )
    _proteus_actions_dispatch e_actions
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
