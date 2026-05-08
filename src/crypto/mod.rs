// SPDX-License-Identifier: GPL-3.0-or-later

//! In-tree crypto primitives Proteus needs to keep its zero-deps stance.
//!
//! Today this is just SHA-256. The functions here are used to stamp a
//! `# sha256:<hex>` header onto every Proteus-managed file (drop-ins,
//! systemd units, sysctl files) so `proteus diff` can spot manual edits
//! and other-tool drift. The hash is a tamper-hint primitive, not an
//! integrity guarantee — header and body live in the same root-owned
//! file, so anything with write access can rewrite the header to match
//! a tampered body.
//!
//! Pure stdlib. No `sha2` / `digest` / RustCrypto dependency.

pub mod sha256;
