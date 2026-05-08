# Contributing to Proteus

Thanks for considering. Proteus is a small, focused project — see status in the [README](README.md). The major build-out is landed; v0.4 is bug-and-vulnerability-hunt only.

## What helps right now

- **Real-world testing** — `proteus doctor` + `proteus apply` on coffee-shop / hotel / conference / airport networks; report bugs via the issue template (highest-value contribution today).
- **Independent security review** — eyes on [`wiki/threat-model.md`](wiki/threat-model.md) and the DBus surface enumerated in [`docs/security/dbus-surface.md`](docs/security/dbus-surface.md).
- **Persona contributions** — `data/personas/*.toml` is open for community PRs to grow the catalogue. See [`wiki/personas.md`](wiki/personas.md) for the schema.
- **Distro packaging** — Alpine, Void, Gentoo packagers, plus AUR / Copr / Debian unstable submission sponsors needed. Recipes live under `dist/`.
- **Threat-model improvements** — what's overlooked, what's overclaimed.
- **Wiki polish** — voice should match [`wiki/intro.md`](wiki/intro.md).
- File a feature suggestion only if it fits the local controllable fingerprint reduction scope. [`docs/PLAN.md`](docs/PLAN.md) and [`docs/PRIOR-ART.md`](docs/PRIOR-ART.md) explain what's in and what's deliberately out (DNS resolution policy, TLS/browser fingerprints, tracker blocking, etc. — all delegated to dedicated tools).

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

- Rust stable, MSRV 1.85 (Edition 2024). The repo pins `1.93.0` via `rust-toolchain.toml`.
- A Linux dev host with systemd, NetworkManager, and BlueZ — Fedora 43+ is the primary target.

```sh
git clone https://github.com/Kit3713/Proteus.git
cd Proteus
cargo build --release --locked
cargo test --locked
```

Privileged integration tests run in a Podman + systemd container and are gated behind `RUN_PRIVILEGED_TESTS=1`. See [`tests/integration/`](tests/integration/) and the integration scenarios under `tests/integration/scenarios/`.

## Style and quality bar

- `cargo fmt --check` clean
- `cargo clippy --all-targets --locked -- -D warnings` clean
- `cargo test --locked` passes (the wiki-bundling test verifies every embedded page parses)
- Binary stays under the release-time cap in `.github/workflows/release.yml` — any dependency that adds more than 200 KB needs justification in the PR
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
