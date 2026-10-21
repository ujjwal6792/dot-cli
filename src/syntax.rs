//! Bundled Tree-sitter grammars and terminal styling shared by previews and editing.
use ratatui::prelude::*;
use std::{
    cell::RefCell,
    collections::VecDeque,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

const INK: Color = Color::Rgb(235, 239, 243);
const MINT: Color = Color::Rgb(97, 218, 179);
const RED: Color = Color::Rgb(255, 140, 140);
const AMBER: Color = Color::Rgb(240, 198, 116);
// Large files remain readable without blocking the terminal on syntax queries.
const MAX_BYTES: usize = 256 * 1024;
const CACHE_ENTRIES: usize = 6;
const NAMES: &[&str] = &[
    "comment",
    "string",
    "keyword",
    "function",
    "type",
    "number",
    "boolean",
    "constant",
    "property",
    "variable",
    "operator",
    "punctuation",
    "embedded",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Language {
    Bash,
    Toml,
    Json,
    Yaml,
    Lua,
    Python,
    Rust,
    JavaScript,
    TypeScript,
    Tsx,
    Ini,
}

impl Language {
    fn detect(path: &str, source: &str) -> Option<Self> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let mut name = name.trim_end_matches(".tmpl");
        // Chezmoi attributes are ordered, but stripping in a loop also handles combinations.
        loop {
            let stripped = [
                "private_",
                "executable_",
                "readonly_",
                "encrypted_",
                "dot_",
                "create_",
                "modify_",
            ]
            .iter()
            .find_map(|prefix| name.strip_prefix(prefix));
            match stripped {
                Some(value) => name = value,
                None => break,
            }
        }
        Some(match name.rsplit('.').next().unwrap_or(name) {
            "ini" | "gitconfig" | "gitmodules" | "editorconfig" => Self::Ini,
            "config" if path.contains("ghostty/") || path.contains("git/") => Self::Ini,
            "sh" | "bash" | "zsh" | "bashrc" | "bash_profile" | "bash_login" | "bash_logout"
            | "profile" | "zshrc" | "zshenv" | "zprofile" | "zlogin" | "zlogout" => Self::Bash,
            "toml" => Self::Toml,
            "json" | "jsonc" | "json5" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "lua" => Self::Lua,
            "py" | "pyi" => Self::Python,
            "rs" => Self::Rust,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            _ if source.starts_with("#!") => {
                let shebang = source.lines().next().unwrap_or_default();
                if shebang.contains("python") {
                    Self::Python
                } else if ["/sh", "/bash", "/zsh", " sh", " bash", " zsh"]
                    .iter()
                    .any(|s| shebang.contains(s))
                {
                    Self::Bash
                } else {
                    return None;
                }
            }
            _ => return None,
        })
    }

    fn config(self) -> Option<&'static HighlightConfiguration> {
        static CONFIGS: [OnceLock<Option<HighlightConfiguration>>; 11] =
            [const { OnceLock::new() }; 11];
        CONFIGS[self as usize]
            .get_or_init(|| {
                let (language, query, locals) = match self {
                    Self::Ini => (
                        tree_sitter_ini::LANGUAGE,
                        tree_sitter_ini::HIGHLIGHTS_QUERY.to_owned(),
                        "",
                    ),
                    Self::Bash => (
                        tree_sitter_bash::LANGUAGE,
                        tree_sitter_bash::HIGHLIGHT_QUERY.to_owned(),
                        "",
                    ),
                    Self::Toml => (
                        tree_sitter_toml_ng::LANGUAGE,
                        tree_sitter_toml_ng::HIGHLIGHTS_QUERY.to_owned(),
                        "",
                    ),
                    Self::Json => (
                        tree_sitter_json::LANGUAGE,
                        tree_sitter_json::HIGHLIGHTS_QUERY.to_owned(),
                        "",
                    ),
                    Self::Yaml => (
                        tree_sitter_yaml::LANGUAGE,
                        tree_sitter_yaml::HIGHLIGHTS_QUERY.to_owned(),
                        "",
                    ),
                    Self::Lua => (
                        tree_sitter_lua::LANGUAGE,
                        tree_sitter_lua::HIGHLIGHTS_QUERY.to_owned(),
                        tree_sitter_lua::LOCALS_QUERY,
                    ),
                    Self::Python => (
                        tree_sitter_python::LANGUAGE,
                        tree_sitter_python::HIGHLIGHTS_QUERY.to_owned(),
                        "",
                    ),
                    Self::Rust => (
                        tree_sitter_rust::LANGUAGE,
                        tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
                        "",
                    ),
                    Self::JavaScript => (
                        tree_sitter_javascript::LANGUAGE,
                        format!(
                            "{}\n{}",
                            tree_sitter_javascript::HIGHLIGHT_QUERY,
                            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
                        ),
                        tree_sitter_javascript::LOCALS_QUERY,
                    ),
                    Self::TypeScript | Self::Tsx => (
                        if self == Self::Tsx {
                            tree_sitter_typescript::LANGUAGE_TSX
                        } else {
                            tree_sitter_typescript::LANGUAGE_TYPESCRIPT
                        },
                        format!(
                            "{}\n{}\n{}",
                            tree_sitter_javascript::HIGHLIGHT_QUERY,
                            tree_sitter_typescript::HIGHLIGHTS_QUERY,
                            if self == Self::Tsx {
                                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
                            } else {
                                ""
                            }
                        ),
                        tree_sitter_typescript::LOCALS_QUERY,
                    ),
                };
                let mut config =
                    HighlightConfiguration::new(language.into(), "dot", &query, "", locals).ok()?;
                config.configure(NAMES);
                Some(config)
            })
            .as_ref()
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Mode {
    Source(String),
    Preview,
}
struct Entry {
    mode: Mode,
    source: String,
    text: Text<'static>,
}
thread_local! { static CACHE: RefCell<VecDeque<Entry>> = const { RefCell::new(VecDeque::new()) }; }

fn cached(mode: Mode, source: &str, render: impl FnOnce() -> Text<'static>) -> Text<'static> {
    if source.len() > MAX_BYTES {
        return render();
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(index) = cache
            .iter()
            .position(|e| e.mode == mode && e.source == source)
        {
            let entry = cache.remove(index).unwrap();
            let text = entry.text.clone();
            cache.push_front(entry);
            return text;
        }
        let text = render();
        cache.push_front(Entry {
            mode,
            source: source.into(),
            text: text.clone(),
        });
        cache.truncate(CACHE_ENTRIES);
        text
    })
}

fn token_style(index: usize) -> Style {
    Style::default().fg(match NAMES[index] {
        "comment" => Color::Rgb(139, 148, 163),
        "string" => MINT,
        "keyword" | "boolean" => Color::Rgb(198, 160, 246),
        "function" => Color::Rgb(137, 180, 250),
        "type" | "property" => Color::Rgb(137, 220, 235),
        "number" | "constant" => AMBER,
        "operator" | "embedded" => RED,
        _ => INK,
    })
}

fn plain(source: &str) -> Text<'static> {
    Text::from(
        source
            .split('\n')
            .map(|s| Line::from(s.to_owned()))
            .collect::<Vec<_>>(),
    )
}

fn code(path: &str, source: &str) -> Text<'static> {
    if source.len() > MAX_BYTES || source.contains('\0') {
        return plain(source);
    }
    let Some(config) = Language::detect(path, source).and_then(Language::config) else {
        return plain(source);
    };
    let mut highlighter = Highlighter::new();
    // Highlighter 0.25 does not expose Parser::parse_with_options. Keep its parser
    // timeout until the highlight API supports the replacement progress callback.
    #[allow(deprecated)]
    highlighter.parser().set_timeout_micros(50_000);
    let started = Instant::now();
    let Ok(events) = highlighter.highlight(config, source.as_bytes(), None, |_| None) else {
        return plain(source);
    };
    let mut lines = vec![Line::default()];
    let mut stack = Vec::new();
    for event in events {
        if started.elapsed() > Duration::from_millis(80) {
            return plain(source);
        }
        match event {
            Ok(HighlightEvent::HighlightStart(h)) => stack.push(h.0),
            Ok(HighlightEvent::HighlightEnd) => {
                stack.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                let style = stack.last().map(|i| token_style(*i)).unwrap_or_default();
                for (i, part) in source[start..end].split('\n').enumerate() {
                    if i > 0 {
                        lines.push(Line::default());
                    }
                    if !part.is_empty() {
                        lines
                            .last_mut()
                            .unwrap()
                            .push_span(Span::styled(part.to_owned(), style));
                    }
                }
            }
            Err(_) => return plain(source),
        }
    }
    Text::from(lines)
}

