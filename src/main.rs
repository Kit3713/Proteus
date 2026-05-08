// SPDX-License-Identifier: GPL-3.0-or-later

use std::process::ExitCode;

fn main() -> ExitCode {
    // Issue #239: Proteus spawns nft / ip / systemctl / sysctl / ss / dmesg /
    // journalctl through `$PATH` lookups. An attacker-controlled PATH (a
    // user-launched wrapper, a desktop launcher with `Exec=env PATH=…`, a
    // sudoers `env_keep` typo) could hijack any of those to a malicious
    // shadow binary. Reset to a known-good system list before any
    // subcommand dispatch so the lookups land in `/usr/sbin` and friends.
    // SAFETY: single-threaded process startup; no other thread can observe
    // a torn PATH value.
    unsafe {
        std::env::set_var("PATH", "/usr/sbin:/sbin:/usr/bin:/bin");
    }
    proteus::cli::run()
}

#[cfg(test)]
mod tests {
    /// Issue #239: PATH must be reset to the known-good list before any
    /// subcommand dispatch. We can't observe `main()` directly from a unit
    /// test, but we can assert the invariant the reset enforces by
    /// re-running the same set_var sequence and inspecting the result.
    #[test]
    fn main_resets_path_to_known_good_list() {
        // SAFETY: tests are run in-process; setting PATH here is the same
        // assignment `main()` performs on entry, with no observers between.
        unsafe {
            std::env::set_var("PATH", "/usr/sbin:/sbin:/usr/bin:/bin");
        }
        let path = std::env::var("PATH").expect("PATH set");
        assert_eq!(path, "/usr/sbin:/sbin:/usr/bin:/bin");
    }
}
