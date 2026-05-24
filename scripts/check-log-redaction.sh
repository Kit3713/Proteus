#!/bin/sh
# check-log-redaction.sh — CI guard: forbid new raw-identifier tracing call sites.
#
# Proteus erases device fingerprints; it must never log the very identifiers
# it is hiding.  Every tracing!() call site that interpolates a raw MAC
# address, SSID, hostname, or 802.1X identity must route the value through
# the corresponding helper in crate::redaction::{mac,ssid,hostname,identity}.
#
# This script greps src/ for the known "raw" patterns and exits non-zero if
# any match is found that is NOT covered by the allowlist.
#
# PATTERNS DETECTED (conservative — favor precision over recall):
#   1. "= %mac,"  / "= ?mac,"  — a field whose value is a Mac typed variable
#      displayed via tracing's Display (%)/Debug (?) formatters.
#   2. "= %candidate,"  / "= ?candidate,"  — same for the common "candidate"
#      variable name used in mac/generator.rs and mac/probe.rs.
#   3. "ssid = " followed by any value that does NOT pass through the
#      redaction helper (i.e. the value string does not contain "redaction").
#      The current pre-existing site uses ssid_safe.as_str(), which provides
#      terminal-injection safety but still exposes the raw SSID identity.
#
# FALSE-POSITIVE EXCLUSIONS (intentional):
#   - "= %iface"  — interface name (eth0/wlan0), not a fingerprinting identifier
#   - "= %token"  — OUI vendor token, not a full MAC or network identity
#   - "= %peer"   — peer IP, not a MAC/SSID/hostname identity
#   - UUID fields — not device identifiers in the fingerprint sense
#   These are excluded by the specificity of the matched variable names.
#
# ALLOWLIST: scripts/log-redaction-allowlist.txt holds FILE:SNIPPET pairs for
# pre-existing raw sites that a sibling PR is removing.  Each site is skipped
# during the scan.  The file and snippet must both match (grep -F on the
# trimmed snippet in the named file) for the bypass to take effect.
#
# EXIT CODES:
#   0 — no raw-identifier violations (clean or all matches are allowlisted)
#   1 — one or more new raw-identifier violations; offending file:line printed
#
# USAGE:
#   bash scripts/check-log-redaction.sh
#   (Run from the repository root; the script anchors itself to the repo root
#    automatically via SCRIPT_DIR.)
#
# NOTE: shellcheck is NOT installed on the dev host but runs in the CI
# packaging-lint job against this file.  The script is written for POSIX sh.

set -eu

# ---------------------------------------------------------------------------
# Resolve repo root regardless of cwd.
# ---------------------------------------------------------------------------
SCRIPT_DIR=$(cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd)
SRC_DIR="$REPO_ROOT/src"
ALLOWLIST="$SCRIPT_DIR/log-redaction-allowlist.txt"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# is_allowlisted FILE_REL SNIPPET
# Returns 0 if both the file-relative-path and the trimmed snippet appear in
# the allowlist as a matched pair.  Line drift is irrelevant — we match by
# file + content snippet, not by line number.
is_allowlisted() {
    al_file="$1"
    al_snippet="$2"

    if [ ! -f "$ALLOWLIST" ]; then
        return 1
    fi

    # Iterate allowlist entries.
    while IFS= read -r entry; do
        # Skip blank lines and comments.
        case "$entry" in
            ''|'#'*) continue ;;
        esac

        # Entry format: FILE:SNIPPET  (first colon is the separator; snippet
        # may itself contain colons so we split only on the first one).
        al_entry_file="${entry%%:*}"
        al_entry_snippet="${entry#*:}"

        # Trim leading whitespace from snippet (the allowlist author may indent).
        al_entry_snippet=$(printf '%s' "$al_entry_snippet" | sed 's/^[[:space:]]*//')

        if [ "$al_file" = "$al_entry_file" ] && \
           grep -qF "$al_entry_snippet" "$REPO_ROOT/$al_entry_file" 2>/dev/null; then
            # Both the file matches and the snippet is still present in that
            # file — this is an allowlisted site.  Now check if the candidate
            # snippet matches the allowlist snippet (same literal string).
            trimmed_snippet=$(printf '%s' "$al_snippet" | sed 's/^[[:space:]]*//')
            if [ "$trimmed_snippet" = "$al_entry_snippet" ]; then
                return 0
            fi
        fi
    done < "$ALLOWLIST"

    return 1
}

