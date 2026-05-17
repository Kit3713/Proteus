# proteus bash completion
# Hand-written for stability and small size. Re-source-able.
#
# Install:
#   - System-wide: copy to /etc/bash_completion.d/proteus
#   - Per-user:    source from ~/.bashrc
#
# install.sh installs this to /usr/share/bash-completion/completions/proteus.

_proteus_subcommands="apply bluetooth completions config current dhcp diff dns doctor dry-run enterprise-wifi events help hostname ipv6 kill nft ntp original persona pin portal probe reset resolved resume revert rf rotate rotate-if-needed session show-config show-defaults ssid stack status timer uninstall unpin wiki"

_proteus_global_opts="-v --verbose -q --quiet --config --state --no-color --format -h --help -V --version"

_proteus_wiki_pages="backend bluetooth captive-portals cli concepts config dhcp discovery distro-support dns doctor enterprise-wifi faq getting-started glossary hostile-environments hostname-recipes internals intro ip-rotation ipv6 journald-network-logs kill-switch mac-recipes network-fingerprint-checklist personas per-ssid probes profiles quickstart real-world-testing recipes reproducible-builds rf-fingerprinting rotation security-checklist stack-fingerprint threat-model throttling-detect timer troubleshooting uninstall verifying wpa-supplicant-hardening"

_proteus_shells="bash zsh fish"
_proteus_formats="table json yaml"
_proteus_kill_actions="status"
_proteus_nft_actions="status apply revert"
_proteus_portal_actions="status mark unmark list open"
_proteus_bluetooth_actions="status apply revert"
_proteus_hostname_actions="status rotate pin revert"
_proteus_ipv6_actions="status apply revert"
_proteus_enterprise_wifi_actions="status enable disable"
_proteus_stack_actions="status apply revert"
_proteus_dns_actions="status apply revert"
_proteus_resolved_actions="status apply revert"
_proteus_ntp_actions="status apply revert"
_proteus_dhcp_actions="status apply revert renew"
_proteus_rf_actions="status apply revert scan chipset"
_proteus_timer_actions="status list enable disable set reset logs"
_proteus_config_actions="show get set enable disable edit validate reset keys set-profile"
_proteus_persona_actions="list show use clear current random new edit validate import export"
_proteus_ssid_actions="list show set clear"
_proteus_events_actions="run"
_proteus_wiki_actions="search"

# Skip virtual @-suffixed entries.
_proteus_ifaces() {
    if command -v ip >/dev/null 2>&1; then
        ip -o link 2>/dev/null | awk -F': ' '{print $2}' | grep -v '@' | tr '\n' ' '
    fi
}

_proteus_pin_targets() {
    local out
    out="$(_proteus_ifaces)"
    if command -v nmcli >/dev/null 2>&1; then
        out="$out $(nmcli -g name connection show 2>/dev/null | tr '\n' ' ')"
    fi
    echo "$out"
}

# Subcommand handlers. Naming convention: command_<subcommand>, kept in sync
# with src/cli/command.rs. The E2E recipe greps for these names.
command_status()         { COMPREPLY=( $(compgen -W "--json --watch --interval $_proteus_global_opts" -- "$cur") ); }
command_session()        { COMPREPLY=( $(compgen -W "--json --watch --interval $_proteus_global_opts" -- "$cur") ); }
command_original()       { COMPREPLY=( $(compgen -W "--json $_proteus_global_opts" -- "$cur") ); }
command_show_config()    { COMPREPLY=( $(compgen -W "--json $_proteus_global_opts" -- "$cur") ); }
command_show_defaults()  { COMPREPLY=( $(compgen -W "--json $_proteus_global_opts" -- "$cur") ); }
command_diff()           { COMPREPLY=( $(compgen -W "--json $_proteus_global_opts" -- "$cur") ); }
command_apply()          { COMPREPLY=( $(compgen -W "--yes $_proteus_global_opts" -- "$cur") ); }
command_revert()         { COMPREPLY=( $(compgen -W "--yes $_proteus_global_opts" -- "$cur") ); }
command_reset()          { COMPREPLY=( $(compgen -W "--yes --dry-run $_proteus_global_opts" -- "$cur") ); }
command_resume()         { COMPREPLY=( $(compgen -W "--yes $_proteus_global_opts" -- "$cur") ); }
command_dry_run()        { COMPREPLY=( $(compgen -W "$_proteus_subcommands" -- "$cur") ); }
command_uninstall()      { COMPREPLY=( $(compgen -W "--purge --yes $_proteus_global_opts" -- "$cur") ); }
command_doctor()         { COMPREPLY=( $(compgen -W "--json --quick $_proteus_global_opts" -- "$cur") ); }
command_probe()          { COMPREPLY=( $(compgen -W "--json --quick $_proteus_global_opts" -- "$cur") ); }
command_help()           { COMPREPLY=( $(compgen -W "$_proteus_wiki_pages $_proteus_subcommands $_proteus_global_opts" -- "$cur") ); }

command_current() {
    if [[ "$prev" == "--iface" ]]; then
        COMPREPLY=( $(compgen -W "$(_proteus_ifaces)" -- "$cur") )
    else
        COMPREPLY=( $(compgen -W "--json --iface --watch --interval $_proteus_global_opts" -- "$cur") )
    fi
}

command_rotate() {
    if [[ "$prev" == "--iface" ]]; then
        COMPREPLY=( $(compgen -W "$(_proteus_ifaces)" -- "$cur") )
    else
        COMPREPLY=( $(compgen -W "--iface --yes --explain $_proteus_global_opts" -- "$cur") )
    fi
}

