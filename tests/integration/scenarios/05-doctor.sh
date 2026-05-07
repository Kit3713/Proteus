#!/bin/sh
# 05-doctor.sh — `proteus doctor --json` produces valid JSON with the
# expected check categories.
#
# Doctor's JSON shape is part of the public contract (a future GUI or
# CI plugin will key on it). This scenario asserts the structural
# skeleton: schema_version, top-level fields, and the four baseline
# check categories from src/commands/doctor.rs.
#
# We can't assume `jq` is installed in the container (it isn't in the
# default image), so we use `python3 -m json.tool` for the parse check
# (python3 is a Fedora 43 default) and grep for category strings.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 05-doctor\n'

OUT=$(proteus doctor --json 2>/dev/null) || {
    fail "proteus doctor --json exited non-zero"
    finish 05-doctor
    exit 1
}

# 1. JSON parses cleanly. python3 is part of the Fedora 43 base image.
if printf '%s' "$OUT" | python3 -m json.tool >/dev/null 2>&1; then
    ok "doctor JSON parses"
else
    fail "doctor JSON did not parse via python3 -m json.tool"
    printf '%s\n' "$OUT" | head -20 >&2
fi

# 2. Top-level fields. We don't pull in jq, so substring checks are
# enough — the field names are unambiguous and serde never reorders.
for field in '"schema_version"' '"proteus_version"' '"phase"' '"checks"' '"summary"'; do
    if printf '%s' "$OUT" | grep -qF -- "$field"; then
        ok "doctor JSON contains $field"
    else
        fail "doctor JSON missing top-level field $field"
    fi
done

# 3. Expected check categories. From src/commands/doctor.rs the four
# baseline categories are: system, daemons, files, runtime. The
# extended-regex grep handles both `"category":"foo"` and
# `"category": "foo"` forms (serde_json compact vs pretty).
for cat in system daemons files runtime; do
    if printf '%s' "$OUT" | grep -Eq "\"category\":[[:space:]]*\"$cat\""; then
        ok "doctor includes \"$cat\" checks"
    else
        fail "doctor JSON missing category \"$cat\""
    fi
done

# 4. Inside this container we run with systemd as PID 1, so the systemd
# check should be present (status content depends on container state).
if printf '%s' "$OUT" | grep -Eq '"name":[[:space:]]*"systemd"'; then
    ok "doctor includes the systemd check"
else
    fail "doctor missing the systemd check entry"
fi

finish 05-doctor
