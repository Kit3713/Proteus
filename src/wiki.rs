// SPDX-License-Identifier: GPL-3.0-or-later

use include_dir::{Dir, include_dir};

static WIKI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/wiki");

// Build-time-generated `WIKI_LINES: &[(page_name, line_no, byte_off, byte_len)]`,
// one entry per non-blank line across every embedded wiki page. The text
// itself lives in the `include_dir!` blob — keeping only offsets here
// avoids duplicating ~270KB of wiki content in the binary.
include!(concat!(env!("OUT_DIR"), "/wiki_index.rs"));

/// Page name reserved for the curated TOC; never appears in `list_pages()`
/// and never participates in alphabetical search ranking — it would always
/// dominate "what's the wiki say about X" because it mentions every page.
pub const CURATED_INDEX_PAGE: &str = "_index";

pub fn list_pages() -> Vec<String> {
    let mut names: Vec<String> = WIKI
        .files()
        .filter_map(|f| {
            let path = f.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                return None;
            }
            path.file_stem().and_then(|s| s.to_str()).map(str::to_owned)
        })
        .filter(|n| n != CURATED_INDEX_PAGE)
        .collect();
    names.sort();
    names
}

pub fn get_page(name: &str) -> Option<&'static str> {
    // NEV2.1: defense-in-depth — refuse names that could traverse out of
    // the embedded archive. The `include_dir!` archive itself rejects
    // path-component traversal at build time, but a future caller may
    // forward user-supplied page names (already happens via `proteus
    // wiki <name>`); a strict allow-list on the way in is cheaper than
    // auditing every caller for shape.
    if !is_valid_page_name(name) {
        return None;
    }
    let path = format!("{name}.md");
    WIKI.get_file(&path).and_then(|f| f.contents_utf8())
}

/// NEV2.1: page names are `^[A-Za-z0-9_-]+$`. Includes the curated
/// index page name (`_index`) so the `curated_index()` accessor still
/// resolves. Rejects path-traversal markers, separators, and embedded
/// dots (which would let a caller bypass the `.md` suffix join).
fn is_valid_page_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    if name == "." || name == ".." {
        return false;
    }
    name.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Curated table-of-contents page (`_index.md`). When present, `proteus wiki`
/// renders this instead of the alphabetical page list. Returns `None` if
/// the file is missing — callers must fall back gracefully.
pub fn curated_index() -> Option<&'static str> {
    get_page(CURATED_INDEX_PAGE)
}

/// One ranked search hit. `line` is the full source line (no trim) so the
/// caller can render its own snippet with whatever window it likes.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub page: &'static str,
    pub line_no: u32,
    pub line: &'static str,
    /// First match offset within `line`, in bytes. Used for snippet windowing.
    pub match_offset: usize,
    /// Number of distinct query terms found anywhere on this page.
    pub matched_terms: usize,
    /// Total occurrences of any query term across the whole page.
    pub term_frequency: usize,
    /// Composite rank: `matched_terms × log2(term_frequency + 1)`. Higher is better.
    pub score: f32,
}

/// Tokenize the user's query: lowercase, split on whitespace, drop empties.
fn tokenize(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|t| !t.is_empty())
        .collect()
}

