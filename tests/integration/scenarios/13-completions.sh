#!/bin/sh
# 13-completions.sh — `proteus completions <shell>` (Roadmap Stream 1
# / CL4, Milestone 6 ergonomics).
#
# `completions` prints the embedded shell-completion script for one of
# three supported shells. The contract:
#   - bash / zsh / fish all exit 0 with non-empty output.
#   - An unknown shell exits CONFIG_ERROR (65) with a friendly
#     "unknown shell '...'; supported: bash, zsh, fish" message.
#   - The output for each shell contains a stable header line so a
#     `--shell mismatch` regression (e.g. fish header under `zsh`) trips.

set -u
. "$(dirname "$0")/lib.sh"
FAILED=0

printf 'scenario: 13-completions\n'

# Each supported shell prints a non-empty script with the expected
# leader. The header strings live in dist/completions/*.{bash,zsh,fish}.
expect_zero "proteus completions bash" proteus completions bash
expect_zero "proteus completions zsh"  proteus completions zsh
expect_zero "proteus completions fish" proteus completions fish

# Header sanity checks. bash + fish use a `# proteus ... completion`
# leader; zsh uses `#compdef proteus`. Catches a shell-swap regression.
expect_contains "bash completion header" "bash completion" \
    proteus completions bash
expect_contains "zsh compdef line" "#compdef proteus" \
    proteus completions zsh
expect_contains "fish completion header" "fish completion" \
    proteus completions fish

# Invalid shell — error code is CONFIG_ERROR (65).
expect_rc_in "proteus completions invalid-shell" 65 -- \
    proteus completions invalid-shell

# Missing required arg trips clap (rc=2).
expect_rc_in "proteus completions (no shell)" 2 -- proteus completions

finish 13-completions
