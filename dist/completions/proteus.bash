# proteus bash completion
# Hand-written for stability and small size. Re-source-able.
#
# Install:
#   - System-wide: copy to /etc/bash_completion.d/proteus
#   - Per-user:    source from ~/.bashrc
#
# install.sh installs this to /usr/share/bash-completion/completions/proteus.

_proteus_subcommands="apply current diff dry-run help original pin reset revert rotate show-config show-defaults status uninstall unpin wiki"

_proteus_global_opts="-v --verbose -q --quiet --config --state --no-color -h --help -V --version"

_proteus_wiki_pages="intro quickstart concepts mac-recipes bluetooth probes rotation captive-portals dhcp ipv6 hostname-recipes enterprise-wifi dns discovery stack-fingerprint rf-fingerprinting threat-model cli config troubleshooting verifying uninstall internals faq glossary"

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
# with src/cli.rs. The E2E recipe greps for these names.
command_status()         { COMPREPLY=( $(compgen -W "--json $_proteus_global_opts" -- "$cur") ); }
command_original()       { COMPREPLY=( $(compgen -W "--json $_proteus_global_opts" -- "$cur") ); }
command_show_config()    { COMPREPLY=( $(compgen -W "--json $_proteus_global_opts" -- "$cur") ); }
command_show_defaults()  { COMPREPLY=( $(compgen -W "--json $_proteus_global_opts" -- "$cur") ); }
command_diff()           { COMPREPLY=( $(compgen -W "--json $_proteus_global_opts" -- "$cur") ); }
command_apply()          { COMPREPLY=( $(compgen -W "--yes $_proteus_global_opts" -- "$cur") ); }
command_revert()         { COMPREPLY=( $(compgen -W "--yes $_proteus_global_opts" -- "$cur") ); }
command_reset()          { COMPREPLY=( $(compgen -W "--yes $_proteus_global_opts" -- "$cur") ); }
command_dry_run()        { COMPREPLY=( $(compgen -W "$_proteus_subcommands" -- "$cur") ); }
command_uninstall()      { COMPREPLY=( $(compgen -W "--purge --yes $_proteus_global_opts" -- "$cur") ); }
command_wiki()           { COMPREPLY=( $(compgen -W "$_proteus_wiki_pages $_proteus_global_opts" -- "$cur") ); }
command_help()           { COMPREPLY=( $(compgen -W "$_proteus_wiki_pages $_proteus_subcommands $_proteus_global_opts" -- "$cur") ); }

command_current() {
    if [[ "$prev" == "--iface" ]]; then
        COMPREPLY=( $(compgen -W "$(_proteus_ifaces)" -- "$cur") )
    else
        COMPREPLY=( $(compgen -W "--json --iface $_proteus_global_opts" -- "$cur") )
    fi
}

command_rotate() {
    if [[ "$prev" == "--iface" ]]; then
        COMPREPLY=( $(compgen -W "$(_proteus_ifaces)" -- "$cur") )
    else
        COMPREPLY=( $(compgen -W "--iface --yes $_proteus_global_opts" -- "$cur") )
    fi
}

_proteus_pin_handler() {
    COMPREPLY=( $(compgen -W "$(_proteus_pin_targets) $_proteus_global_opts" -- "$cur") )
}
command_pin()   { _proteus_pin_handler; }
command_unpin() { _proteus_pin_handler; }

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
        if [[ "$w" == "--config" || "$w" == "--state" ]]; then
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