pub(crate) fn source(path: &str, text: &str) -> Text<'static> {
    cached(Mode::Source(path.into()), text, || code(path, text))
}

/// Insert the editor caret after parsing so it cannot change the syntax tree.
pub(crate) fn caret(mut text: Text<'static>, source: &str, byte: usize) -> Text<'static> {
    let prefix = &source[..byte];
    let row = prefix.bytes().filter(|b| *b == b'\n').count();
    let column = prefix.rsplit('\n').next().unwrap_or_default().len();
    while text.lines.len() <= row {
        text.lines.push(Line::default());
    }
    let line = &mut text.lines[row];
    let mut offset = 0;
    for i in 0..line.spans.len() {
        let end = offset + line.spans[i].content.len();
        if column <= end {
            let span = line.spans.remove(i);
            let (before, after) = span.content.split_at(column - offset);
            line.spans.splice(
                i..i,
                [
                    Span::styled(before.to_owned(), span.style),
                    Span::styled("▏", Style::default().fg(AMBER)),
                    Span::styled(after.to_owned(), span.style),
                ],
            );
            return text;
        }
        offset = end;
    }
    line.push_span(Span::styled("▏", Style::default().fg(AMBER)));
    text
}

/// Render dot's preview envelopes; explanatory text remains plain.
pub(crate) fn preview(text: &str) -> Text<'static> {
    cached(Mode::Preview, text, || render_preview(text))
}

