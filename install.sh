#!/bin/sh
# Proteus installer. POSIX shell only — no bashisms; must run under dash.
#
# Installs the binary to /usr/local/bin (so a future distro package in
# /usr/bin doesn't conflict), creates /etc/proteus and /var/lib/proteus,
# installs systemd units from dist/systemd if present, and labels the
# binary for SELinux on systems where semanage is available.
#
# Does NOT run `proteus apply` automatically — applying is mutating, and
# the user should review their config first.
#
# Usage: sudo ./install.sh [--dry-run]

set -eu

# ---- defaults ---------------------------------------------------------------

BINARY_SRC="target/release/proteus"
BINARY_DST="/usr/local/bin/proteus"
CONFIG_DIR="/etc/proteus"
STATE_DIR="/var/lib/proteus"
SYSTEMD_DIR="/etc/systemd/system"
UNITS_SRC="dist/systemd"
POLKIT_SRC="dist/polkit/com.kit3713.proteus.policy"
POLKIT_DIR="/usr/share/polkit-1/actions"
NM_DISPATCHER_SRC="dist/networkmanager/dispatcher.d/01-proteus"
NM_DISPATCHER_DST="/etc/NetworkManager/dispatcher.d/01-proteus" 

DRY_RUN=0

# ---- helpers ----------------------------------------------------------------

err() {
    printf 'install.sh: error: %s\n' "$1" >&2
}

warn() {
    printf 'install.sh: warning: %s\n' "$1" >&2
}

info() {
    printf 'install.sh: %s\n' "$1"
}

# Run a command, or just print it under --dry-run.
run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf 'would run: %s\n' "$*"
    else
        "$@"
    fi
}

usage() {
    cat <<EOF
Usage: $0 [--dry-run] [--help]

Installs the proteus binary, config and state directories, and systemd units.
Run as root. Requires Linux + systemd. Build the binary first with:
    cargo build --release

Options:
  --dry-run   Print what would be done without modifying the system.
  --help      Show this help.
EOF
}

# ---- arg parsing ------------------------------------------------------------

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN=1 ;;
        -h|--help) usage; exit 0 ;;
        *) err "unknown argument: $1"; usage >&2; exit 64 ;;
    esac
    shift
done

# ---- pre-flight -------------------------------------------------------------

