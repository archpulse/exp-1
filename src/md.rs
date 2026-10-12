//! Markdown (and inline/display math) to styled terminal runs.
//!
//! Rendering produces theme-independent [`Run`]s that the UI maps to colours at
//! draw time, so switching themes costs nothing and parsing happens once per
//! message rather than once per frame.
//!
//! Hand written single pass: no regex engine, no external renderer, no fonts,
//! no rasterisation. A long answer parses in well under a millisecond.

use crate::latex;

#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    Text,
    Heading,
    Strong,
    Emphasis,
    Code,
    CodeBlock,
    Quote,
    Bullet,
    Rule,
    Link,
    Math,
}

#[derive(Clone)]
pub struct Run {
    pub style: Style,
    pub text: String,
    /// Where a link points, when it is one: the label is not the address.
    pub url: Option<String>,
}

impl Run {
    pub fn new(style: Style, text: impl Into<String>) -> Self {
        Self {
            style,
            text: text.into(),
            url: None,
        }
    }

    /// A link: the text is what is drawn, the url is where a click goes.
    pub fn link(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            style: Style::Link,
            text: text.into(),
            url: Some(url.into()),
        }
    }
}

fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
}

/// Parses inline spans: emphasis, code, links, math.
fn inline(text: &str) -> Vec<Run> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs = Vec::new();
    let mut plain = String::new();
    let mut index = 0usize;

    macro_rules! flush {
        () => {
            if !plain.is_empty() {
                runs.push(Run::new(Style::Text, std::mem::take(&mut plain)));
            }
        };
    }

    while index < chars.len() {
        let rest: String = chars[index..].iter().collect();

        // Inline code wins over everything: no nested formatting inside.
        if rest.starts_with('`') {
            if let Some(end) = rest[1..].find('`') {
                flush!();
                runs.push(Run::new(Style::Code, &rest[1..1 + end]));
                index += rest[..1 + end + 1].chars().count();
                continue;
            }
        }

        // $...$ inline math.
        if rest.starts_with('$') && !rest.starts_with("$$") {
            if let Some(end) = rest[1..].find('$') {
                let body = &rest[1..1 + end];
                if !body.trim().is_empty() {
                    flush!();
                    runs.push(Run::new(Style::Math, latex::render(body)));
                    index += rest[..2 + end].chars().count();
                    continue;
                }
            }
        }

        // \(...\) inline math, the LaTeX spelling of $...$.
        if rest.starts_with("\\(") {
            if let Some(end) = rest[2..].find("\\)") {
                let body = &rest[2..2 + end];
                if !body.trim().is_empty() {
                    flush!();
                    runs.push(Run::new(Style::Math, latex::render(body)));
                    index += rest[..4 + end].chars().count();
                    continue;
                }
            }
        }

        // **bold** and __bold__
        if rest.starts_with("**") || rest.starts_with("__") {
            let marker = &rest[..2];
            if let Some(end) = rest[2..].find(marker) {
                flush!();
                runs.push(Run::new(Style::Strong, &rest[2..2 + end]));
                index += rest[..4 + end].chars().count();
                continue;
            }
        }

        // ~~strike~~
        if rest.starts_with("~~") {
            if let Some(end) = rest[2..].find("~~") {
                flush!();
                runs.push(Run::new(Style::Text, &rest[2..2 + end]));
                index += rest[..4 + end].chars().count();
                continue;
            }
        }

        // *italic* and _italic_
        if rest.starts_with('*') || rest.starts_with('_') {
            let marker = &rest[..1];
            let after = &rest[1..];
            // Avoid treating a trailing '*' as an opener.
            if let Some(end) = after.find(marker)
                && end > 0
            {
                flush!();
                runs.push(Run::new(Style::Emphasis, &after[..end]));
                index += rest[..2 + end].chars().count();
                continue;
            }
        }

        // [text](url)
        if rest.starts_with('[') {
            if let Some(close) = rest.find(']') {
                if rest[close..].starts_with("](") {
                    if let Some(paren) = rest[close..].find(')') {
                        flush!();
                        // `[label](url)`: the label is drawn, the url is kept
                        // for the click.
                        let url = &rest[close + 2..close + paren];
                        runs.push(Run::link(&rest[1..close], url));
                        index += rest[..close + paren + 1].chars().count();
                        continue;
                    }
                }
            }
        }

        plain.push(chars[index]);
        index += 1;
    }

    flush!();
    runs
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Center,
    Right,
}

