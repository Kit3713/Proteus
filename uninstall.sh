#!/bin/sh
# Proteus uninstaller. Thin wrapper around `proteus uninstall --purge --yes`
# so distro packages can reuse the same code path. Falls back to manual
# cleanup if the binary is missing or fails — see `proteus wiki uninstall`
# for the full manual recipe.
#
# Usage: sudo ./uninstall.sh [--dry-run] [--no-purge]

set -eu

# ---- defaults ---------------------------------------------------------------

BINARY="/usr/local/bin/proteus"
CONFIG_DIR="/etc/proteus"
STATE_DIR="/var/lib/proteus"
SYSTEMD_DIR="/etc/systemd/system"
WIKI_PAGE="proteus wiki uninstall"

DRY_RUN=0
PURGE=1

# ---- helpers ----------------------------------------------------------------

err() {
    printf 'uninstall.sh: error: %s\n' "$1" >&2
}

warn() {
    printf 'uninstall.sh: warning: %s\n' "$1" >&2
}

info() {
    printf 'uninstall.sh: %s\n' "$1"
}

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf 'would run: %s\n' "$*"
    else
        "$@"
    fi
}

usage() {
    cat <<EOF
Usage: $0 [--dry-run] [--no-purge] [--help]

Removes the proteus binary, systemd units, and (with --purge, the default)
$CONFIG_DIR and $STATE_DIR. Run as root.

Options:
  --no-purge   Keep $CONFIG_DIR and $STATE_DIR after removal.
  --dry-run    Print what would be done without modifying the system.
  --help       Show this help.
EOF
}

# ---- arg parsing ------------------------------------------------------------

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)  DRY_RUN=1 ;;
        --no-purge) PURGE=0 ;;
        -h|--help)  usage; exit 0 ;;
        *) err "unknown argument: $1"; usage >&2; exit 64 ;;
    esac
    shift
done

# ---- pre-flight -------------------------------------------------------------

if [ "$DRY_RUN" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then
    err "must run as root (try: sudo $0)"
    exit 1
fi

# ---- primary path: delegate to the binary -----------------------------------

# Build the arg list once so dry-run and real-run stay in sync.
if [ "$PURGE" -eq 1 ]; then
    set -- uninstall --purge --yes
else
    set -- uninstall --yes
fi

USED_FALLBACK=0
if [ -x "$BINARY" ]; then
    info "delegating to $BINARY $*"
    # `if !` keeps `set -e` from aborting on a non-zero exit.
    if [ "$DRY_RUN" -eq 1 ]; then
        printf 'would run: %s %s\n' "$BINARY" "$*"
    elif ! "$BINARY" "$@"; then
        warn "'$BINARY uninstall' failed; falling back to manual cleanup"
        warn "see $WIKI_PAGE for the manual recipe"
        USED_FALLBACK=1
    fi
else
    warn "$BINARY not found; falling back to manual cleanup"
    warn "see $WIKI_PAGE for the manual recipe"
    USED_FALLBACK=1
fi

# ---- fallback: manual cleanup ----------------------------------------------

# Mirrors `proteus uninstall --purge`: stop and disable timers, remove unit
# files, remove the binary, and (under --purge) wipe state and config.
# Kept minimal because the binary path above is the canonical implementation.
if [ "$USED_FALLBACK" -eq 1 ]; then
    info "manual cleanup"

    for unit in proteus-rotate.timer proteus-check.timer proteus-boot.service; do
        unit_path="$SYSTEMD_DIR/$unit"
        [ -f "$unit_path" ] || continue
        # disable --now stops + disables in one call; ignore failures since
        # the unit may already be inactive.
        if [ "$DRY_RUN" -eq 1 ]; then
            printf 'would run: systemctl disable --now %s\n' "$unit"
            printf 'would run: rm -f %s\n' "$unit_path"
        else
            systemctl disable --now "$unit" >/dev/null 2>&1 || true
            rm -f "$unit_path"
        fi
    done

    if [ -f "$BINARY" ]; then
        info "removing $BINARY"
        run rm -f "$BINARY"
        # Best-effort SELinux cleanup: remove the persistent fcontext rule.
        if command -v semanage >/dev/null 2>&1; then
            if [ "$DRY_RUN" -eq 1 ]; then
                printf 'would run: semanage fcontext -d %s\n' "$BINARY"
            else
                semanage fcontext -d "$BINARY" >/dev/null 2>&1 || true
            fi
        fi
    fi

    if [ "$PURGE" -eq 1 ]; then
        if [ -d "$CONFIG_DIR" ]; then
            info "removing $CONFIG_DIR"
            run rm -rf "$CONFIG_DIR"
        fi
        if [ -d "$STATE_DIR" ]; then
            info "removing $STATE_DIR"
            run rm -rf "$STATE_DIR"
        fi
    else
        info "keeping $CONFIG_DIR and $STATE_DIR (--no-purge)"
    fi

    run systemctl daemon-reload || true
fi

# ---- summary ----------------------------------------------------------------

if [ "$USED_FALLBACK" -eq 1 ]; then
    info "uninstall complete via manual fallback"
    info "if anything looks off, see: $WIKI_PAGE"
else
    info "uninstall complete"
fi
