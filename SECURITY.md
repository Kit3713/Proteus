# Security Policy

Proteus erases network-layer identifiers. A vulnerability in Proteus is a real privacy harm, not a cosmetic bug. Reports are taken seriously.

## Supported versions

Beta. The latest pre-release on `main` is the only supported line. Fix the bug there and the fix lands for everyone.

Once v1.0.0 ships, the latest minor of the latest major is the only supported line. Older lines do not get backports.

| Version          | Supported               |
| ---------------- | ----------------------- |
| `main`           | yes (pre-release)       |
| `v0.4.x` (beta)  | yes (latest beta line)  |
| `< v0.4`         | no — upgrade to `v0.4`  |

## Reporting a vulnerability

Do not open a public issue or pull request. Both leak the bug to anyone watching the repo.

Two private channels, in order of preference:

1. GitHub's private security advisory form: <https://github.com/Kit3713/Proteus/security/advisories/new>. Preferred — it threads the discussion, lets a fix be drafted privately, and produces a CVE if one is warranted.
2. Email the maintainer. The address is in [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). Use this if GitHub is down or you need PGP.

Please include, where you can:

- Affected commit SHA or version
- Distro, kernel, NetworkManager / systemd / BlueZ versions
- A minimal reproduction — the exact `proteus` invocation, relevant config, observable symptom
- What you expected vs. what happened
- Whether the bug is exploitable without root, or only by a local root user

A proof-of-concept is welcome but not required.

## Disclosure timeline

- Acknowledgment within 7 days of the report landing.
- Triage and severity within 14 days.
- Target fix landed for critical and high within 90 days. Medium and low are best-effort.
- Coordinated disclosure preferred. The reporter and the maintainer agree a public-disclosure date once a fix or mitigation is ready.

If the maintainer goes quiet for more than 30 days after acknowledgment, you are free to disclose. Please say so first.

## In scope

Anything that causes Proteus to:

- Leak the original MAC, hostname, DUID, Bluetooth name, or any other identifier it was meant to erase or rotate.
- Corrupt a working network setup — leaves NetworkManager, systemd-resolved, BlueZ, firewalld, or nftables in a broken state that the user did not opt into and `proteus revert` cannot undo.
- Weaken Fedora's hardening — touches `crypto-policies`, `/etc/ssh/ssh_config`, or `/etc/machine-id`. These are explicit non-goals (see [`docs/PLAN.md`](docs/PLAN.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md)); a regression is a security bug.
- Silently fail — reports `applied` in `proteus status` for a feature that did not actually take effect on the live system.
- Break the captive-portal or probe logic in a way that loops MAC rotation behind a portal or against a transient outage.
- Ship a vulnerable dependency. Supply-chain reports against the published `Cargo.lock` are in scope; please name the advisory or CVE.

Reports of new attack classes against the locally controllable fingerprint surface — L2 through L4 network identifiers, network-joining protocols (DHCP, mDNS, LLMNR, NetBIOS, SSDP, WSD, NTP, IPv6 derivations, ICMP, captive-portal exchanges), and the OS-controllable RF surface (TX power, probe behavior, scan policy) — are in scope even if Proteus does not yet defend against them — they inform the threat model.

## Out of scope

These are deliberate non-goals. The [README](README.md), [`docs/PLAN.md`](docs/PLAN.md), and [`CONTRIBUTING.md`](CONTRIBUTING.md) explain why and which dedicated tool to reach for instead.

- TLS, browser, or SSH client fingerprinting — application-protocol scope. Use Tor Browser, librewolf, or your own `ssh_config`.
- DNS resolution policy beyond the one ECS-strip knob — use dnscrypt-proxy, NextDNS, AdGuard Home, or Pi-hole.
- Tracker or ad blocking — Pi-hole, NextDNS, uBlock Origin.
- Traffic correlation — Tor, Mullvad VPN.
- L1 RF fingerprinting (analog transmitter characteristics) — software cannot fix this. A swappable USB Wi-Fi adapter is the real answer.
- Bugs in NetworkManager, systemd, BlueZ, firewalld, or the kernel themselves — please report upstream. Proteus may add a workaround, but the fix belongs there.
- Local root attackers. Root can already do anything to a Linux system; defending against it is not a goal.
- Findings that require physical access plus an unlocked session.
- Denial of service from a malicious local user against their own machine.
- Theoretical issues without a reproduction or affected version.

## Safe harbor

Good-faith security research conducted under this policy is welcome. The maintainer will not pursue or support legal action against researchers who:

- Make a good-faith effort to avoid privacy violations, data loss, or service disruption.
- Only interact with systems and accounts they own or have explicit permission to test.
- Report through the private channels above and give a reasonable window before public disclosure.
- Do not exfiltrate data beyond what is necessary to demonstrate the issue.

Proteus is licensed under GPL-3.0-or-later. Nothing in this policy overrides the license — including the disclaimer of warranty.

## Credit

Reporters are credited in the advisory and the changelog unless they ask not to be. Anonymous reports are accepted.
