// SPDX-License-Identifier: GPL-3.0-or-later

//! `proteus completions <shell>` — emit one of the bundled completion
//! scripts on stdout.
//!
//! Roadmap Milestone 6: CLI ergonomics. The completion files under
//! `dist/completions/` are hand-written for stability and small size; this
//! command embeds them via `include_str!` so a user can install without
//! sudo or repo access:
//!
//! ```sh
//! proteus completions bash > ~/.local/share/bash-completion/completions/proteus
//! ```
//!
//! Three knobs only — bash, zsh, fish — matching what the install scripts
//! deploy. PowerShell / nushell are explicitly out of scope.

use anyhow::Result;

use crate::exit;

const BASH: &str = include_str!("../../dist/completions/proteus.bash");
const ZSH: &str = include_str!("../../dist/completions/proteus.zsh");
const FISH: &str = include_str!("../../dist/completions/proteus.fish");

pub fn run(shell: &str) -> Result<u8> {
    let body = match shell.to_ascii_lowercase().as_str() {
        "bash" => BASH,
        "zsh" => ZSH,
        "fish" => FISH,
        other => {
            eprintln!("proteus: unknown shell '{other}'; supported: bash, zsh, fish");
            return Ok(exit::CONFIG_ERROR);
        }
    };
    print!("{body}");
    Ok(exit::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_supported_shell_resolves() {
        for shell in ["bash", "zsh", "fish"] {
            // We don't capture stdout here; just confirm the resolution path
            // returns SUCCESS rather than CONFIG_ERROR.
            assert_eq!(run(shell).unwrap(), exit::SUCCESS);
        }
    }

    #[test]
    fn case_insensitive_shell_match() {
        assert_eq!(run("BASH").unwrap(), exit::SUCCESS);
        assert_eq!(run("Zsh").unwrap(), exit::SUCCESS);
    }

    #[test]
    fn unknown_shell_returns_config_error() {
        assert_eq!(run("powershell").unwrap(), exit::CONFIG_ERROR);
        assert_eq!(run("").unwrap(), exit::CONFIG_ERROR);
    }

    #[test]
    fn embedded_scripts_are_non_empty() {
        // Issue #206-G-style guard: `include_str!` would panic at compile
        // time if the path were wrong, but a 0-byte file would still
        // succeed. Catch the trivial-content regression early.
        assert!(BASH.len() > 100);
        assert!(ZSH.len() > 100);
        assert!(FISH.len() > 100);
    }

    #[test]
    fn bash_completion_has_proteus_marker() {
        // Sanity: the bundled bash script should reference `proteus` as the
        // command. Defends against a future copy-paste error that ships an
        // empty stub or someone else's completion.
        assert!(BASH.contains("proteus"));
        assert!(ZSH.contains("proteus"));
        assert!(FISH.contains("proteus"));
    }

    /// Assert every needle appears in all three bundled completion scripts.
    fn assert_bundled_scripts_contain(kind: &str, needles: &[&str]) {
        for needle in needles {
            assert!(
                BASH.contains(needle),
                "bash completion missing {kind} '{needle}'"
            );
            assert!(
                ZSH.contains(needle),
                "zsh completion missing {kind} '{needle}'"
            );
            assert!(
                FISH.contains(needle),
                "fish completion missing {kind} '{needle}'"
            );
        }
    }

    /// Regression guard for issues #285 and #291 — the bundled scripts
    /// drifted to the v0.1.0 surface and missed roughly half of `Command`.
    /// The list is a representative cross-section, not an enumeration:
    /// adding a new subcommand should not force a test edit unless that
    /// subcommand is one of the top-level feature areas.
    #[test]
    fn bundled_scripts_cover_current_subcommand_surface() {
        assert_bundled_scripts_contain(
            "subcommand",
            &[
                "session",
                "doctor",
                "probe",
                "pin",
                "unpin",
                "kill",
                "resume",
                "nft",
                "portal",
                "rf",
                "persona",
                "ssid",
                "events",
                "timer",
                "bluetooth",
                "dhcp",
                "ipv6",
                "hostname",
                "wiki",
                "completions",
                "rotate-if-needed",
            ],
        );
    }

    /// Wiki page names are hard-coded into each completion script so TAB
    /// works without running `proteus wiki list`. Catch drift between
    /// `wiki/*.md` and the bundled lists at test time.
    #[test]
    fn bundled_scripts_cover_current_wiki_pages() {
        assert_bundled_scripts_contain(
            "wiki page",
            &[
                "intro",
                "quickstart",
                "concepts",
                "mac-recipes",
                "rf-fingerprinting",
                "personas",
                "per-ssid",
                "doctor",
                "kill-switch",
                "timer",
                "profiles",
                "discovery",
                "stack-fingerprint",
                "captive-portals",
                "dhcp",
                "ipv6",
                "hostname-recipes",
                "enterprise-wifi",
                "dns",
                "threat-model",
                "internals",
                "troubleshooting",
                "faq",
                "glossary",
            ],
        );
    }
}
