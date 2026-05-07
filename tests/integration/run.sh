#!/bin/sh
# Proteus integration-test driver.
#
# Builds proteus on the host, builds a privileged Fedora 43 container
# with systemd as PID 1 (see tests/integration/Containerfile), then
# `podman exec`s each scenario script inside the running container.
#
# Idempotent: re-runnable without manual cleanup. We always remove any
# stale `proteus-it` container before starting and remove ours on exit.
#
# Constraints:
#   - POSIX shell (no bashisms; tested under dash).
#   - Tests don't require root on the dev host (podman --privileged
#     handles the in-container privilege).
#   - No new Rust dependencies — we just `cargo build --release`.
#
# Exit code:
#   0 if every scenario passes.
#   1 if any scenario fails.
#
# PLAN.md phase G: "Integration tests in a privileged Podman + systemd
# container with stubbed NM and BlueZ."

set -eu

# Anchor at repo root so the script works regardless of cwd.
SCRIPT_DIR=$(cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)
cd "$REPO_ROOT"

CONTAINER_NAME="proteus-it"
IMAGE_TAG="proteus-it"
CONTAINERFILE="tests/integration/Containerfile"
BIN_DROP="tests/integration/proteus.bin"
SCENARIO_DIR="tests/integration/scenarios"

log() { printf '[run.sh] %s\n' "$*"; }
err() { printf '[run.sh] ERROR: %s\n' "$*" >&2; }

PASSED=0
FAILED=0
FAIL_NAMES=""

# Always remove our container on exit, success or failure. Stale state
# breaks idempotency and the next run.
cleanup() {
    rc=$?
    log "cleanup: removing container ${CONTAINER_NAME} (best effort)"
    podman rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true
    rm -f "${BIN_DROP}" || true
    exit "$rc"
}
trap cleanup EXIT INT TERM

# Run a `podman exec proteus-it ...` smoke and tally PASS/FAIL. A smoke
# passes if its rc is < ${max_pass_rc} (defaults to 64). Used for
# scenario scripts (rc=0 only) and orchestrator commands (where rc=1
# may be expected when components skip due to missing hardware).
#
# Args: <name> <max_pass_rc> -- <cmd> [args...]
run_in_container() {
    name=$1
    max_pass_rc=$2
    shift 2
    [ "$1" = "--" ] && shift
    log "=== ${name} ==="
    set +e
    podman exec "${CONTAINER_NAME}" "$@"
    rc=$?
    set -e
    if [ "$rc" -lt "$max_pass_rc" ]; then
        log "PASS: ${name} (rc=${rc})"
        PASSED=$((PASSED + 1))
    else
        err "FAIL: ${name} (rc=${rc}, expected < ${max_pass_rc})"
        FAILED=$((FAILED + 1))
        FAIL_NAMES="${FAIL_NAMES} ${name}"
    fi
}

# --- 1. Preconditions ------------------------------------------------------
if ! command -v podman >/dev/null 2>&1; then
    err "podman is not installed; cannot run integration tests"
    err "install with: sudo dnf install podman   (Fedora)"
    err "             or: sudo apt-get install podman   (Debian/Ubuntu)"
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    err "cargo is not installed; cannot build proteus"
    exit 1
fi

# --- 2. Build proteus on the host -----------------------------------------
log "building proteus (release)"
cargo build --release

if [ ! -x target/release/proteus ]; then
    err "expected target/release/proteus to exist after build"
    exit 1
fi

# Stage the binary where the Containerfile's COPY can find it. We don't
# bind-mount because the binary needs to be inside the image so the
# image is a self-contained test artifact (CI cache, reproducibility).
cp -f target/release/proteus "${BIN_DROP}"
log "staged binary at ${BIN_DROP} ($(wc -c < "${BIN_DROP}") bytes)"

# --- 3. Remove any stale container, then build the image ------------------
log "removing any stale ${CONTAINER_NAME} container"
podman rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true

log "building image ${IMAGE_TAG}"
podman build -t "${IMAGE_TAG}" -f "${CONTAINERFILE}" .

# --- 4. Start the container with systemd as PID 1 -------------------------
log "starting container ${CONTAINER_NAME}"
# --privileged: needed for systemd to manage cgroups inside the container.
# --systemd=always: tell podman this image expects systemd as PID 1
#   (handles /sys/fs/cgroup mounting and tmpfs setup automatically).
podman run \
    --privileged \
    --systemd=always \
    -d \
    --name "${CONTAINER_NAME}" \
    "${IMAGE_TAG}" >/dev/null

# --- 5. Wait for systemd to come up ---------------------------------------
# `systemctl is-system-running` returns one of: initializing, starting,
# running, degraded, maintenance, stopping, offline. We accept anything
# other than initializing/starting/offline because a containerized
# systemd often ends up "degraded" (some services can't start without
# real hardware). Tests target individual commands, not full health.
log "waiting for systemd to come up inside container (timeout 60s)"
SYSTEMD_READY=0
i=0
while [ "$i" -lt 30 ]; do
    state=$(podman exec "${CONTAINER_NAME}" systemctl is-system-running 2>/dev/null || true)
    case "$state" in
        running|degraded|maintenance)
            SYSTEMD_READY=1
            log "systemd state: ${state}"
            break
            ;;
        *)
            i=$((i + 1))
            sleep 2
            ;;
    esac
done

if [ "$SYSTEMD_READY" -ne 1 ]; then
    err "systemd did not come up within 60s (last state: '${state:-unknown}')"
    err "container logs:"
    podman logs "${CONTAINER_NAME}" >&2 || true
    exit 1
fi

# --- 6. Run each scenario script -----------------------------------------
# Auto-discover scenarios in lexical order. Numeric prefix (01-, 02-, ...)
# controls execution order: read-only smokes run before mutators. We skip
# `lib.sh` because that's the shared helpers file, sourced by every
# scenario, not a runnable scenario itself.
for s in "${SCENARIO_DIR}"/*.sh; do
    [ -f "$s" ] || continue
    name=$(basename "$s")
    [ "$name" = "lib.sh" ] && continue
    # Scenario scripts must exit 0; anything non-zero is a failure.
    run_in_container "scenario: ${name}" 1 -- sh "/opt/proteus-tests/scenarios/${name}"
done

# --- 7. Orchestrator-level smokes ----------------------------------------
# apply will skip most components in the container (no Wi-Fi, no Bluetooth
# hardware). rc=0 means every component succeeded; rc=1 means at least one
# component failed (acceptable in this environment because the *path* ran
# end-to-end). rc>=64 indicates a parse/config/permission error and is a
# real regression. revert must always succeed cleanly. rotate against `lo`
# is expected to fail (not NM-managed) but must not crash with a code in
# the parse/permission range.
run_in_container "orchestrator: apply --yes"          64 -- proteus apply --yes
run_in_container "orchestrator: revert --yes"          1 -- proteus revert --yes
run_in_container "smoke: rotate --iface lo --yes"     64 -- proteus rotate --iface lo --yes

# --- 8. Summary -----------------------------------------------------------
log ""
log "==================="
log "scenarios: ${PASSED} passed, ${FAILED} failed"
if [ "$FAILED" -gt 0 ]; then
    err "failed scenarios:${FAIL_NAMES}"
    exit 1
fi
log "all scenarios passed"
exit 0
