# Contributing to Proteus

Thanks for considering. Proteus is a small, focused project — see
status in the [README](README.md) — so the most useful contributions
vary by cycle.

## What helps right now (v0.4 beta cycle)

The v0.3 alpha cycle closed at `v0.3.2-alpha` (83 ✅ / 1 💭). The
active cycle is **v0.4 beta — bug + vulnerability hunting only**. **No
new feature work lands until beta closes.** Feature proposals queue
under the `proposal` label for v0.5.

The two highest-leverage contributions today:

1. **Real-world testing.** Run `proteus doctor` + `proteus apply` on
   coffee shop / hotel / conference / airport networks. The `tests/realworld/`
   probe set captures everything triage needs; attach the output dump
   to a bug filed against the [`v0.4-beta-intake`](docs/BETA-INTAKE.md)
   process.
2. **Vulnerability hunting.** The bypass-hardening pass
   ([`docs/security/bypass-hardening-pass.md`](docs/security/bypass-hardening-pass.md))
   audited every shellout and every recent parser. Higher-leverage hunt
   areas the pass *did not* cover are listed in the "Hunt suggestions"
   section of [`docs/BETA-INTAKE.md`](docs/BETA-INTAKE.md): CLI parser
   fuzzing, state-migration replay, DBus argument validation, race
   conditions, unicode handling, FD-leak verification.

Read [`docs/BETA-INTAKE.md`](docs/BETA-INTAKE.md) for the intake form,
severity rubric, triage cadence, and explicit out-of-scope list before
filing.

Bug fix PRs are welcome — please pair them with a regression test that
locks the fix in place.

## Scope

Proteus reduces every fingerprint the local OS can control — L2 through L4 network identifiers, network-joining protocol chatter, and the OS-controllable parts of the L1 RF surface (TX power, probe behavior, scan policy, chipset inventory). Hardware-baked RF (oscillator drift, IQ imbalance) and identifiers owned by other tool layers (TLS, SSH, browser, DNS resolution policy) stay out. Features that fit:

- Anything that rotates or scrubs an identifier broadcast at L1–L4 or in network-joining protocols (DHCP, mDNS, LLMNR, NetBIOS, SSDP, WSD, WPAD, NTP, captive-portal exchanges)
- Anything in the OS-controllable RF surface (TX power, probe-request privacy, scan policy, chipset reporting)
- Anything that improves observability, reversibility, or recoverability of the above
- Anything that makes the CLI easier to wrap, the wiki easier to search, or the help text more honest

Features that don't fit (please open an issue elsewhere):

- TLS, browser, or SSH client fingerprinting — application-protocol scope
- DNS resolution policy beyond the one ECS-strip knob — use dnscrypt-proxy, NextDNS, AdGuard Home, Pi-hole
- Tracker blocking, ad blocking — Pi-hole, NextDNS, uBlock Origin
- Traffic correlation defenses — Tor, Mullvad VPN
- Anything that weakens Fedora's `crypto-policies`, touches `/etc/ssh/ssh_config`, or rotates `/etc/machine-id`

## Development setup

Requires:

- Rust stable (latest)
- A Linux dev host with systemd, NetworkManager, and BlueZ — Fedora 43+ is the primary target

Once Phase A is in:

```sh
git clone https://github.com/Kit3713/Proteus.git
cd Proteus
cargo build
cargo test
```

Privileged integration tests run in a Podman + systemd container and are gated behind `RUN_PRIVILEGED_TESTS=1`. Documented in `docs/PLAN.md` phase G.

## Style and quality bar

- `cargo fmt` clean
- `cargo clippy --all-targets -- -D warnings` clean
- Binary stays under 4 MB stripped (release-time hard cap in `release.yml`) — any dependency that adds more than 200 KB needs justification in the PR
- New feature flags ship with their wiki page and `proteus help <feature>` text in the same PR
- New error paths include a `→ see: proteus wiki <page>` or `→ run: proteus help <feature>` hint where applicable
- Anything that touches privileged operations is covered by an integration test
- `proteus revert` still rolls back cleanly

## Commit and PR conventions

- Small, focused commits
- Conventional commit prefixes (`feat:`, `fix:`, `docs:`, `chore:`) are fine but not required
- PR description: what changed, why, how to verify
- Link the issue if there is one

## Code of Conduct

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

Contributions are licensed under GPL-3.0-or-later, the same as the project. By submitting a PR you agree to license your contribution under those terms.
