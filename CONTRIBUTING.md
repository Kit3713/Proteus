# Contributing to Proteus

Thanks for considering. Proteus is a small, focused project — see status in the [README](README.md) — so the most useful contributions vary by phase.

## What helps right now

The project is pre-release. The v1 plan is in [`docs/PLAN.md`](docs/PLAN.md). The most useful contributions today:

- Read the plan and open an issue or discussion if a phase looks wrong-shaped, missing, or scope-creeping.
- Suggest concrete improvements to the threat model — what's overlooked, what's overclaimed.
- File a feature suggestion only if it fits the network-layer fingerprint eraser scope. The plan and [`docs/PRIOR-ART.md`](docs/PRIOR-ART.md) explain what's in and what's deliberately out (DNS resolution policy, TLS/browser fingerprints, tracker blocking, etc. — all delegated to dedicated tools).

Code contributions are welcome once Phase A (the skeleton) lands.

## Scope

Proteus is a network-layer fingerprint eraser. Features that fit:

- Anything that rotates or scrubs an identifier broadcast at L1–L4 or in network-joining protocols (DHCP, mDNS, LLMNR, NetBIOS, SSDP, WSD, WPAD, NTP, captive-portal exchanges)
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
- Binary stays under 3.75 MB stripped (release-time hard cap in `release.yml`) — any dependency that adds more than 200 KB needs justification in the PR
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
