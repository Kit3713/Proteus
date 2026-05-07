<!-- See [CONTRIBUTING.md](../CONTRIBUTING.md) for the full quality bar. Keep this terse. -->

## Summary

<!-- What changed and why. 1-3 sentences. -->

## How to verify

<!-- Concrete steps a reviewer can run. Distinguish "I tested by..." from "you can verify by...". -->

## Checklist

- [ ] `cargo fmt` clean
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test` passes
- [ ] Binary stays under 3 MB stripped (run `strip target/release/proteus && wc -c < target/release/proteus`)
- [ ] If this adds a new feature flag or knob: wiki page added in `wiki/` AND `proteus help <feature>` text wired up
- [ ] If this adds a new error path: error message points at a wiki page or `proteus help <feature>` where applicable
- [ ] If this touches privileged operations: integration test added (or noted why not, e.g., "phase A has no privileged ops")
- [ ] `proteus revert` still rolls back cleanly (or "n/a — phase A has no mutators")
- [ ] Any new dependency >200 KB justified in this PR description

## Scope check

- [ ] This change fits the local controllable fingerprint reduction scope — L2 through L4 network identifiers, network-joining protocols, or the OS-controllable parts of the L1 RF surface (see [CONTRIBUTING.md](../CONTRIBUTING.md))
- [ ] This change does NOT weaken Fedora's `crypto-policies`, touch `/etc/ssh/ssh_config`, or rotate `/etc/machine-id` (or if it does, it's opt-in, default off, and documented in a wiki page with concrete failure modes)

## Linked issue

<!-- Closes #N — if applicable. -->
