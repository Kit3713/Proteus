#!/bin/sh
# Verify a published Proteus release binary against a local rebuild.
#
# Goal: a security-relevant tool needs an audit trail that lets users confirm
# the binary they downloaded was built from the public source. This script
# clones the repo at a tagged release, rebuilds the x86_64 binary with the
# pinned toolchain and SOURCE_DATE_EPOCH, and compares its sha256 against the
# published `.sha256` file from the GitHub release.
#
# Usage:
#   scripts/verify-build.sh <tag>            # e.g. v0.1.0-alpha
#   scripts/verify-build.sh <tag> <workdir>  # custom temp dir (kept on exit)
#
# Exit codes:
#   0  binary sha256 matches the published one
#   1  mismatch (binary differs)
#   2  usage error or missing prerequisite
#   3  download / build / checkout failure
#
# POSIX shell (no bashisms). Prerequisites: git, curl, sha256sum, cargo +
# rustup so the pinned toolchain can be resolved from rust-toolchain.toml.
# No new crate dependencies; verification is a stand-alone script.
#
# Caveats (documented in wiki/reproducible-builds.md):
#   - Verifies only the binary, not the embedded wiki content per page (the
#     wiki bytes are part of the binary, so any change there changes the
#     hash; we just don't expose a per-page diff).
#   - The release artifact is built inside the fedora:43 container with a
#     specific glibc; rebuilding on a host with a different glibc will
#     produce a different sha256. Use the provided container or match the
#     environment described in the release's `*.build-info` file.
#   - The script does not verify GitHub release signatures; that is a
#     separate concern (see wiki/security-checklist.md).

set -eu

REPO_URL_DEFAULT="https://github.com/Kit3713/Proteus.git"
RELEASE_BASE_DEFAULT="https://github.com/Kit3713/Proteus/releases/download"
ARTIFACT_NAME="proteus-x86_64-unknown-linux-gnu"

usage() {
    cat <<EOF
Usage: scripts/verify-build.sh <tag> [workdir]

Rebuilds Proteus from the given tag with the pinned toolchain and
deterministic settings, then compares the sha256 of the resulting binary
against the published .sha256 from the matching GitHub release.

Arguments:
  tag       Release tag to verify, e.g. v0.1.0-alpha
  workdir   Optional working directory (defaults to a fresh mktemp dir
            that is removed on success)

Environment overrides:
  PROTEUS_REPO_URL      git URL to clone (default: ${REPO_URL_DEFAULT})
  PROTEUS_RELEASE_BASE  release asset base URL
                        (default: ${RELEASE_BASE_DEFAULT})
  PROTEUS_KEEP_WORKDIR  set to 1 to keep workdir even on success

Exit codes:
  0  match
  1  mismatch
  2  usage / prerequisite error
  3  download / build error
EOF
}

die() {
    rc=$1
    shift
    printf 'verify-build: %s\n' "$*" >&2
    exit "$rc"
}

require_cmd() {
    cmd=$1
    if ! command -v "$cmd" >/dev/null 2>&1; then
        die 2 "missing required command: $cmd"
    fi
}

# -- argument parsing -------------------------------------------------------

case "${1:-}" in
    -h|--help)
        usage
        exit 0
        ;;
    "")
        usage >&2
        exit 2
        ;;
esac

TAG=$1
WORKDIR=${2:-}

REPO_URL=${PROTEUS_REPO_URL:-$REPO_URL_DEFAULT}
RELEASE_BASE=${PROTEUS_RELEASE_BASE:-$RELEASE_BASE_DEFAULT}

# -- prerequisite check -----------------------------------------------------

require_cmd git
require_cmd curl
require_cmd sha256sum
require_cmd cargo
# rustup is used implicitly: Cargo will install the toolchain pinned in
# rust-toolchain.toml on first invocation. If rustup is missing the user
# probably has a system-rustc; warn but do not fail.
if ! command -v rustup >/dev/null 2>&1; then
    printf 'verify-build: warn: rustup not found; pinned toolchain will not auto-install\n' >&2
fi

# -- workdir setup ----------------------------------------------------------

CLEANUP_WORKDIR=0
if [ -z "$WORKDIR" ]; then
    WORKDIR=$(mktemp -d -t proteus-verify-XXXXXX)
    if [ "${PROTEUS_KEEP_WORKDIR:-0}" -ne 1 ]; then
        CLEANUP_WORKDIR=1
    fi