# Root check is skipped under --dry-run so reviewers can preview without sudo.
if [ "$DRY_RUN" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then
    err "must run as root (try: sudo $0)"
    exit 1
fi

if [ "$(uname)" != "Linux" ]; then
    err "Proteus is Linux-only (uname=$(uname))"
    exit 70
fi

# systemd is a hard requirement — boot oneshot + two timers are part of the design.
if ! systemctl --version >/dev/null 2>&1; then
    err "systemd is required (systemctl not found)"
    exit 1
fi

if [ ! -f "$BINARY_SRC" ]; then
    err "binary not found at $BINARY_SRC"
    err "build first: cargo build --release"
    exit 1
fi

# ---- install binary ---------------------------------------------------------

info "installing binary to $BINARY_DST"
run install -m 0755 "$BINARY_SRC" "$BINARY_DST"

# ---- install dirs -----------------------------------------------------------

# Config dir is world-readable (0755): config is non-sensitive by design.
if [ ! -d "$CONFIG_DIR" ]; then
    info "creating $CONFIG_DIR"
    run install -d -m 0755 "$CONFIG_DIR"
else
    info "$CONFIG_DIR already exists, leaving as-is"
fi

# State dir is root-only (0700): caches the permanent MAC and original
# hostname — these are sacred. See wiki concepts page.
#
# Issue #275: a pre-existing $STATE_DIR may have been created by a
# previous install with a permissive umask, leaving state.json (and the
# .lock file) deletable by any local user. Re-chmod / re-chown on every
# install so an upgrade automatically tightens a misconfigured dir.
if [ ! -d "$STATE_DIR" ]; then
    info "creating $STATE_DIR"
    run install -d -m 0700 -o root -g root "$STATE_DIR"
else
    info "tightening $STATE_DIR permissions to 0700 root:root (issue #275)"
    run chmod 0700 "$STATE_DIR"
    run chown root:root "$STATE_DIR"
fi

# ---- systemd units ----------------------------------------------------------

UNITS_INSTALLED=0
if [ -d "$UNITS_SRC" ]; then
    # Iterate over .service and .timer files. Use a simple for-loop on a glob
    # rather than find -print0 / arrays; under nullglob-less POSIX sh we have
    # to guard against the unmatched-glob case explicitly.
    for unit in "$UNITS_SRC"/*.service "$UNITS_SRC"/*.timer; do
        [ -e "$unit" ] || continue
        base=$(basename "$unit")
        info "installing $base"
        run install -m 0644 "$unit" "$SYSTEMD_DIR/$base"
        UNITS_INSTALLED=1
    done
    if [ "$UNITS_INSTALLED" -eq 0 ]; then
        warn "$UNITS_SRC exists but contains no .service or .timer files"
    fi
else
    warn "$UNITS_SRC not found — skipping systemd units (timers and boot service won't be enabled)"
fi

# ---- polkit policy ----------------------------------------------------------

# A PolicyKit action policy that lets a future GUI wrapper elevate mutating
# proteus commands via pkexec (desktop password prompt) instead of sudo (TTY
# prompt). The binary itself does not enforce per-action policy; this file is
# a hint to pkexec and desktop tooling. Skip when polkit is not installed.
#
# The bundled policy points at /usr/bin/proteus (the distro-package path) so
# that the same source file works unmodified for RPM/.deb/Arch/Nix builds.
# install.sh deploys to $BINARY_DST (default /usr/local/bin/proteus) to avoid
# clashing with a future distro package, so we rewrite the exec.path here.
if [ -f "$POLKIT_SRC" ]; then
    if [ -d "$POLKIT_DIR" ]; then
        info "installing polkit policy to $POLKIT_DIR"
        polkit_dst="$POLKIT_DIR/com.kit3713.proteus.policy"
        annotate='<annotate key="org.freedesktop.policykit.exec.path">'
        if [ "$DRY_RUN" -eq 1 ]; then
            printf 'would run: install -m 0644 %s %s (with exec.path -> %s)\n' \
                "$POLKIT_SRC" "$polkit_dst" "$BINARY_DST"
        else
            tmp_policy=$(mktemp)
            trap 'rm -f "$tmp_policy"' EXIT
            sed "s|${annotate}/usr/bin/proteus</annotate>|${annotate}${BINARY_DST}</annotate>|g" \
                "$POLKIT_SRC" >"$tmp_policy"
            install -m 0644 "$tmp_policy" "$polkit_dst"
            rm -f "$tmp_policy"
            trap - EXIT
        fi
    else
        warn "$POLKIT_DIR not found — skipping polkit policy (PolicyKit not installed?)"
    fi
else
    warn "$POLKIT_SRC not found — skipping polkit policy"
fi

# ---- NetworkManager dispatcher ---------------------------------------------

# Event-driven rotation hook. Fires on every NM connection state change so
# Proteus reacts immediately to disconnect/reconnect — much faster than the
# 5-minute polling check timer. See dist/networkmanager/README.md.
if [ -f "$NM_DISPATCHER_SRC" ]; then
    info "installing NM dispatcher to $NM_DISPATCHER_DST"
    # Parent dir may not exist on systems without NM; create it idempotently.
    run install -d -m 0755 "$(dirname "$NM_DISPATCHER_DST")"
    run install -m 0755 -o root -g root "$NM_DISPATCHER_SRC" "$NM_DISPATCHER_DST"
else
    warn "$NM_DISPATCHER_SRC not found — skipping NM dispatcher hook"
fi

# ---- SELinux ----------------------------------------------------------------

# Fedora 43 has SELinux enforcing by default. Without bin_t on the binary,
# policy may block network operations (rtnetlink, dbus, etc.). semanage makes
# the label persistent across relabels; restorecon applies it now. Either may
# be missing on minimal installs — degrade gracefully.
if command -v semanage >/dev/null 2>&1; then
    info "setting SELinux fcontext for $BINARY_DST (bin_t)"
    # `-a` errors if the rule exists; on failure fall through to `-m` (modify),
    # which only succeeds if it does. Together they're idempotent.
    if [ "$DRY_RUN" -eq 1 ]; then
        printf 'would run: semanage fcontext -a -t bin_t %s\n' "$BINARY_DST"
    else
        semanage fcontext -a -t bin_t "$BINARY_DST" 2>/dev/null \
            || semanage fcontext -m -t bin_t "$BINARY_DST" 2>/dev/null \
            || warn "semanage fcontext failed; binary may be mislabeled"
    fi
elif command -v restorecon >/dev/null 2>&1; then
    warn "semanage not found; using restorecon only (label won't survive a full relabel)"
else
    warn "no SELinux tools found; skipping context labeling"
    warn "if SELinux is enforcing, install policycoreutils-python-utils and re-run"
fi

# restorecon applies the label now (or just enforces the default if no semanage rule).
if command -v restorecon >/dev/null 2>&1; then
    if [ "$DRY_RUN" -eq 1 ]; then
        printf 'would run: restorecon %s\n' "$BINARY_DST"
    else
        restorecon "$BINARY_DST" >/dev/null 2>&1 || true
    fi
fi

# ---- post-install -----------------------------------------------------------

info "reloading systemd"
run systemctl daemon-reload

if [ "$UNITS_INSTALLED" -eq 1 ]; then
    info "enabling timers and boot service"
    # Enable each unit individually so a missing one doesn't abort the others.
    # Idempotent: enable --now is a no-op when already enabled and active.
    # Under --dry-run we trust the just-printed copy step rather than re-checking
    # the destination (which we didn't actually write to).
    for unit in proteus-rotate.timer proteus-check.timer proteus-boot.service proteus-resume.service; do
        if [ "$DRY_RUN" -eq 1 ] || [ -f "$SYSTEMD_DIR/$unit" ]; then
            run systemctl enable --now "$unit" || warn "failed to enable $unit"
        fi
    done
fi

info "Proteus installed."
info "Run 'proteus status' to inspect, 'sudo proteus apply' to apply config."
info "See: proteus wiki quickstart"