command_rotate_if_needed() {
    if [[ "$prev" == "--iface" ]]; then
        COMPREPLY=( $(compgen -W "$(_proteus_ifaces)" -- "$cur") )
    else
        COMPREPLY=( $(compgen -W "--iface --cooldown --ssid --yes $_proteus_global_opts" -- "$cur") )
    fi
}

_proteus_pin_handler() {
    COMPREPLY=( $(compgen -W "$(_proteus_pin_targets) --mac --yes $_proteus_global_opts" -- "$cur") )
}
command_pin()   { _proteus_pin_handler; }
command_unpin() {
    COMPREPLY=( $(compgen -W "$(_proteus_pin_targets) --all --scope --yes $_proteus_global_opts" -- "$cur") )
}

command_completions() {
    COMPREPLY=( $(compgen -W "$_proteus_shells" -- "$cur") )
}

command_wiki() {
    # First positional after `wiki` may be `search` (a subcommand) or a page name.
    local sub=""
    local i
    for ((i=2; i < cword; i++)); do
        local w="${words[i]}"
        if [[ "$w" != -* && "$w" != "wiki" ]]; then
            sub="$w"
            break
        fi
    done
    if [[ "$sub" == "search" ]]; then
        COMPREPLY=( $(compgen -W "--json --limit $_proteus_global_opts" -- "$cur") )
    else
        COMPREPLY=( $(compgen -W "$_proteus_wiki_actions $_proteus_wiki_pages $_proteus_global_opts" -- "$cur") )
    fi
}

# Generic subcommand-with-actions handler: completes the action list when no
# action chosen, otherwise offers a coarse union of every flag any action
# accepts. We deliberately don't filter the flag list per-action — the
# precision win isn't worth maintaining ~70 per-action handlers in a
# hand-written completion. Action enums are kept in src/cli/actions.rs.
_proteus_action_handler() {
    local actions="$1"
    local sub=""
    local i
    for ((i=2; i < cword; i++)); do
        local w="${words[i]}"
        if [[ "$w" != -* ]]; then
            sub="$w"
            break
        fi
    done
    if [[ -z "$sub" ]]; then
        COMPREPLY=( $(compgen -W "$actions $_proteus_global_opts" -- "$cur") )
    else
        COMPREPLY=( $(compgen -W "--json --yes --connection --apply --reason --interval --lines --limit --kind --category --from --force --max-triggers --once-after-secs $_proteus_global_opts" -- "$cur") )
    fi
}

command_kill()            { _proteus_action_handler "$_proteus_kill_actions"; }
command_nft()             { _proteus_action_handler "$_proteus_nft_actions"; }
command_portal()          { _proteus_action_handler "$_proteus_portal_actions"; }
command_bluetooth()       { _proteus_action_handler "$_proteus_bluetooth_actions"; }
command_hostname()        { _proteus_action_handler "$_proteus_hostname_actions"; }
command_ipv6()            { _proteus_action_handler "$_proteus_ipv6_actions"; }
command_enterprise_wifi() { _proteus_action_handler "$_proteus_enterprise_wifi_actions"; }
command_stack()           { _proteus_action_handler "$_proteus_stack_actions"; }
command_dns()             { _proteus_action_handler "$_proteus_dns_actions"; }
command_resolved()        { _proteus_action_handler "$_proteus_resolved_actions"; }
command_ntp()             { _proteus_action_handler "$_proteus_ntp_actions"; }
command_dhcp()            { _proteus_action_handler "$_proteus_dhcp_actions"; }
command_rf()              { _proteus_action_handler "$_proteus_rf_actions"; }
command_timer()           { _proteus_action_handler "$_proteus_timer_actions"; }
command_config()          { _proteus_action_handler "$_proteus_config_actions"; }
command_persona()         { _proteus_action_handler "$_proteus_persona_actions"; }
command_ssid()            { _proteus_action_handler "$_proteus_ssid_actions"; }
command_events()          { _proteus_action_handler "$_proteus_events_actions"; }

_proteus() {
    local cur prev words cword
    _init_completion 2>/dev/null || {
        COMPREPLY=()
        cur="${COMP_WORDS[COMP_CWORD]}"
        prev="${COMP_WORDS[COMP_CWORD-1]}"
        words=("${COMP_WORDS[@]}")
        cword=$COMP_CWORD
    }

    case "$prev" in
        --config|--state)
            COMPREPLY=( $(compgen -f -- "$cur") )
            return 0
            ;;
        --format)
            COMPREPLY=( $(compgen -W "$_proteus_formats" -- "$cur") )
            return 0
            ;;
    esac

    # Walk forward to find the first non-flag word: that's the subcommand.
    local i subcmd=""
    for ((i=1; i < cword; i++)); do
        local w="${words[i]}"
        if [[ "$w" != -* ]]; then
            subcmd="$w"
            break
        fi
        # Skip the value of value-taking flags so it isn't mistaken for the subcommand.
        if [[ "$w" == "--config" || "$w" == "--state" || "$w" == "--format" ]]; then
            ((i++))
        fi
    done

    if [[ -z "$subcmd" ]]; then
        COMPREPLY=( $(compgen -W "$_proteus_subcommands $_proteus_global_opts" -- "$cur") )
        return 0
    fi

    local handler="command_${subcmd//-/_}"
    if declare -F "$handler" >/dev/null 2>&1; then
        "$handler"
    else
        COMPREPLY=( $(compgen -W "$_proteus_global_opts" -- "$cur") )
    fi
}

complete -F _proteus proteus