fn split_row(line: &str) -> Vec<String> {
    let trimmed = line.trim().trim_start_matches('|').trim_end_matches('|');
    trimmed
        .split('|')
        .map(|cell| plain_inline(cell.trim()))
        .collect()
}

/// The visible text of an inline markdown string. A table cell is drawn as one
/// run, so its markers have to go even though its styling cannot be kept.
fn plain_inline(text: &str) -> String {
    inline(text)
        .into_iter()
        .map(|run| run.text)
        .collect::<String>()
}

/// True when the line is the `|---|---|` separator row.
fn is_delimiter(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return false;
    }
    let cells = split_row(trimmed);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            !cell.is_empty()
                && cell
                    .chars()
                    .all(|c| matches!(c, '-' | ':' | ' '))
                && cell.contains('-')
        })
}

fn align_of(cell: &str) -> Align {
    let left = cell.starts_with(':');
    let right = cell.ends_with(':');
    match (left, right) {
        (true, true) => Align::Center,
        (true, false) => Align::Left,
        (false, true) => Align::Right,
        (false, false) => Align::Left,
    }
}

fn pad(text: &str, width: usize, align: Align) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.chars().take(width).collect();
    }
    let gap = width - len;
    match align {
        Align::Left => format!("{text}{}", " ".repeat(gap)),
        Align::Right => format!("{}{text}", " ".repeat(gap)),
        Align::Center => {
            let left = gap / 2;
            format!("{}{text}{}", " ".repeat(left), " ".repeat(gap - left))
        }
    }
}

/// Lays a table out with box-drawing borders, sized to its widest cell.
fn render_table(rows: &[Vec<String>], aligns: &[Align]) -> Vec<Vec<Run>> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return Vec::new();
    }
    let mut widths = vec![0usize; columns];
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let border = |left: &str, mid: &str, right: &str| {
        let mut line = String::from(left);
        for (index, width) in widths.iter().enumerate() {
            line.push_str(&"─".repeat(width + 2));
            if index + 1 < widths.len() {
                line.push_str(mid);
            }
        }
        line.push_str(right);
        line
    };

    let body = |row: &Vec<String>, style: Style| {
        let mut line = String::from("│");
        for (index, width) in widths.iter().enumerate() {
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            let align = aligns.get(index).copied().unwrap_or(Align::Left);
            line.push(' ');
            line.push_str(&pad(cell, *width, align));
            line.push_str(" │");
        }
        Run::new(style, line)
    };

    let mut lines = Vec::new();
    lines.push(vec![Run::new(Style::Rule, border("┌", "┬", "┐"))]);
    for (index, row) in rows.iter().enumerate() {
        lines.push(vec![body(row, if index == 0 { Style::Strong } else { Style::Text })]);
        if index == 0 {
            lines.push(vec![Run::new(Style::Rule, border("├", "┼", "┤"))]);
        }
    }
    lines.push(vec![Run::new(Style::Rule, border("└", "┴", "┘"))]);
    lines
}

/// Parses a GitHub-style pipe table spanning consecutive `|` lines.
fn try_table(src: &[&str], start: usize) -> Option<(Vec<Vec<Run>>, usize)> {
    if !src.get(start)?.trim().starts_with('|') {
        return None;
    }
    let delimiter = src.get(start + 1)?;
    if !is_delimiter(delimiter) {
        return None;
    }

    let header = split_row(src[start].trim());
    let aligns: Vec<Align> = split_row(delimiter.trim()).iter().map(|c| align_of(c)).collect();

    let mut rows = vec![header];
    let mut index = start + 2;
    while index < src.len() {
        let line = src[index].trim();
        if !line.starts_with('|') {
            break;
        }
        rows.push(split_row(line));
        index += 1;
    }
    Some((render_table(&rows, &aligns), index))
}

fn paragraph(line: &str, indent: usize) -> Vec<Run> {
    let mut runs = Vec::new();
    if indent > 0 {
        runs.push(Run::new(Style::Text, " ".repeat(indent)));
    }
    runs.extend(inline(line));
    runs
}

