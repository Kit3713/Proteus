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
USR_BIN_SYMLINK="/usr/bin/proteus"
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

# Mirrors `proteus uninstall --purge` from `src/commands/uninstall.rs`. Issue
# #219: the previous fallback only iterated three units and ignored every
# external integration file (sysctl/ipv6/timesyncd drop-ins, NM dispatcher,
# polkit policy, systemd-resolved drop-ins, SELinux fcontext). After a
# binary-missing fallback the system was left with a stale dispatcher hook
# firing on every connect and an orphan polkit action. The shell fallback
# now mirrors the Rust UNITS + EXTERNAL_DROPINS + EXTERNAL_FILES lists.
if [ "$USED_FALLBACK" -eq 1 ]; then
    info "manual cleanup"

    # Mirrors `UNITS` in src/commands/uninstall.rs (timers first so they
    # stop firing before we tear down the services they trigger).
    for unit in \
        proteus-rotate.timer \
        proteus-check.timer \
        proteus-rotate.service \
        proteus-check.service \
        proteus-resume.service \
        proteus-boot.service \
        proteus-events.service \
    ; do
        unit_path="$SYSTEMD_DIR/$unit"
        if [ "$DRY_RUN" -eq 1 ]; then
            printf 'would run: systemctl disable --now %s\n' "$unit"
            [ -f "$unit_path" ] && printf 'would run: rm -f %s\n' "$unit_path"
            continue
        fi
        # disable --now stops + disables in one call; ignore failures since
        # the unit may already be inactive or never have been installed.
        systemctl disable --now "$unit" >/dev/null 2>&1 || true
        [ -f "$unit_path" ] && rm -f "$unit_path"
        # Drop-in directory written by some Proteus releases (e.g. timer
        # drop-ins for PROTEUS_LOCK_TIMEOUT_MS); harmless when absent.
        [ -d "${unit_path}.d" ] && rm -rf "${unit_path}.d"
    done

    # Mirrors `EXTERNAL_DROPINS` in src/commands/uninstall.rs.
    for f in \
        /etc/sysctl.d/95-proteus.conf \
        /etc/sysctl.d/96-proteus-ipv6.conf \
        /etc/systemd/timesyncd.conf.d/10-proteus.conf \
    ; do
        [ -f "$f" ] || continue
        info "removing $f"
        run rm -f "$f"
    done

    # systemd-resolved drop-ins (10-proteus-*.conf in resolved.conf.d).
    if [ -d /etc/systemd/resolved.conf.d ]; then
        for f in /etc/systemd/resolved.conf.d/10-proteus-*.conf; do
            [ -f "$f" ] || continue
            info "removing $f"
            run rm -f "$f"
        done
    fi

    # Mirrors `EXTERNAL_FILES` in src/commands/uninstall.rs (issue #216):
    # NM dispatcher hook + polkit policy. Both are deployed by install.sh
    # but were missing from both uninstall paths.
    for f in \
        /etc/NetworkManager/dispatcher.d/01-proteus \
        /usr/share/polkit-1/actions/com.kit3713.proteus.policy \
    ; do
        [ -f "$f" ] || continue
        info "removing $f"
        run rm -f "$f"
    done

    if [ -f "$BINARY" ]; then
        info "removing $BINARY"
        # B11: ${BINARY:?…} pattern guards against a sourced/edited copy of
        # this script ever expanding $BINARY to empty and triggering `rm -f /`.
        # Belt-and-suspenders: $BINARY is set at the top of this file, but if
        # someone slices this fallback block into another script the guard
        # makes the failure mode loud rather than catastrophic.
        run rm -f "${BINARY:?BINARY must be set}"
        # Best-effort SELinux cleanup: remove the persistent fcontext rule.
        if command -v semanage >/dev/null 2>&1; then
            if [ "$DRY_RUN" -eq 1 ]; then
                printf 'would run: semanage fcontext -d %s\n' "$BINARY"
            else
                semanage fcontext -d "$BINARY" >/dev/null 2>&1 || true
            fi
        fi
    fi

    # Clean up the /usr/bin/proteus -> /usr/local/bin/proteus compat symlink
    # install.sh creates (B10). Only remove if it's still a symlink — never
    # touch a real file (that's a distro package's binary).
    if [ -L "$USR_BIN_SYMLINK" ]; then
        info "removing $USR_BIN_SYMLINK symlink"
        run rm -f "${USR_BIN_SYMLINK:?USR_BIN_SYMLINK must be set}"
    fi

    # nft table teardown — name matches src/nft (table inet proteus).
    if command -v nft >/dev/null 2>&1; then
        if [ "$DRY_RUN" -eq 1 ]; then
            printf 'would run: nft delete table inet proteus\n'
        else
            nft delete table inet proteus >/dev/null 2>&1 || true
        fi
    fi

    if [ "$PURGE" -eq 1 ]; then
        # B11: the `${var:?…}` guard turns `rm -rf $UNSET_VAR` (== `rm -rf`)
        # and `rm -rf "$UNSET_VAR"` (== `rm -rf ""`) into a hard error before
        # rm runs, instead of a no-op or a silent-recursive-delete-from-cwd.
        if [ -d "$CONFIG_DIR" ]; then
            info "removing $CONFIG_DIR"
            run rm -rf "${CONFIG_DIR:?CONFIG_DIR must be set}"
        fi
        if [ -d "$STATE_DIR" ]; then
            info "removing $STATE_DIR"
            run rm -rf "${STATE_DIR:?STATE_DIR must be set}"
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