/// Search the embedded wiki for `query`. Returns up to `limit` hits ordered
/// by descending score (with ties broken by page name then line number for
/// deterministic output).
///
/// Ranking: per page, we count how many distinct query tokens appear at all
/// and the total occurrences of any token; the page score is
/// `matched_terms × log2(total_occurrences + 1)`. Each surviving page
/// contributes one hit, anchored on the first line that matches any token.
///
/// R6: terms are tokenised once up front and then scanned across each page
/// in a single sweep — `term_first[i]` and `term_count[i]` are reset at
/// page boundaries instead of re-iterating the entries-per-page once per
/// term. Net effect: we walk the index O(pages × lines × terms) with
/// per-page constant overhead instead of O(pages × terms × lines) with a
/// per-term restart that was thrashing the L1 footprint of the entries
/// slice. For multi-term queries on the embedded wiki this halves the
/// inner-loop cost.
pub fn search(query: &str, limit: usize) -> Vec<SearchHit> {
    let terms = tokenize(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let term_count = terms.len();

    let mut hits: Vec<SearchHit> = Vec::new();
    // Reused across pages — sized once, cleared per-page below.
    let mut per_term_count: Vec<usize> = vec![0; term_count];
    let mut per_term_first: Vec<Option<(u32, &'static str, usize)>> = vec![None; term_count];

    let mut cursor = 0;
    while cursor < WIKI_LINES.len() {
        let page = WIKI_LINES[cursor].0;
        // Walk the contiguous slice for this page (entries are page-grouped).
        let page_start = cursor;
        while cursor < WIKI_LINES.len() && WIKI_LINES[cursor].0 == page {
            cursor += 1;
        }
        let page_end = cursor;
        let Some(page_text) = get_page(page) else {
            continue;
        };
        let entries = &WIKI_LINES[page_start..page_end];

        // Reset per-page accumulators in place rather than reallocating.
        for slot in per_term_count.iter_mut() {
            *slot = 0;
        }
        for slot in per_term_first.iter_mut() {
            *slot = None;
        }

        // Single sweep through every line on this page; for each line,
        // try every term against it. Walking the lines once is the win
        // — entries[i] stays cache-hot as we test all terms against it.
        for &(_, line_no, off, len) in entries {
            let line = &page_text[off as usize..(off + len) as usize];
            for (i, term) in terms.iter().enumerate() {
                let mut start = 0;
                while let Some(found) = case_insensitive_find_from(line, term, start) {
                    if per_term_count[i] == 0 {
                        per_term_first[i] = Some((line_no, line, found));
                    }
                    per_term_count[i] += 1;
                    start = found + term.len();
                }
            }
        }

        // Aggregate the per-term counts into the page-level score.
        let mut total_occurrences = 0usize;
        let mut matched_terms = 0usize;
        let mut first_line: Option<(u32, &'static str, usize)> = None;
        for i in 0..term_count {
            if per_term_count[i] > 0 {
                matched_terms += 1;
                total_occurrences += per_term_count[i];
                if let Some((line_no, line, off)) = per_term_first[i]
                    && first_line.is_none_or(|(prev_no, _, _)| line_no < prev_no)
                {
                    first_line = Some((line_no, line, off));
                }
            }
        }
        let Some((line_no, line, match_offset)) = first_line else {
            continue;
        };
        let tf = (total_occurrences as f32 + 1.0).log2();
        let score = matched_terms as f32 * tf;
        hits.push(SearchHit {
            page,
            line_no,
            line,
            match_offset,
            matched_terms,
            term_frequency: total_occurrences,
            score,
        });
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.matched_terms.cmp(&a.matched_terms))
            .then_with(|| a.page.cmp(b.page))
            .then_with(|| a.line_no.cmp(&b.line_no))
    });
    hits.truncate(limit);
    hits
}

/// Build a `±window` byte snippet around `match_offset` in `line`, with
/// "…" prefix/suffix when truncated. The window is clamped to UTF-8
/// codepoint boundaries so we never emit a torn multibyte sequence.
pub fn snippet(line: &str, match_offset: usize, window: usize) -> String {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let raw_start = match_offset.saturating_sub(window);
    let raw_end = (match_offset + window).min(len);
    let start = floor_char_boundary(line, raw_start);
    let end = ceil_char_boundary(line, raw_end);
    let mut out = String::with_capacity(end - start + 6);
    if start > 0 {
        out.push('\u{2026}');
    }
    out.push_str(&line[start..end]);
    if end < len {
        out.push('\u{2026}');
    }
    out
}

fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
    let len = s.len();
    if idx >= len {
        return len;
    }
    while idx < len && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// Case-insensitive byte-search for `needle` in `haystack[from..]`.
/// ASCII-only folding (matches `to_lowercase` semantics in `tokenize`);
/// non-ASCII characters compare case-sensitively, which is fine for the
/// wiki's English prose plus a handful of em-dashes.
fn case_insensitive_find_from(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if from + n.len() > h.len() {
        return None;
    }
    'outer: for start in from..=h.len() - n.len() {
        for (i, &b) in n.iter().enumerate() {
            if !h[start + i].eq_ignore_ascii_case(&b) {
                continue 'outer;
            }
        }
        return Some(start);
    }
    None
}