/// Renders markdown into lines of theme-independent runs.
pub fn render(source: &str) -> Vec<Vec<Run>> {
    let mut lines: Vec<Vec<Run>> = Vec::new();
    let src: Vec<&str> = source.lines().collect();
    let mut index = 0usize;

    while index < src.len() {
        let line = src[index];
        let trimmed = line.trim();

        // Fenced code block.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let fence = &trimmed[..3];
            let language = trimmed[3..].trim().to_string();
            index += 1;
            if !language.is_empty() {
                lines.push(vec![Run::new(Style::Code, language)]);
            }
            while index < src.len() {
                let inner = src[index].trim();
                if inner.starts_with(fence) {
                    index += 1;
                    break;
                }
                lines.push(vec![Run::new(Style::CodeBlock, src[index])]);
                index += 1;
            }
            lines.push(Vec::new());
            continue;
        }

        // Display math $$ ... $$
        if trimmed.starts_with("$$") {
            let mut body = String::new();
            let first = trimmed.trim_start_matches('$').trim();
            if let Some(end) = first.find("$$") {
                body.push_str(&first[..end]);
                index += 1;
            } else {
                body.push_str(first);
                index += 1;
                while index < src.len() {
                    let inner = src[index];
                    if let Some(end) = inner.find("$$") {
                        body.push(' ');
                        body.push_str(&inner[..end]);
                        index += 1;
                        break;
                    }
                    body.push(' ');
                    body.push_str(inner.trim());
                    index += 1;
                }
            }
            lines.push(Vec::new());
            for row in latex::render(body.trim()).split('\n') {
                lines.push(vec![Run::new(Style::Math, row)]);
            }
            lines.push(Vec::new());
            continue;
        }

        // Display math \[ ... \], the spelling LaTeX itself uses for the same
        // thing. Models reach for it as readily as for $$, so both are read.
        if trimmed.starts_with("\\[") {
            let mut body = String::new();
            let first = trimmed.trim_start_matches("\\[").trim();
            if let Some(end) = first.find("\\]") {
                body.push_str(&first[..end]);
                index += 1;
            } else {
                body.push_str(first);
                index += 1;
                while index < src.len() {
                    let inner = src[index];
                    if let Some(end) = inner.find("\\]") {
                        body.push(' ');
                        body.push_str(&inner[..end]);
                        index += 1;
                        break;
                    }
                    body.push(' ');
                    body.push_str(inner.trim());
                    index += 1;
                }
            }
            lines.push(Vec::new());
            for row in latex::render(body.trim()).split('\n') {
                lines.push(vec![Run::new(Style::Math, row)]);
            }
            lines.push(Vec::new());
            continue;
        }

        // Blank line, but never two in a row: a heading already emitted its own
        // spacer, and a double gap reads as a bug.
        if trimmed.is_empty() {
            if !lines.last().is_none_or(|line| line.is_empty()) {
                lines.push(Vec::new());
            }
            index += 1;
            continue;
        }

        // Pipe table, taken as a whole block.
        if trimmed.starts_with('|') {
            if let Some((table, next)) = try_table(&src, index) {
                lines.extend(table);
                lines.push(Vec::new());
                index = next;
                continue;
            }
        }

        // Heading.
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count().min(6);
            let content = trimmed[level..].trim();
            // The hashes are markdown's syntax, not its content: a heading is
            // shown by its style, so the markers are not printed at all.
            lines.push(vec![Run::new(Style::Heading, content)]);
            lines.push(Vec::new());
            index += 1;
            continue;
        }

        // Horizontal rule.
        if is_rule(line) {
            lines.push(vec![Run::new(Style::Rule, "─".repeat(40))]);
            lines.push(Vec::new());
            index += 1;
            continue;
        }

        // Blockquote.
        if trimmed.starts_with('>') {
            let content = trimmed.trim_start_matches('>').trim();
            let mut runs = vec![Run::new(Style::Quote, "  │ ")];
            runs.extend(inline(content));
            lines.push(runs);
            index += 1;
            continue;
        }

        // Bullet and ordered lists.
        let mut handled = false;
        for marker in ["- ", "* ", "+ "] {
            if let Some(rest) = trimmed.strip_prefix(marker) {
                // The item's own text is markdown like any other: bold, code
                // and links inside a bullet have to be parsed, not printed.
                let mut runs = vec![Run::new(Style::Bullet, "  • ")];
                runs.extend(inline(rest));
                lines.push(runs);
                handled = true;
                break;
            }
        }
        if !handled {
            let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() && trimmed[digits.len()..].starts_with(". ") {
                let rest = &trimmed[digits.len() + 2..];
                let mut runs = vec![Run::new(Style::Bullet, format!("  {digits}. "))];
                runs.extend(inline(rest));
                lines.push(runs);
                handled = true;
            }
        }
        if !handled {
            lines.push(paragraph(line, 0));
        }
        index += 1;
    }

    // Drop a trailing blank line.
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The math runs of a rendering, in order.
    fn math_lines(source: &str) -> Vec<String> {
        render(source)
            .iter()
            .flatten()
            .filter(|run| run.style == Style::Math)
            .map(|run| run.text.clone())
            .collect()
    }

    /// Markers are the parser's job to remove: a heading must arrive without
    /// its `##`, and bold without its `**`.
    #[test]
    fn heading_and_bold_markers_are_stripped() {
        let out = render("## Итог\n\n- **важно**: тест");
        let text: String = out
            .iter()
            .flatten()
            .map(|run| run.text.clone())
            .collect::<Vec<_>>()
            .join("|");
        assert!(!text.contains("##"), "heading marker survived: {text}");
        assert!(!text.contains("**"), "bold marker survived: {text}");
        assert!(text.contains("Итог"), "{text}");
        assert!(text.contains("важно"), "{text}");
    }

    /// A table cell is one run, so its inline markers are stripped rather than
    /// printed: `**День**` must read as `День`.
    #[test]
    fn table_cells_lose_their_markers() {
        let out = render("| **День** | `код` |\n| --- | --- |\n| утро | `x` |");
        let text: String = out
            .iter()
            .flatten()
            .map(|run| run.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("**"), "{text}");
        assert!(!text.contains('`'), "{text}");
        assert!(text.contains("День"), "{text}");
    }

    #[test]
    fn bracket_display_math_is_rendered() {
        // The multi-line spelling, as a model writes it.
        let lines = math_lines("Например:\n\n\\[\nax^2 + bx + c = 0\n\\]\n\nпосле");
        assert_eq!(lines, vec!["ax² + bx + c = 0"]);

        // And on one line, with a fraction and a root in it.
        let lines = math_lines(r"\[ x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a} \]");
        let joined = lines.join(" ");
        assert!(joined.contains('±'), "no ±: {joined}");
        assert!(joined.contains('√'), "no √: {joined}");
        assert!(!joined.contains("\\frac"), "raw command left: {joined}");
    }

    #[test]
    fn paren_inline_math_is_rendered() {
        assert_eq!(math_lines(r"radius \(r = 2\) units"), vec!["r = 2"]);
    }

    #[test]
    fn headings_and_emphasis() {
        let out = render("# Title\n\nsome **bold** and *soft*");
        assert!(out[0].iter().any(|r| r.style == Style::Heading && r.text == "Title"));
        assert!(out[2].iter().any(|r| r.style == Style::Strong && r.text == "bold"));
        assert!(out[2].iter().any(|r| r.style == Style::Emphasis && r.text == "soft"));
    }

    #[test]
    fn code_fence_is_literal() {
        let out = render("```rust\nlet x = **1**;\n```");
        assert!(out
            .iter()
            .any(|line| line.iter().any(|r| r.text == "let x = **1**;")));
    }

    #[test]
    fn inline_code_and_links() {
        let out = render("use `cargo build` and [docs](https://example.com)");
        assert!(out[0].iter().any(|r| r.style == Style::Code && r.text == "cargo build"));
        assert!(out[0].iter().any(|r| r.style == Style::Link && r.text == "docs"));
        // The label is what is drawn; the address has to survive for a click.
        let link = out[0]
            .iter()
            .find(|r| r.style == Style::Link)
            .expect("link run");
        assert_eq!(link.url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn pipe_table_gets_borders_and_alignment() {
        let out = render("| name | qty |\n| :--- | ---: |\n| bolt | 3 |");
        let text: Vec<String> = out
            .iter()
            .map(|line| line.iter().map(|r| r.text.clone()).collect())
            .collect();
        assert_eq!(text[0], "┌──────┬─────┐");
        assert_eq!(text[1], "│ name │ qty │");
        assert!(text[3].contains("bolt"));
        // Right-aligned column pads on the left: width is len("qty") = 3.
        assert!(text[3].ends_with("   3 │"), "{:?}", text[3]);
    }

    #[test]
    fn table_is_not_confused_with_a_rule() {
        assert!(render("| a | b |").iter().len() <= 2);
    }

    #[test]
    fn inline_math_is_converted() {
        let out = render("value $\\alpha + \\beta$ here");
        assert!(out[0].iter().any(|r| r.text == "α + β"));
    }

    #[test]
    fn display_math_block() {
        let out = render("$$\n\\frac{1}{2}\n$$");
        assert!(out.iter().any(|line| line.iter().any(|r| r.text == "1⁄2")));
    }
}