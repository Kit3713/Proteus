#!/bin/sh
# Local pre-push checker for Proteus. Mirrors .github/workflows/ci.yml so
# failures surface here, not on the CI runner. POSIX-only: must run under dash.
#
# Steps (fail-fast, in CI order):
#   1. cargo fmt --check
#   2. cargo clippy --all-targets -- -D warnings
#   3. cargo test
#   4. cargo build --release
#   5. strip target/release/proteus  (Linux only)
#   6. binary size <= 4,000,000 bytes (release-time hard cap)
#   7. shell-syntax check install.sh / uninstall.sh
#   8. groff lint dist/man/proteus.1
#
# Exit code: 0 on success, 1 on first failure.

set -e

usage() {
    cat <<'EOF'
Usage: scripts/check.sh [--no-build] [--quick] [-h|--help]

Runs the same checks CI does, fail-fast, in CI order.

Options:
    --no-build  Skip release build, size check, shell, and man checks
                (fmt + clippy + test only).
    --quick     Same as --no-build, kept as a familiar alias.
    -h, --help  Print this message and exit.

Exit code: 0 on success, 1 on first failure.
EOF
}

NO_BUILD=0
for arg in "$@"; do
    case "$arg" in
        --no-build|--quick) NO_BUILD=1 ;;
        -h|--help)          usage; exit 0 ;;
        *)                  printf 'check.sh: unknown option: %s\n' "$arg" >&2
                            usage >&2
                            exit 1 ;;
    esac
done

# Anchor to repo root so the script works regardless of cwd.
SCRIPT_DIR=$(cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd)
cd "$REPO_ROOT"

# Per-step timings appended one line at a time: "STEP\tSECONDS".
TIMINGS_FILE=$(mktemp)
# `trap` runs even on `set -e` aborts; cleans the temp file unconditionally.
trap 'rm -f "$TIMINGS_FILE"' EXIT INT TERM

# Print a header, run a step, record wallclock seconds, abort on failure.
# Step stdout/stderr stream live so failures aren't suppressed.
run_step() {
    step_name=$1
    shift
    printf '\n=== %s ===\n' "$step_name"
    start=$(date +%s)
    # Subshell isolates `set -e` and lets us catch the rc explicitly.
    if ( "$@" ); then
        end=$(date +%s)
        printf '%s\t%s\n' "$step_name" "$((end - start))" >> "$TIMINGS_FILE"
    else
        rc=$?
        end=$(date +%s)
        printf '\nFAIL: step "%s" exited with code %d after %ss\n' \
            "$step_name" "$rc" "$((end - start))" >&2
        exit 1
    fi
}

# Render the timings file as a summary block; optionally append a total line.
emit_summary() {
    title=$1
    show_total=$2
    printf '\n=== %s ===\n' "$title"
    total=0
    while IFS='	' read -r name secs; do
        printf '  %4ss  %s\n' "$secs" "$name"
        total=$((total + secs))
    done < "$TIMINGS_FILE"
    if [ "$show_total" -eq 1 ]; then
        printf '  ----\n'
        printf '  %4ss  total\n' "$total"
    fi
}

# --- step 1: fmt -----------------------------------------------------------
run_step "cargo fmt --check" cargo fmt --check

# --- step 2: clippy --------------------------------------------------------
run_step "cargo clippy --all-targets -- -D warnings" \
    cargo clippy --all-targets -- -D warnings

# --- step 3: test ----------------------------------------------------------
run_step "cargo test --locked" cargo test --locked

if [ "$NO_BUILD" -eq 1 ]; then
    emit_summary "SUMMARY (quick mode)" 0
    printf '\nOK: quick checks passed (build, size, shell, man skipped)\n'
    exit 0
fi

# --- step 4: release build -------------------------------------------------
run_step "cargo build --release --locked" cargo build --release --locked

# --- step 5: strip ---------------------------------------------------------
# `strip` only ships on Linux/BSD; on macOS dev hosts the size invariant
# would be measured against an unstripped binary, which is misleading. Skip
# cleanly there rather than half-doing it.
UNAME=$(uname -s)
case "$UNAME" in
    Linux)
        run_step "strip target/release/proteus" strip target/release/proteus
        ;;
    *)
        printf '\n=== strip target/release/proteus ===\n'
        printf 'SKIP: not on Linux (uname=%s); size check will use unstripped binary\n' \
            "$UNAME"
        printf 'strip (skipped)\t0\n' >> "$TIMINGS_FILE"
        ;;
esac

# --- step 6: size cap ------------------------------------------------------
size_check() {
    bin=target/release/proteus
    if [ ! -f "$bin" ]; then
        printf 'FAIL: %s not found\n' "$bin" >&2
        return 1
    fi
    size=$(wc -c < "$bin")
    # Strip surrounding whitespace from `wc` output (BSD wc pads).
    size=$(printf '%s' "$size" | tr -d ' \t\n')
    printf 'PROTEUS_BINARY_SIZE_BYTES=%s\n' "$size"
    printf 'PROTEUS_BINARY_SIZE_LIMIT_BYTES=4000000\n'
    if [ "$size" -gt 4000000 ]; then
        printf 'FAIL: stripped proteus binary is %s bytes, exceeds 4,000,000 byte cap\n' \
            "$size" >&2
        return 1
    fi
    printf 'OK: stripped proteus binary is %s bytes (under 4,000,000 byte cap)\n' \
        "$size"
}
run_step "binary size <= 4 MB" size_check

# --- step 7: install / uninstall shell syntax ------------------------------
shell_check() {
    rc=0
    # B13: validate install.sh / uninstall.sh under POSIX sh, not bash. Both
    # scripts declare `#!/bin/sh` and document themselves as POSIX-only;
    # using `bash -n` here let bashisms slip through silently because bash
    # accepts a strict superset of dash. `sh -n` runs the system /bin/sh
    # in noexec mode — on Debian/Ubuntu CI runners that's dash, which is
    # the same shell distro packagers will see when they invoke the script.
    # Also covers the NM dispatcher hook (POSIX-ified per B6).
    for f in install.sh uninstall.sh dist/networkmanager/dispatcher.d/01-proteus; do
        if [ -f "$f" ]; then
            printf 'sh -n %s\n' "$f"
            if ! sh -n "$f"; then
                rc=1
            fi
        else
            printf 'SKIP: %s not present\n' "$f"
        fi
    done
    return "$rc"
}
run_step "shell syntax (install.sh, uninstall.sh)" shell_check

# --- step 8: man-page lint -------------------------------------------------
man_check() {
    man=dist/man/proteus.1
    if [ ! -f "$man" ]; then
        printf 'SKIP: %s not present\n' "$man"
        return 0
    fi
    if ! command -v groff >/dev/null 2>&1; then
        printf 'SKIP: groff not installed; man-page lint requires groff\n'
        return 0
    fi
    printf 'groff -ww -Tutf8 -man %s\n' "$man"
    groff -ww -Tutf8 -man "$man" > /dev/null
}
run_step "man-page lint (dist/man/proteus.1)" man_check

# --- summary ---------------------------------------------------------------
emit_summary "SUMMARY" 1
printf '\nOK: all checks passed\n'