# ---------------------------------------------------------------------------
# Pattern definitions
# ---------------------------------------------------------------------------
# Each pattern is a grep -E extended regex.  Patterns are intentionally
# conservative: prefer matching the exact variable names used for MAC/SSID
# identifiers rather than broad wildcards.
#
# Pattern 1 & 2: raw Mac Display/Debug in tracing field position.
#   Matches: candidate = %mac,   candidate = ?mac,
#            candidate = %candidate,  candidate = ?candidate,
# We anchor to the tracing named-field syntax: IDENTIFIER = [%?]VARNAME[,)].
# The [,)] after the variable name prevents false positives from longer names
# like %macros or %macaddr ("%macros," does not match because "ros," ≠ [,)]).
#
# KNOWN LIMITATION: this pattern does NOT catch the tracing positional-shorthand
# form `tracing::warn!(%mac, "msg")` where the Mac value is unnamed.  The
# codebase currently uses only named fields, so this is not a live gap, but
# reviewers adding new logging should be aware.
MAC_PATTERN='=[[:space:]]*[%?](mac|candidate)[,)]'

# Pattern 3: raw SSID structured-field in a tracing call, i.e. a field named
# "ssid" whose value is NOT routed through the redaction helper.
# The tracing structured-field syntax always ends the field assignment with a
# comma: `ssid = <expr>,`.  We match exactly that form.
#
# The exclusion set keeps false positives out:
#   - Lines containing "let "         → variable bindings (let ssid = ...)
#   - Lines containing ".ssid"        → struct-field access (session.ssid = ...)
#   - Lines containing "per_ssid"     → module-path references
#   - Lines containing "//"           → comments
#   - Lines containing "#["           → attribute macros
#   - Lines containing "redaction"    → already routed through the helper
#
# The pattern `ssid[[:space:]]*=[[:space:]]*[^;{]*,` requires the value to
# be terminated with a comma (not a semicolon or brace), which selects only
# tracing structured-field syntax and rejects plain Rust assignments.
SSID_FIELD_PATTERN='ssid[[:space:]]*=[[:space:]]*[^;{]*,'
SSID_REDACTED_MARKER='redaction'
# Exclusions as a grep -vE pattern (pipe-separated alternation):
SSID_FP_EXCLUSIONS='let |\.ssid|per_ssid|//|#\['

# ---------------------------------------------------------------------------
# Scan
# ---------------------------------------------------------------------------
violations=0

# --- Pass 1: MAC / candidate patterns ---------------------------------------
while IFS=: read -r filepath lineno linetext; do
    # filepath from grep is absolute when we pass an absolute glob; normalise
    # to repo-relative for allowlist matching and error messages.
    relpath="${filepath#"$REPO_ROOT/"}"

    # Trim leading whitespace from linetext for allowlist snippet comparison.
    trimmed=$(printf '%s' "$linetext" | sed 's/^[[:space:]]*//')

    if is_allowlisted "$relpath" "$trimmed"; then
        continue
    fi

    printf 'FAIL: raw identifier in tracing call — %s:%s\n' "$relpath" "$lineno" >&2
    printf '      line: %s\n' "$trimmed" >&2
    printf '      Fix:  route the value through crate::redaction::mac() (or the\n' >&2
    printf '            appropriate crate::redaction::{ssid,hostname,identity} helper).\n' >&2
    violations=$((violations + 1))
done << EOF
$(grep -rEn "$MAC_PATTERN" "$SRC_DIR" 2>/dev/null || true)
EOF

# --- Pass 2: raw SSID structured-field -------------------------------------
while IFS=: read -r filepath lineno linetext; do
    relpath="${filepath#"$REPO_ROOT/"}"
    trimmed=$(printf '%s' "$linetext" | sed 's/^[[:space:]]*//')

    if is_allowlisted "$relpath" "$trimmed"; then
        continue
    fi

    printf 'FAIL: raw SSID in tracing call — %s:%s\n' "$relpath" "$lineno" >&2
    printf '      line: %s\n' "$trimmed" >&2
    printf '      Fix:  route the value through crate::redaction::ssid().\n' >&2
    violations=$((violations + 1))
done << EOF
$(grep -rEn "$SSID_FIELD_PATTERN" "$SRC_DIR" 2>/dev/null \
    | grep -v "$SSID_REDACTED_MARKER" \
    | grep -vE "$SSID_FP_EXCLUSIONS" \
    || true)
EOF

# ---------------------------------------------------------------------------
# Result
# ---------------------------------------------------------------------------
if [ "$violations" -gt 0 ]; then
    printf '\ncheck-log-redaction: %d violation(s) found.\n' "$violations" >&2
    printf 'Add crate::redaction::* wrappers (added by the log-redaction core PR).\n' >&2
    printf 'If this is a pre-existing site being fixed by a pending PR, add it to\n' >&2
    printf 'scripts/log-redaction-allowlist.txt.\n' >&2
    exit 1
fi

printf 'check-log-redaction: OK — no raw-identifier tracing violations.\n'
exit 0