fn render_preview(text: &str) -> Text<'static> {
    if let Some(rest) = text.strip_prefix("SOURCE ")
        && let Some((path, body)) = rest.split_once("\n\n")
        && !path.starts_with('→')
    {
        let mut result = plain(&format!("SOURCE {path}\n\n"));
        result.lines.pop();
        result.lines.extend(code(path, body).lines);
        return result;
    }
    if let Some(rest) = text.strip_prefix("UNTRACKED ~/")
        && let Some((path, rest)) = rest.split_once("\n\n")
        && let Some((intro, body)) = rest.split_once("\n\n")
    {
        let mut result = plain(&format!("UNTRACKED ~/{path}\n\n{intro}\n\n"));
        result.lines.pop();
        result.lines.extend(code(path, body).lines);
        return result;
    }
    if let Some(rest) = text.strip_prefix("Replace source for ")
        && let Some((path, body)) = rest.split_once(" with:\n")
    {
        let mut lines = vec![Line::from(format!("Replace source for {path} with:"))];
        lines.extend(code(path, body).lines);
        return Text::from(lines);
    }
    if text.starts_with('{') || text.starts_with('[') {
        return code("record.json", text);
    }
    diff(text)
}

fn diff(text: &str) -> Text<'static> {
    let highlight = text.len() <= MAX_BYTES;
    let render_code = |path: &str, source: &str| {
        if highlight {
            code(path, source)
        } else {
            plain(source)
        }
    };
    let raw: Vec<_> = text.split('\n').collect();
    let mut lines = Vec::new();
    let mut path = "";
    let mut i = 0;
    while i < raw.len() {
        let line = raw[i];
        if let Some(p) = line.strip_prefix("NEW source from HOME: ") {
            lines.push(Line::from(format!("New saved copy: {p}")).fg(INK));
            i += 1;
            let start = i;
            while i < raw.len() && !raw[i].starts_with("NEW source from HOME: ") {
                i += 1;
            }
            lines.extend(render_code(p, &raw[start..i].join("\n")).lines);
            continue;
        }
        if let Some(p) = line
            .strip_prefix("--- ")
            .or_else(|| line.strip_prefix("+++ "))
        {
            let p = p.split('\t').next().unwrap_or(p).trim_matches('"');
            if p != "/dev/null" {
                path = p
                    .strip_prefix("a/")
                    .or_else(|| p.strip_prefix("b/"))
                    .unwrap_or(p);
            }
        }
        if let Some((mut old, mut new)) = hunk_counts(line) {
            lines.push(Line::from(line.to_owned()).fg(AMBER));
            i += 1;
            let start = i;
            let mut before = String::new();
            let mut after = String::new();
            while i < raw.len() && (old > 0 || new > 0) {
                let l = raw[i];
                match l.as_bytes().first() {
                    Some(b' ') if old > 0 && new > 0 => {
                        before.push_str(&l[1..]);
                        before.push('\n');
                        after.push_str(&l[1..]);
                        after.push('\n');
                        old -= 1;
                        new -= 1;
                    }
                    Some(b'-') if old > 0 => {
                        before.push_str(&l[1..]);
                        before.push('\n');
                        old -= 1;
                    }
                    Some(b'+') if new > 0 => {
                        after.push_str(&l[1..]);
                        after.push('\n');
                        new -= 1;
                    }
                    Some(b'\\') => {}
                    _ => break,
                }
                i += 1;
            }
            // Each side is parsed separately to keep multiline strings and removed code valid.
            let before = render_code(path, &before);
            let after = render_code(path, &after);
            let (mut old_row, mut new_row) = (0, 0);
            for l in &raw[start..i] {
                let (mut styled, color, bg) = match l.as_bytes().first() {
                    Some(b'-') => {
                        let line = before.lines[old_row].clone();
                        old_row += 1;
                        (line, RED, Some(Color::Rgb(53, 29, 34)))
                    }
                    Some(b'+') => {
                        let line = after.lines[new_row].clone();
                        new_row += 1;
                        (line, MINT, Some(Color::Rgb(25, 46, 39)))
                    }
                    Some(b' ') => {
                        let line = after.lines[new_row].clone();
                        old_row += 1;
                        new_row += 1;
                        (line, INK, None)
                    }
                    _ => {
                        lines.push(Line::from((*l).to_owned()).fg(INK));
                        continue;
                    }
                };
                styled.spans.insert(
                    0,
                    Span::styled(l[..1].to_owned(), Style::default().fg(color).bold()),
                );
                if let Some(bg) = bg {
                    styled.style = Style::default().fg(color).bg(bg);
                }
                lines.push(styled);
            }
            continue;
        }
        let color =
            if line.starts_with("diff ") || line.starts_with("--- ") || line.starts_with("+++ ") {
                INK
            } else if line.starts_with('+') {
                MINT
            } else if line.starts_with('-') {
                RED
            } else if line.starts_with("@@") {
                AMBER
            } else {
                INK
            };
        lines.push(Line::from(line.to_owned()).fg(color));
        i += 1;
    }
    Text::from(lines)
}

