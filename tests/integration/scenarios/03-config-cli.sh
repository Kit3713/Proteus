#!/bin/sh
# 03-config-cli.sh — read-only config CLI works.
#
# `proteus config show / keys / validate / get` must all work without
# root and without --yes. They read /etc/proteus/config.toml (or fall
# back to defaults if it doesn't exist), so they need to handle the
# missing-file case cleanly.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 03-config-cli\n'

expect_zero "proteus config show"          proteus config show
expect_zero "proteus config show --json"   proteus config show --json
expect_zero "proteus config keys"          proteus config keys
expect_zero "proteus config keys --json"   proteus config keys --json
expect_zero "proteus config validate"      proteus config validate

# `mac.enabled` is a default-true bool that round-trips through the
# defaults loader even without a config file on disk.
expect_zero "proteus config get mac.enabled"        proteus config get mac.enabled
expect_zero "proteus config get mac.enabled --json" proteus config get mac.enabled --json

finish 03-config-cli
