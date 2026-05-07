# tests/integration/scenarios/lib.sh
#
# Shared POSIX helpers sourced by every scenario script. Sourced (not
# exec'd) so the FAILED counter stays in the caller's shell.
#
# Conventions:
#   FAILED  — running count, owned by the caller, init to 0 before sourcing.
#   ok / fail — print a one-line result, increment FAILED on fail.
#   expect_zero <desc> <cmd...> — run cmd; expect exit 0.
#   expect_rc_in <desc> <rc-pattern> <cmd...> — accept any rc matching the
#     case pattern (e.g. "64|66").
#   expect_contains <desc> <needle> <cmd...> — run cmd, assert stdout
#     contains the literal needle.

# shellcheck shell=sh

ok() { printf '  ok: %s\n' "$1"; }

fail() {
    printf '  FAIL: %s\n' "$1" >&2
    FAILED=$((FAILED + 1))
}

# Run cmd, ignore stdout/stderr, assert rc=0.
expect_zero() {
    desc=$1
    shift
    if "$@" >/dev/null 2>&1; then
        ok "$desc"
    else
        fail "$desc (rc=$?)"
    fi
}

# Run cmd, ignore stdout/stderr, assert rc is one of the listed codes.
# Example: expect_rc_in "no --yes" 64 66 -- proteus apply
expect_rc_in() {
    desc=$1
    shift
    # Collect expected codes until we hit `--`.
    expected=
    while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
        expected="$expected $1"
        shift
    done
    [ "$1" = "--" ] && shift
    "$@" >/dev/null 2>&1
    rc=$?
    for e in $expected; do
        if [ "$rc" -eq "$e" ]; then
            ok "$desc (rc=$rc)"
            return
        fi
    done
    fail "$desc expected rc in [${expected# }], got $rc"
}

# Run cmd, capture stdout, assert it contains the literal needle.
expect_contains() {
    desc=$1
    needle=$2
    shift 2
    if "$@" 2>/dev/null | grep -qF -- "$needle"; then
        ok "$desc"
    else
        fail "$desc — expected output to contain '$needle'"
    fi
}

# Print the per-scenario summary and return 0/1 for the caller's exit.
finish() {
    name=$1
    if [ "$FAILED" -gt 0 ]; then
        printf 'scenario %s: %d failure(s)\n' "$name" "$FAILED" >&2
        return 1
    fi
    printf 'scenario %s: all checks passed\n' "$name"
    return 0
}
