// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;

use crate::exit;
use crate::wiki;

const NO_PAGES_NOTE: &str =
    "no pages embedded yet — `intro`, `quickstart`, `concepts` land in phase A alongside this PR";

pub fn run(page: Option<&str>) -> Result<u8> {
    if let Some(name) = page {
        return print_page(name);
    }
    list_pages();
    Ok(exit::SUCCESS)
}

pub fn run_help(feature: Option<&str>) -> Result<u8> {
    if let Some(name) = feature {
        return print_page(name);
    }
    println!("Usage: proteus help <feature>");
    println!();
    println!("Known wiki pages:");
    list_pages();
    Ok(exit::SUCCESS)
}

fn print_page(name: &str) -> Result<u8> {
    if let Some(content) = wiki::get_page(name) {
        print!("{content}");
        if !content.ends_with('\n') {
            println!();
        }
        return Ok(exit::SUCCESS);
    }
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
    Ok(exit::GENERIC_ERROR)
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
