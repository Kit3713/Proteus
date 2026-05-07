// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::{IsTerminal, Write};

use anyhow::Result;

use crate::exit;
use crate::wiki::{self, RenderStyle};

const NO_PAGES_NOTE: &str =
    "no pages embedded yet — `intro`, `quickstart`, `concepts` land in phase A alongside this PR";

pub fn run(page: Option<&str>, no_color: bool) -> Result<u8> {
    if let Some(name) = page {
        return print_page(name, no_color);
    }
    list_pages();
    Ok(exit::SUCCESS)
}

pub fn run_help(feature: Option<&str>, no_color: bool) -> Result<u8> {
    if let Some(name) = feature {
        return print_page(name, no_color);
    }
    println!("Usage: proteus help <feature>");
    println!();
    println!("Known wiki pages:");
    list_pages();
    Ok(exit::SUCCESS)
}

fn print_page(name: &str, no_color_flag: bool) -> Result<u8> {
    let Some(content) = wiki::get_page(name) else {
        eprintln!("proteus: no wiki page '{name}'");
        let pages = wiki::list_pages();
        if pages.is_empty() {
            eprintln!("  ({NO_PAGES_NOTE})");
        } else {
            eprintln!("  available pages:");
            for p in &pages {
                eprintln!("    {p}");
            }
        }
        return Ok(exit::GENERIC_ERROR);
    };
    let no_color_env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let style = pick_style(
        std::io::stdout().is_terminal(),
        no_color_flag || no_color_env,
    );
    let rendered = wiki::render(content, style);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    // Ignore broken pipe (e.g. piping into `head`).
    let _ = handle.write_all(rendered.as_bytes());
    Ok(exit::SUCCESS)
}

/// Pick the render style:
/// - not a TTY → `Raw` (preserve markdown for `mdcat`, `less`, file capture)
/// - `NO_COLOR` set → `Plain` (formatted layout, no ANSI)
/// - otherwise → `Ansi`
fn pick_style(is_tty: bool, no_color: bool) -> RenderStyle {
    if !is_tty {
        RenderStyle::Raw
    } else if no_color {
        RenderStyle::Plain
    } else {
        RenderStyle::Ansi
    }
}

fn list_pages() {
    let pages = wiki::list_pages();
    if pages.is_empty() {
        println!("{NO_PAGES_NOTE}");
        return;
    }
    println!("embedded wiki pages:");
    for p in &pages {
        println!("  {p}");
    }
    println!();
    println!("Run `proteus wiki <page>` to read one.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_picks_raw() {
        assert_eq!(pick_style(false, false), RenderStyle::Raw);
        assert_eq!(pick_style(false, true), RenderStyle::Raw);
    }

    #[test]
    fn tty_no_color_picks_plain() {
        assert_eq!(pick_style(true, true), RenderStyle::Plain);
    }

    #[test]
    fn tty_with_color_picks_ansi() {
        assert_eq!(pick_style(true, false), RenderStyle::Ansi);
    }
}
