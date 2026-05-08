// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::{IsTerminal, Write};

use anyhow::Result;
use serde::Serialize;

use crate::commands::print_json;
use crate::exit;
use crate::wiki::{self, RenderStyle, SearchHit};

/// Bytes of context shown on either side of the first match in a snippet.
const SNIPPET_WINDOW: usize = 40;

const NO_PAGES_NOTE: &str =
    "no pages embedded yet — `intro`, `quickstart`, `concepts` land in phase A alongside this PR";

pub fn run(page: Option<&str>, json: bool, no_color: bool) -> Result<u8> {
    // CL6: emit a small JSON payload when `--json` is set so wrappers
    // can navigate the wiki without grepping the rendered markdown. The
    // shape is intentionally narrow: page-name list when no page is
    // given, `{ page, content }` when one is.
    if json {
        return run_json(page);
    }
    if let Some(name) = page {
        return print_page(name, no_color);
    }
    // Prefer the curated TOC. Fall back to the alphabetical list when the
    // index file is missing (older trees, partial extraction).
    if let Some(content) = wiki::curated_index() {
        render_to_stdout(content, no_color);
    } else {
        list_pages();
    }
    Ok(exit::SUCCESS)
}

#[derive(Serialize)]
struct WikiPageList {
    pages: Vec<String>,
}

#[derive(Serialize)]
struct WikiPageContent<'a> {
    page: &'a str,
    content: &'a str,
}

fn run_json(page: Option<&str>) -> Result<u8> {
    match page {
        Some(name) => match wiki::get_page(name) {
            Some(content) => {
                print_json(&WikiPageContent {
                    page: name,
                    content,
                })?;
                Ok(exit::SUCCESS)
            }
            None => {
                // Match the human path: stderr "no wiki page" and
                // exit GENERIC_ERROR so wrappers see a typed failure.
                eprintln!("proteus: no wiki page '{name}'");
                Ok(exit::GENERIC_ERROR)
            }
        },
        None => {
            print_json(&WikiPageList {
                pages: wiki::list_pages(),
            })?;
            Ok(exit::SUCCESS)
        }
    }
}

fn render_to_stdout(content: &str, no_color_flag: bool) {
    let no_color_env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let style = pick_style(
        std::io::stdout().is_terminal(),
        no_color_flag || no_color_env,
    );
    let rendered = wiki::render(content, style);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(rendered.as_bytes());
}

/// Implements `proteus wiki search <query...>`. Tokenizes the query by
/// whitespace, scans the embedded `WIKI_LINES` table case-insensitively,
/// and prints up to `limit` ranked hits.
pub fn run_search(query: &[String], json: bool, limit: usize) -> Result<u8> {
    let joined = query.join(" ");
    let trimmed = joined.trim();
    if trimmed.is_empty() {
        eprintln!("proteus: empty search query — pass at least one term");
        return Ok(exit::GENERIC_ERROR);
    }

    let hits = wiki::search(trimmed, limit);

    if json {
        let payload = SearchOutput {
            query: trimmed,
            count: hits.len(),
            hits: hits.iter().map(SearchHitJson::from).collect(),
        };
        print_json(&payload)?;
        return Ok(exit::SUCCESS);
    }

    if hits.is_empty() {
        println!("no matches for '{trimmed}'");
        return Ok(exit::SUCCESS);
    }

    println!(
        "{} match{} for '{}':",
        hits.len(),
        if hits.len() == 1 { "" } else { "es" },
        trimmed
    );
    for hit in &hits {
        let snippet = wiki::snippet(hit.line, hit.match_offset, SNIPPET_WINDOW);
        println!("  {}:{}  {}", hit.page, hit.line_no, snippet);
    }
    println!();
    println!("Run `proteus wiki <page>` to read a result.");
    Ok(exit::SUCCESS)
}

#[derive(Serialize)]
struct SearchOutput<'a> {
    query: &'a str,
    count: usize,
    hits: Vec<SearchHitJson>,
}

#[derive(Serialize)]
struct SearchHitJson {
    page: String,
    line_no: u32,
    line: String,
    snippet: String,
    matched_terms: usize,
    term_frequency: usize,
    score: f32,
}

impl SearchHitJson {
    fn from(hit: &SearchHit) -> Self {
        Self {
            page: hit.page.to_string(),
            line_no: hit.line_no,
            line: hit.line.to_string(),
            snippet: wiki::snippet(hit.line, hit.match_offset, SNIPPET_WINDOW),
            matched_terms: hit.matched_terms,
            term_frequency: hit.term_frequency,
            score: hit.score,
        }
    }
}

pub fn run_help(feature: Option<&str>, no_color: bool) -> Result<u8> {
    if let Some(name) = feature {
        // Issue #166: when an exact page lookup misses (e.g.
        // `proteus help apply` — there's a subcommand but no wiki page
        // named `apply`), fall through to a search rather than dumping
        // an alphabetical page list.
        if wiki::get_page(name).is_some() {
            return print_page(name, no_color);
        }
        return print_help_search_fallback(name, no_color);
    }
    println!("Usage: proteus help <feature>");
    println!();
    println!("Known wiki pages:");
    list_pages();
    Ok(exit::SUCCESS)
}

/// Issue #166: fallback path for `proteus help <feature>` when no exact
/// page matches. Runs a wiki search and surfaces the top hits with line
/// snippets so the user can find the right page name.
fn print_help_search_fallback(query: &str, no_color_flag: bool) -> Result<u8> {
    let hits = wiki::search(query, 5);
    if hits.is_empty() {
        eprintln!("proteus: no wiki page or matches for '{query}'");
        eprintln!("  try `proteus wiki` for the curated index");
        return Ok(exit::GENERIC_ERROR);
    }
    eprintln!("proteus: no wiki page '{query}'; closest matches:");
    for hit in &hits {
        let snippet = wiki::snippet(hit.line, hit.match_offset, SNIPPET_WINDOW);
        eprintln!("  {}:{}  {}", hit.page, hit.line_no, snippet);
    }
    eprintln!();
    eprintln!("Run `proteus wiki <page>` to read one.");
    let _ = no_color_flag; // search output is plain stderr; no styling
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