fn hunk_counts(line: &str) -> Option<(usize, usize)> {
    let mut parts = line.strip_prefix("@@ -")?.split_whitespace();
    fn count(range: &str) -> Option<usize> {
        range
            .split_once(',')
            .map_or(Some(1), |(_, n)| n.parse().ok())
    }
    let old = count(parts.next()?)?;
    let new = count(parts.next()?.strip_prefix('+')?)?;
    (parts.next()? == "@@").then_some((old, new))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(text: &Text<'_>) -> String {
        text.lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn colored(text: &Text<'_>, token: &str, color: Color) -> bool {
        text.lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.content.contains(token) && span.style.fg == Some(color))
    }

    #[test]
    fn bundled_grammars_highlight_real_content_without_changing_it() {
        for (path, input) in [
            (
                "private_dot_zshrc.tmpl",
                "# comment\nexport NAME=\"hello\"\n",
            ),
            ("config.toml", "# comment\nname = \"hello\"\n"),
            ("config.jsonc", "{\"name\": \"hello\", \"count\": 42}\n"),
            ("config.yaml", "name: \"hello\"\n"),
            ("init.lua", "local name = \"hello\"\n"),
            ("tool.py", "name = \"hello\"\n"),
            ("main.rs", "fn main() { let name = \"hello\"; }\n"),
            ("config.jsx", "const name = <div title=\"hello\" />;\n"),
            ("config.ts", "const name: string = \"hello\";\n"),
            ("component.tsx", "const name = <div title=\"hello\" />;\n"),
        ] {
            let language = Language::detect(path, input).unwrap();
            assert!(language.config().is_some(), "invalid query for {path}");
            let text = source(path, input);
            assert_eq!(content(&text), input, "text changed for {path}");
            assert!(
                colored(&text, "hello", MINT),
                "missing string highlighting for {path}: {text:?}"
            );
        }
        assert_eq!(Language::detect(".bash_profile", ""), Some(Language::Bash));
        for path in [
            ".gitconfig",
            "dot_gitconfig",
            ".config/ghostty/config",
            "private_dot_config/ghostty/config",
            ".editorconfig",
        ] {
            assert_eq!(Language::detect(path, ""), Some(Language::Ini));
            let input = "[core]\n  editor = vim\n";
            let text = source(path, input);
            assert!(Language::Ini.config().is_some());
            assert_eq!(content(&text), input);
            assert!(
                text.lines
                    .iter()
                    .flat_map(|line| &line.spans)
                    .any(|span| span.style.fg.is_some()),
                "missing INI highlights for {path}"
            );
        }
        assert_eq!(
            Language::detect("bin/tool", "#!/usr/bin/env python3\n"),
            Some(Language::Python)
        );
    }

    #[test]
    fn multiline_unicode_and_caret_preserve_source_and_string_style() {
        let input = "name = \"\"\"héllo\n世界\"\"\"\n";
        let styled = source("config.toml", input);
        assert!(colored(&styled, "世界", MINT));
        for byte in input.char_indices().map(|(i, _)| i).chain([input.len()]) {
            let result = caret(styled.clone(), input, byte);
            let mut expected = input.to_owned();
            expected.insert(byte, '▏');
            assert_eq!(content(&result), expected);
        }
        // Non-source input also permits a caret after a pasted trailing newline.
        assert_eq!(content(&caret(Text::from("x\n"), "x\n", 2)), "x\n▏");
    }

    #[test]
    fn diff_preserves_both_sides_multiline_context_and_multiple_files() {
        let input = "diff --git a/config.toml b/config.toml\n--- a/config.toml\n+++ b/config.toml\n@@ -1,3 +1,3 @@\n name = \"\"\"\n-old\n+new\n end\"\"\"\n--- a/init.lua\n+++ b/init.lua\n@@ -1 +1 @@\n-local x = \"old\"\n+local x = \"new\"\n\\ No newline at end of file";
        let styled = preview(input);
        assert_eq!(content(&styled), input);
        assert!(colored(&styled, "old", MINT));
        assert!(colored(&styled, "new", MINT));
        for line in &styled.lines {
            if let Some(marker) = line.spans.first() {
                if marker.content == "+" {
                    assert_eq!(marker.style.fg, Some(MINT));
                    assert!(line.style.bg.is_some());
                }
                if marker.content == "-" {
                    assert_eq!(marker.style.fg, Some(RED));
                    assert!(line.style.bg.is_some());
                }
            }
        }
        // Content beginning with ++/-- inside a hunk must not be mistaken for a header.
        let tricky = "--- a/config.toml\n+++ b/config.toml\n@@ -1 +1 @@\n---name = \"old\"\n+++name = \"new\"";
        assert_eq!(content(&preview(tricky)), tricky);
    }

    #[test]
    fn preview_envelopes_highlight_code_and_keep_explanations_plain() {
        let source_view = "SOURCE private_dot_zshrc.tmpl\n\nexport NAME=\"hello\"\n";
        let untracked = "UNTRACKED ~/.config/tool.toml\n\nSelect with Space, then Capture to add through chezmoi.\n\nname = \"hello\"\n";
        let edit = "Replace source for .config/tool.toml with:\nname = \"hello\"\n";
        for input in [source_view, untracked, edit] {
            let text = preview(input);
            assert_eq!(content(&text), input);
            assert!(colored(&text, "hello", MINT));
            assert!(
                text.lines[0]
                    .spans
                    .iter()
                    .all(|span| span.style.fg.is_none())
            );
        }
        let capture = "\nNEW source from HOME: init.lua\nlocal name = \"hello\"\n\nNEW source from HOME: config.toml\nname = \"world\"\n";
        let text = preview(capture);
        assert!(colored(&text, "hello", MINT));
        assert!(colored(&text, "world", MINT));
        assert_eq!(
            content(&text),
            capture.replace("NEW source from HOME:", "New saved copy:")
        );
    }

    #[test]
    fn unsupported_large_and_malformed_files_remain_readable() {
        for (path, input) in [
            ("notes.unknown", "hello 🌍\n".to_owned()),
            (
                "large.toml",
                format!("name = \"{}\"\n", "x".repeat(MAX_BYTES)),
            ),
            ("binary.json", "x\0y".to_owned()),
        ] {
            let text = source(path, &input);
            assert_eq!(content(&text), input);
            assert!(
                text.lines
                    .iter()
                    .flat_map(|line| &line.spans)
                    .all(|span| span.style == Style::default())
            );
        }
        let unfinished = "name = \"hello\n";
        assert_eq!(content(&source("config.toml", unfinished)), unfinished);
        let malformed_diff = "@@ -2,5 +2,5 @@\n+one\ninvalid\n@@ invalid\n";
        assert_eq!(content(&preview(malformed_diff)), malformed_diff);
    }

    #[test]
    fn cache_reuses_unchanged_content_but_tracks_edits_and_language() {
        CACHE.with(|cache| cache.borrow_mut().clear());
        let input = "name = \"hello\"";
        let first = source("config.toml", input);
        let reused = cached(Mode::Source("config.toml".into()), input, || {
            panic!("reparsed cached content")
        });
        assert_eq!(first, reused);
        assert!(!colored(&source("notes.txt", input), "hello", MINT));
        assert!(colored(
            &source("config.toml", "name = \"changed\""),
            "changed",
            MINT
        ));
    }
}