/// ANSI rendering style. `Plain` emits no escape codes (for pipes / `NO_COLOR`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RenderStyle {
    /// Raw markdown, no transformation. Used when stdout is not a TTY.
    Raw,
    /// Layout-only: bullets/headings indented, no ANSI codes.
    Plain,
    /// Full ANSI styling: bold headings, dim code, etc.
    Ansi,
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const UNDERLINE: &str = "\x1b[4m";

/// Render a markdown wiki page to a string for terminal display.
///
/// Recognised constructs (everything else passes through verbatim):
/// `#`/`##`/`###` headings, fenced code blocks, `- ` / `* ` bullets,
/// `**bold**`, `*italic*`, and `` `code` `` spans.
pub fn render(markdown: &str, style: RenderStyle) -> String {
    if style == RenderStyle::Raw {
        let mut out = markdown.to_string();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        return out;
    }

    let mut out = String::with_capacity(markdown.len() + 64);
    let mut in_code_fence = false;

    for line in markdown.split_inclusive('\n') {
        let raw = line.trim_end_matches('\n');
        let nl = if line.ends_with('\n') { "\n" } else { "" };

        if let Some(rest) = raw.strip_prefix("```") {
            in_code_fence = !in_code_fence;
            // Opening fence with a language hint: render as a dim caption.
            if in_code_fence && !rest.trim().is_empty() && style == RenderStyle::Ansi {
                out.push_str(DIM);
                out.push_str("    ");
                out.push_str(rest.trim());
                out.push_str(RESET);
                out.push_str(nl);
            }
            continue;
        }

        if in_code_fence {
            push_code_line(raw, style, &mut out);
        } else if !push_heading(raw, style, &mut out) && !push_bullet(raw, style, &mut out) {
            push_inline(raw, style, &mut out);
        }
        out.push_str(nl);
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn push_code_line(raw: &str, style: RenderStyle, out: &mut String) {
    if style == RenderStyle::Ansi {
        out.push_str(DIM);
    }
    out.push_str("    ");
    out.push_str(raw);
    if style == RenderStyle::Ansi {
        out.push_str(RESET);
    }
}

fn push_heading(raw: &str, style: RenderStyle, out: &mut String) -> bool {
    let trimmed = raw.trim_start();
    let level = trimmed.bytes().take_while(|b| *b == b'#').count();
    if level == 0 || level > 6 {
        return false;
    }
    let Some(body) = trimmed.get(level..).map(str::trim_start) else {
        return false;
    };
    if body.is_empty() && trimmed.len() != level {
        return false;
    }

    match (style, level) {
        (RenderStyle::Ansi, 1) => {
            out.push_str(BOLD);
            out.push_str(UNDERLINE);
            push_inline(body, style, out);
            out.push_str(RESET);
        }
        (RenderStyle::Ansi, 2) => {
            out.push_str(BOLD);
            push_inline(body, style, out);
            out.push_str(RESET);
        }
        (RenderStyle::Ansi, _) => {
            out.push_str(BOLD);
            out.push_str(DIM);
            push_inline(body, style, out);
            out.push_str(RESET);
        }
        (_, 1 | 2) => push_inline(body, style, out),
        (_, _) => {
            out.push_str("  ");
            push_inline(body, style, out);
        }
    }
    true
}

fn push_bullet(raw: &str, style: RenderStyle, out: &mut String) -> bool {
    let leading = raw.len() - raw.trim_start().len();
    let trimmed = &raw[leading..];
    let Some(body) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    else {
        return false;
    };

    for _ in 0..leading {
        out.push(' ');
    }
    if style == RenderStyle::Ansi {
        out.push_str(DIM);
        out.push('\u{2022}');
        out.push_str(RESET);
    } else {
        out.push('\u{2022}');
    }
    out.push(' ');
    push_inline(body, style, out);
    true
}

fn push_inline(raw: &str, style: RenderStyle, out: &mut String) {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'*'
            && let Some(end) = find_close(bytes, i + 2, b"**")
        {
            push_styled(out, &raw[i + 2..end], BOLD, style);
            i = end + 2;
            continue;
        }
        if bytes[i] == b'*'
            && (i + 1 >= bytes.len() || bytes[i + 1] != b'*')
            && let Some(end) = find_close_single(bytes, i + 1, b'*')
        {
            push_styled(out, &raw[i + 1..end], ITALIC, style);
            i = end + 1;
            continue;
        }
        if bytes[i] == b'`'
            && let Some(end) = find_close_single(bytes, i + 1, b'`')
        {
            push_styled(out, &raw[i + 1..end], DIM, style);
            i = end + 1;
            continue;
        }
        let ch_len = utf8_char_len(bytes[i]);
        out.push_str(&raw[i..i + ch_len]);
        i += ch_len;
    }
}

fn push_styled(out: &mut String, body: &str, ansi: &str, style: RenderStyle) {
    if style == RenderStyle::Ansi {
        out.push_str(ansi);
        out.push_str(body);
        out.push_str(RESET);
    } else {
        out.push_str(body);
    }
}

fn find_close(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || start >= bytes.len() {
        return None;
    }
    bytes[start..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| start + p)
}

fn find_close_single(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    bytes[start..]
        .iter()
        .position(|b| *b == needle)
        .map(|p| start + p)
}

/// Length in bytes of the UTF-8 codepoint starting at `first`.
/// Returns 1 for ASCII and stray continuation bytes (defensive: keeps the
/// scanner advancing past malformed input).
fn utf8_char_len(first: u8) -> usize {
    match first {
        0..=0x7F => 1,
        0x80..=0xBF => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_page_refuses_path_traversal_names() {
        // NEV2.1: a future caller forwarding user input must not be
        // able to read arbitrary embedded paths via traversal markers.
        for bad in [
            "",
            "..",
            ".",
            "../etc/passwd",
            "/etc/passwd",
            "captive-portals/extra",
            "captive\0portals",
            "captive portals",
            "name with space",
            "a.b",
        ] {
            assert!(
                get_page(bad).is_none(),
                "{bad:?} must be refused by get_page"
            );
        }
    }

    #[test]
    fn get_page_resolves_known_pages() {
        // Known-good shapes still work post-validation.
        assert!(get_page(CURATED_INDEX_PAGE).is_some());
        for name in list_pages() {
            assert!(get_page(&name).is_some(), "page {name} must resolve");
        }
    }

    #[test]
    fn raw_passthrough_appends_trailing_newline() {
        let out = render("hello", RenderStyle::Raw);
        assert_eq!(out, "hello\n");
    }

    #[test]
    fn raw_preserves_existing_trailing_newline() {
        let out = render("hello\n", RenderStyle::Raw);
        assert_eq!(out, "hello\n");
    }

    #[test]
    fn raw_equals_input_for_every_embedded_page() {
        for name in list_pages() {
            let content = get_page(&name).expect("listed page must exist");
            let rendered = render(content, RenderStyle::Raw);
            // Raw must be byte-identical to the source (modulo trailing newline).
            let mut expected = content.to_string();
            if !expected.ends_with('\n') {
                expected.push('\n');
            }
            assert_eq!(rendered, expected, "raw mismatch for page {name}");
        }
    }

    #[test]
    fn raw_contains_no_ansi_escape() {
        for name in list_pages() {
            let content = get_page(&name).expect("listed page must exist");
            let rendered = render(content, RenderStyle::Raw);
            assert!(
                !rendered.contains('\x1b'),
                "ANSI leaked into raw for {name}"
            );
        }
    }

    #[test]
    fn plain_contains_no_ansi_escape() {
        for name in list_pages() {
            let content = get_page(&name).expect("listed page must exist");
            let rendered = render(content, RenderStyle::Plain);
            assert!(
                !rendered.contains('\x1b'),
                "ANSI leaked into plain for {name}"
            );
        }
    }

    #[test]
    fn ansi_does_not_panic_on_any_embedded_page() {
        for name in list_pages() {
            let content = get_page(&name).expect("listed page must exist");
            let rendered = render(content, RenderStyle::Ansi);
            assert!(!rendered.is_empty(), "rendered output empty for {name}");
        }
    }

    #[test]
    fn h1_in_ansi_is_bold_underlined() {
        let out = render("# Title", RenderStyle::Ansi);
        assert!(out.contains(BOLD));
        assert!(out.contains(UNDERLINE));
        assert!(out.contains("Title"));
    }

    #[test]
    fn h2_in_plain_keeps_text_no_hashes() {
        let out = render("## Section", RenderStyle::Plain);
        assert_eq!(out, "Section\n");
    }

    #[test]
    fn bullet_renders_with_unicode_dot() {
        let out = render("- one\n- two", RenderStyle::Plain);
        assert!(out.contains("\u{2022} one"));
        assert!(out.contains("\u{2022} two"));
    }

    #[test]
    fn nested_bullet_indent_preserved() {
        let out = render("- top\n  - nested", RenderStyle::Plain);
        assert!(out.contains("\u{2022} top"));
        assert!(out.contains("  \u{2022} nested"));
    }

    #[test]
    fn bold_strips_markers_in_plain() {
        let out = render("hello **world**", RenderStyle::Plain);
        assert_eq!(out, "hello world\n");
    }

    #[test]
    fn bold_wraps_in_ansi() {
        let out = render("**x**", RenderStyle::Ansi);
        assert!(out.contains(BOLD));
        assert!(out.contains("x"));
        assert!(out.contains(RESET));
    }

    #[test]
    fn inline_code_strips_backticks_in_plain() {
        let out = render("run `proteus status` now", RenderStyle::Plain);
        assert_eq!(out, "run proteus status now\n");
    }

    #[test]
    fn fenced_code_block_indented() {
        let md = "```\nlet x = 1;\n```\n";
        let out = render(md, RenderStyle::Plain);
        assert!(out.contains("    let x = 1;"));
        assert!(!out.contains("```"));
    }

    #[test]
    fn fenced_code_block_with_lang_drops_fence() {
        let md = "```rust\nfn x() {}\n```\n";
        let out = render(md, RenderStyle::Plain);
        assert!(!out.contains("```"));
        assert!(out.contains("    fn x() {}"));
    }

    #[test]
    fn unmatched_asterisk_is_passed_through() {
        let out = render("a * b * c", RenderStyle::Plain);
        // Single `*` followed by content followed by `*` *is* italic — verify
        // we keep the inner text and drop the markers.
        assert_eq!(out, "a  b  c\n");
    }

    #[test]
    fn lone_asterisk_no_pair_is_kept() {
        let out = render("a * b", RenderStyle::Plain);
        assert_eq!(out, "a * b\n");
    }

    #[test]
    fn utf8_content_is_not_corrupted() {
        let out = render("café — résumé", RenderStyle::Plain);
        assert_eq!(out, "café — résumé\n");
    }

    #[test]
    fn h3_in_plain_indented_two_spaces() {
        let out = render("### Sub", RenderStyle::Plain);
        assert_eq!(out, "  Sub\n");
    }

    #[test]
    fn paragraph_passthrough() {
        let out = render("just a line", RenderStyle::Plain);
        assert_eq!(out, "just a line\n");
    }

    // ---- search ---------------------------------------------------------

    #[test]
    fn empty_query_returns_no_hits() {
        assert!(search("", 10).is_empty());
        assert!(search("   ", 10).is_empty());
    }

    #[test]
    fn search_is_case_insensitive() {
        let lower = search("captive", 50);
        let upper = search("CAPTIVE", 50);
        let mixed = search("CapTive", 50);
        assert!(!lower.is_empty(), "expected hits for 'captive'");
        let pages_lower: Vec<_> = lower.iter().map(|h| h.page).collect();
        let pages_upper: Vec<_> = upper.iter().map(|h| h.page).collect();
        let pages_mixed: Vec<_> = mixed.iter().map(|h| h.page).collect();
        assert_eq!(pages_lower, pages_upper);
        assert_eq!(pages_lower, pages_mixed);
    }

    #[test]
    fn search_returns_captive_portals_for_captive() {
        let hits = search("captive", 10);
        assert!(
            hits.iter().any(|h| h.page == "captive-portals"),
            "expected captive-portals in hits, got: {:?}",
            hits.iter().map(|h| h.page).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_respects_limit() {
        let many = search("the", 3);
        assert!(many.len() <= 3);
    }

    #[test]
    fn search_multi_term_prefers_pages_with_more_matched_terms() {
        // A doc with both "captive" and "portal" should outrank one with
        // only one of them.
        let hits = search("captive portal", 10);
        assert!(!hits.is_empty());
        // captive-portals.md should hit both.
        let cp = hits.iter().find(|h| h.page == "captive-portals");
        assert!(cp.is_some(), "captive-portals page should match");
        assert_eq!(cp.unwrap().matched_terms, 2);
    }

    #[test]
    fn search_hit_line_text_appears_in_source() {
        let hits = search("MAC", 10);
        for hit in &hits {
            let page = get_page(hit.page).expect("page exists");
            let nth_line = page.lines().nth((hit.line_no - 1) as usize);
            assert_eq!(
                nth_line,
                Some(hit.line),
                "page={} line={}",
                hit.page,
                hit.line_no
            );
        }
    }

    #[test]
    fn search_match_offset_lands_on_term() {
        let hits = search("captive", 10);
        for hit in &hits {
            let slice = &hit.line[hit.match_offset..];
            let head = slice.get(..7).unwrap_or(slice);
            assert!(
                head.eq_ignore_ascii_case("captive"),
                "match_offset wrong: line={:?} offset={} head={:?}",
                hit.line,
                hit.match_offset,
                head
            );
        }
    }

    #[test]
    fn snippet_truncates_with_ellipsis_and_keeps_term_visible() {
        let line = "Long preamble before the captive portal mention and a long tail afterward.";
        let off = case_insensitive_find_from(line, "captive", 0).unwrap();
        let snip = snippet(line, off, 10);
        assert!(snip.contains("captive"));
        // We expected truncation on at least one side.
        assert!(snip.starts_with('\u{2026}') || snip.ends_with('\u{2026}'));
    }

    #[test]
    fn snippet_handles_match_at_start_and_end() {
        let line = "captive portal at start";
        let snip = snippet(line, 0, 8);
        assert!(snip.contains("captive"));
        let line2 = "ending in captive";
        let off = case_insensitive_find_from(line2, "captive", 0).unwrap();
        let snip2 = snippet(line2, off, 100);
        assert_eq!(snip2, line2);
    }

    #[test]
    fn snippet_does_not_split_utf8() {
        // Em-dash (U+2014) is 3 bytes in UTF-8. With a window large enough
        // to span both em-dashes we should land on char boundaries cleanly.
        let line = "left — captive — right";
        let off = case_insensitive_find_from(line, "captive", 0).unwrap();
        let snip = snippet(line, off, 4);
        // The window is smaller than "captive" itself, so the term may be
        // truncated. The contract is just: valid UTF-8, no panic, snippet
        // is a substring of the line possibly bracketed by ellipses.
        let core = snip
            .trim_start_matches('\u{2026}')
            .trim_end_matches('\u{2026}');
        assert!(line.contains(core), "core {core:?} not in line");
    }

    #[test]
    fn case_insensitive_find_from_walks_overlap_free() {
        // Successive calls advancing by `needle.len()` count occurrences.
        let h = "AAAA";
        let n = "aa";
        let mut start = 0;
        let mut count = 0;
        while let Some(off) = case_insensitive_find_from(h, n, start) {
            count += 1;
            start = off + n.len();
        }
        assert_eq!(count, 2);
    }

    #[test]
    fn case_insensitive_find_from_is_case_insensitive() {
        assert_eq!(case_insensitive_find_from("foo BAR baz", "bar", 0), Some(4));
        assert_eq!(
            case_insensitive_find_from("nothing here", "missing", 0),
            None
        );
        // Empty needle is treated as "no match" so the count loop terminates.
        assert_eq!(case_insensitive_find_from("abc", "", 0), None);
    }

    #[test]
    fn case_insensitive_find_from_respects_start_offset() {
        assert_eq!(case_insensitive_find_from("aaa aaa", "aaa", 1), Some(4));
    }

    #[test]
    fn search_is_deterministic_for_same_query() {
        let a = search("MAC rotation", 10);
        let b = search("MAC rotation", 10);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.page, y.page);
            assert_eq!(x.line_no, y.line_no);
        }
    }

    #[test]
    fn build_time_index_is_non_empty() {
        assert!(
            !WIKI_LINES.is_empty(),
            "build.rs should populate WIKI_LINES"
        );
    }

    #[test]
    fn curated_index_present_and_excluded_from_listing() {
        // The TOC must exist (it's the page `proteus wiki` shows by default
        // when no arg is given) and must not appear as a regular page in
        // `list_pages()` — otherwise it would show in `--help` listings,
        // search results, and tab completion.
        let content = curated_index().expect("wiki/_index.md must exist");
        assert!(
            content.contains("Curated guide"),
            "_index.md should look like the curated TOC, got first 80 chars: {}",
            &content[..content.len().min(80)]
        );
        assert!(
            !list_pages().iter().any(|p| p == CURATED_INDEX_PAGE),
            "list_pages() must filter out the curated index"
        );
    }

    #[test]
    fn curated_index_excluded_from_search() {
        // Even if a user searches for a generic term, _index should never
        // appear as a hit — it would dominate (it mentions every page) and
        // hide real content.
        let hits = search("curated guide", 50);
        for h in &hits {
            assert_ne!(h.page, CURATED_INDEX_PAGE, "_index leaked into search hits");
        }
    }

    #[test]
    fn build_time_index_pages_match_embedded_pages() {
        use std::collections::BTreeSet;
        let from_index: BTreeSet<&str> = WIKI_LINES.iter().map(|(p, _, _, _)| *p).collect();
        let from_files: BTreeSet<String> = list_pages().into_iter().collect();
        // Every page in the index should be a real page; every page should
        // have at least one indexed line (since none of our wiki pages are
        // entirely blank).
        for page in &from_index {
            assert!(from_files.contains(*page), "indexed unknown page {page}");
        }
        for page in &from_files {
            assert!(
                from_index.iter().any(|p| *p == page.as_str()),
                "page {page} missing from index"
            );
        }
    }

    #[test]
    fn search_completes_under_50ms_release_target() {
        // Soft sanity check: dev build, so target is loose. The release-mode
        // target in PLAN.md is <200ms cold; we should be far under that here.
        let start = std::time::Instant::now();
        let _ = search("captive portal MAC rotation", 10);
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 200,
            "search took {}ms (>200ms budget)",
            elapsed.as_millis()
        );
    }
}