fi

cleanup() {
    rc=$?
    if [ "$CLEANUP_WORKDIR" -eq 1 ] && [ "$rc" -eq 0 ]; then
        rm -rf "$WORKDIR"
    elif [ "$CLEANUP_WORKDIR" -eq 1 ]; then
        printf 'verify-build: keeping workdir for inspection: %s\n' "$WORKDIR" >&2
    fi
}
trap cleanup EXIT

mkdir -p "$WORKDIR"
cd "$WORKDIR"

printf 'verify-build: tag=%s workdir=%s\n' "$TAG" "$WORKDIR"

# -- clone source at tag ----------------------------------------------------

if [ ! -d src/.git ]; then
    printf 'verify-build: cloning %s at %s\n' "$REPO_URL" "$TAG"
    # --depth 1 + --branch refs are enough; we only need the tagged tree
    # plus enough history to read the tag commit timestamp.
    git clone --depth 1 --branch "$TAG" "$REPO_URL" src \
        || die 3 "git clone failed for tag $TAG"
else
    printf 'verify-build: reusing existing src/ checkout\n'
fi

cd src

# -- compute SOURCE_DATE_EPOCH ---------------------------------------------

SDE=$(git log -1 --pretty=%ct)
if [ -z "$SDE" ]; then
    die 3 "could not read tag commit timestamp"
fi
SOURCE_DATE_EPOCH=$SDE
export SOURCE_DATE_EPOCH
printf 'verify-build: SOURCE_DATE_EPOCH=%s\n' "$SOURCE_DATE_EPOCH"

# -- record toolchain identity ---------------------------------------------

printf 'verify-build: rustc=%s\n' "$(rustc --version 2>/dev/null || echo unknown)"
printf 'verify-build: cargo=%s\n' "$(cargo --version 2>/dev/null || echo unknown)"

# -- build with pinned, locked, frozen settings -----------------------------

printf 'verify-build: cargo build --release --frozen --locked\n'
if ! cargo build --release --frozen --locked; then
    die 3 "cargo build failed"
fi

BIN=target/release/proteus
if [ ! -f "$BIN" ]; then
    die 3 "expected binary not found at $BIN"
fi

# Strip the binary; the published artifact is stripped, so an apples-to-
# apples comparison strips locally too.
if command -v strip >/dev/null 2>&1; then
    strip "$BIN" || die 3 "strip failed on $BIN"
else
    printf 'verify-build: warn: strip not found; comparison may differ\n' >&2
fi

LOCAL_SHA=$(sha256sum "$BIN" | awk '{print $1}')
printf 'verify-build: local  sha256=%s\n' "$LOCAL_SHA"

# -- fetch published sha256 ------------------------------------------------

cd "$WORKDIR"
PUB_SHA_URL="${RELEASE_BASE}/${TAG}/${ARTIFACT_NAME}.sha256"
printf 'verify-build: fetching %s\n' "$PUB_SHA_URL"
if ! curl -fsSL -o published.sha256 "$PUB_SHA_URL"; then
    die 3 "could not fetch published sha256 from $PUB_SHA_URL"
fi

# `sha256sum -c` format is "<hash>  <filename>"; we just want the hash.
PUBLISHED_SHA=$(awk '{print $1}' published.sha256)
if [ -z "$PUBLISHED_SHA" ]; then
    die 3 "published.sha256 was empty or malformed"
fi
printf 'verify-build: remote sha256=%s\n' "$PUBLISHED_SHA"

# -- compare ---------------------------------------------------------------

if [ "$LOCAL_SHA" = "$PUBLISHED_SHA" ]; then
    printf '\nverify-build: MATCH — locally rebuilt binary equals published artifact for %s\n' "$TAG"
    exit 0
fi

printf '\nverify-build: MISMATCH for %s\n' "$TAG" >&2
printf '  local : %s\n' "$LOCAL_SHA" >&2
printf '  remote: %s\n' "$PUBLISHED_SHA" >&2
printf '\nThis can happen if:\n' >&2
printf '  - your host glibc differs from the release container (fedora:43)\n' >&2
printf '  - your toolchain channel does not match rust-toolchain.toml\n' >&2
printf '  - the release was built with a different SOURCE_DATE_EPOCH\n' >&2
printf 'See wiki/reproducible-builds.md for the recipe to match the release env.\n' >&2
exit 1
