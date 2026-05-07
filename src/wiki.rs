// SPDX-License-Identifier: GPL-3.0-or-later

use include_dir::{Dir, include_dir};

static WIKI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/wiki");

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
        .collect();
    names.sort();
    names
}

pub fn get_page(name: &str) -> Option<&'static str> {
    let path = format!("{name}.md");
    WIKI.get_file(&path).and_then(|f| f.contents_utf8())
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
}
