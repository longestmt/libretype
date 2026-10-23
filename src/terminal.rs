// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Terminal prompt extraction. Output is never treated as an editable document.
//! Recognition is deliberately separate from cursor/focus validation in the AX adapter.

use unicode_width::UnicodeWidthStr;

pub fn supported(app: &str) -> bool {
    matches!(app, "com.apple.Terminal" | "com.googlecode.iterm2")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    row: usize,
    input_column: usize,
    columns: Columns,
    pub marker: String,
    // Text immediately before the prompt distinguishes redraws at reused offsets.
    anchor: String,
    // A boxed composer's mode/model row is part of its identity, not context.
    composer: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Prompt {
    pub scope: Scope,
    pub prefix: String,
    pub suffix: String,
    pub automatic: bool,
    /// Actual input start in the source snapshot, independent of projected length.
    pub start_utf16: usize,
    /// Border glyph offsets in the AX value, for independent screen-geometry checks.
    pub edges: Vec<Edges>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Columns {
    left: Option<usize>,
    right: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edges {
    pub left_utf16: Option<usize>,
    pub right_utf16: Option<usize>,
}

fn vertical(ch: char) -> bool {
    matches!(ch, '│' | '┃' | '║')
}
fn borders(line: &str) -> Vec<(usize, usize)> {
    line.char_indices()
        .filter(|(_, c)| vertical(*c))
        .map(|(byte, _)| (byte, line[..byte].width()))
        .collect()
}
fn segment(line: &str, columns: Columns) -> Option<(usize, usize)> {
    if line.len() > 8192
        || line.chars().any(char::is_control)
        || line.chars().filter(|c| vertical(*c)).take(65).count() > 64
    {
        return None;
    }
    let marks = borders(line);
    let left = match columns.left {
        Some(column) => {
            let (byte, _) = marks.iter().find(|(_, col)| *col == column)?;
            byte + line[*byte..].chars().next()?.len_utf8()
        }
        None => 0,
    };
    let right = match columns.right {
        Some(column) => marks.iter().find(|(_, col)| *col == column)?.0,
        None => line.len(),
    };
    (left <= right
        && !line[left..right]
            .chars()
            .any(|c| vertical(c) || matches!(c, '┬' | '┴' | '┼' | '├' | '┤' | '╬' | '╦' | '╩')))
    .then_some((left, right))
}
fn edges(text: &str, line_start: usize, line: &str, columns: Columns) -> Option<Edges> {
    let marks = borders(line);
    let offset = |column: Option<usize>| -> Option<Option<usize>> {
        match column {
            None => Some(None),
            Some(column) => {
                let byte = marks.iter().find(|(_, col)| *col == column)?.0;
                Some(Some(text[..line_start + byte].encode_utf16().count()))
            }
        }
    };
    Some(Edges {
        left_utf16: offset(columns.left)?,
        right_utf16: offset(columns.right)?,
    })
}

fn byte_at_utf16(text: &str, target: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == target {
            return Some(byte);
        }
        units += ch.len_utf16();
        if units > target {
            return None;
        }
    }
    (units == target).then_some(text.len())
}

/// Pi's default idle editor has no prompt glyph. Recognize its complete local
/// frame instead: horizontal borders, working-directory row, then usage/model.
/// Custom editors, widgets and autocomplete lists need separate adapters; a
/// nearby usage string alone must never grant automatic activation. The input
/// span may include multiple complete rows selected by the projection below.
fn pi_frame(text: &str, start: usize, end: usize, columns: Columns) -> bool {
    let Some(previous) = text[..start].strip_suffix('\n') else {
        return false;
    };
    let previous = previous.rsplit('\n').next().unwrap_or("");
    let part = |row: &str| segment(row, columns);
    let border_width = |row: &str| {
        let (a, b) = part(row)?;
        let trimmed = row[a..b].trim_matches(' ');
        let width = trimmed.chars().count();
        (width >= 8 && trimmed.chars().all(|c| c == '─')).then_some((
            row[a..b].len() - row[a..b].trim_start_matches(' ').len(),
            width,
        ))
    };
    let Some(width) = border_width(previous) else {
        return false;
    };
    let Some(tail) = text.get(end..).and_then(|s| s.strip_prefix('\n')) else {
        return false;
    };
    let mut rows = tail.split('\n');
    let Some(bottom) = rows.next() else {
        return false;
    };
    if border_width(bottom) != Some(width) {
        return false;
    }
    for row in text[start..end].split('\n') {
        let Some((a, b)) = part(row) else {
            return false;
        };
        if row[a..b].trim_end_matches(' ').width() > width.0 + width.1 {
            return false;
        }
    }
    let Some(directory) = rows.next() else {
        return false;
    };
    let Some((a, b)) = part(directory) else {
        return false;
    };
    let directory = directory[a..b].trim();
    if !(directory == "~"
        || directory.starts_with("~/")
        || directory.starts_with("~ (")
        || directory.starts_with("~ • ")
        || directory.starts_with('/'))
    {
        return false;
    }
    let Some(stats) = rows.next() else {
        return false;
    };
    let Some((a, b)) = part(stats) else {
        return false;
    };
    let stats = stats[a..b].trim();
    // The model is separated from usage by at least two spaces. Require it even
    // though Pi can omit it in very narrow windows, where identity is ambiguous.
    let Some((usage, model)) = stats.split_once("  ") else {
        return false;
    };
    if model.trim().is_empty() {
        return false;
    }
    let decimal = |s: &str| {
        !s.is_empty()
            && s.len() <= 12
            && s.split('.').count() <= 2
            && s.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    let usage = usage.strip_suffix(" • xp").unwrap_or(usage);
    let usage = usage.strip_suffix(" (auto)").unwrap_or(usage);
    usage.split_whitespace().next_back().is_some_and(|token| {
        let Some((percent, window)) = token.split_once('/') else {
            return false;
        };
        let valid_percent = percent == "?" || percent.strip_suffix('%').is_some_and(decimal);
        let window = window.strip_suffix(['k', 'M']).unwrap_or(window);
        valid_percent && decimal(window)
    })
}

fn pane_text(row: &str, columns: Columns) -> Option<&str> {
    let (a, b) = segment(row, columns)?;
    Some(&row[a..b])
}

/// OpenCode 1.x and Kilo's default composer. The left glyph belongs to the
/// composer, so its footer must be read within the *outer* pane, after validating
/// the composer's closing cap. A neighbor's shortcuts never qualify this input.
fn opencode_frame(text: &str, start: usize, end: usize, columns: Columns) -> Option<(&str, bool)> {
    let column = columns.left?;
    let input = text[start..end].split('\n').next()?;
    let (border_byte, _) = borders(input).into_iter().find(|(_, c)| *c == column)?;
    if !input[border_byte..].starts_with('┃') {
        return None;
    }
    let previous_end = start.checked_sub(1)?;
    let previous_start = text[..previous_end].rfind('\n').map_or(0, |i| i + 1);
    let previous = &text[previous_start..previous_end];
    if !pane_text(previous, columns)?.trim().is_empty() {
        return None;
    }
    if previous_start > 0 {
        let before = text[..previous_start - 1].rsplit('\n').next()?;
        if pane_text(before, columns).is_some() {
            return None; // A blank input row is not the composer's top margin.
        }
    }
    let mut below = text[end..].strip_prefix('\n')?.split('\n');
    if !pane_text(below.next()?, columns)?.trim().is_empty() {
        return None;
    }
    let metadata = pane_text(below.next()?, columns)?.trim();
    if metadata.is_empty() || metadata.len() > 512 {
        return None;
    }
    let outer = Columns {
        left: borders(input)
            .into_iter()
            .filter(|(_, c)| *c < column)
            .map(|(_, c)| c)
            .next_back(),
        right: columns.right,
    };
    let bottom = below.next()?;
    if bottom.len() > 8192 {
        return None;
    }
    if let Some((cap, _)) = bottom
        .char_indices()
        .find(|(i, c)| *c == '╹' && bottom[..*i].width() == column)
    {
        // The cap and border have the same UTF-8/display width. This temporary
        // row validates layout only; actual AX offsets are never changed.
        let mut closed = bottom.to_owned();
        closed.replace_range(cap..cap + '╹'.len_utf8(), "┃");
        let fill = pane_text(&closed, columns)?.trim();
        if fill.chars().count() < 8 || !fill.chars().all(|c| c == '▀') {
            return None;
        }
    } else if !pane_text(bottom, outer)?.trim().is_empty() {
        // With a transparent background, the renderer omits both cap and fill.
        // Still require the complete blank row within these outer pane borders.
        return None;
    }
    let footer = pane_text(below.next()?, outer)?.trim();
    let words: Vec<_> = footer.split_whitespace().collect();
    if metadata == "Shell" {
        return footer
            .ends_with("esc exit shell mode")
            .then_some((metadata, false));
    }
    if metadata.split_whitespace().count() < 2 || metadata.starts_with("Shell ") {
        return None;
    }
    let key = |word: &str| {
        word.len() == 1 && word.is_ascii()
            || matches!(
                word,
                "tab"
                    | "enter"
                    | "return"
                    | "space"
                    | "escape"
                    | "esc"
                    | "up"
                    | "down"
                    | "left"
                    | "right"
            )
            || ["ctrl+", "alt+", "shift+", "super+", "meta+"]
                .iter()
                .any(|p| word.strip_prefix(p).is_some_and(|rest| !rest.is_empty()))
    };
    let commands =
        words.len() >= 2 && words.last() == Some(&"commands") && key(words[words.len() - 2]);
    let agents = words
        .windows(2)
        .any(|pair| key(pair[0]) && pair[1] == "agents");
    let usage = words.iter().any(|word| {
        word.strip_prefix('(')
            .and_then(|s| s.strip_suffix("%)"))
            .is_some_and(|n| !n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit()))
    });
    (commands && (agents || usage)).then_some((metadata, true))
}

const INPUT_ROWS: usize = 16;
const WINDOW_ROWS: usize = INPUT_ROWS + 4;

#[derive(Clone, Copy)]
struct Row {
    start: usize,
    end: usize,
}
impl Row {
    fn text(self, text: &str) -> &str {
        &text[self.start..self.end]
    }
}

#[derive(Clone, Copy)]
enum Decoration {
    Spaces(usize),
    Prompt { indent: usize, marker: &'static str },
}
impl Decoration {
    fn offset(self, text: &str, first: bool) -> Option<usize> {
        let count = match self {
            Self::Spaces(count) => count,
            Self::Prompt { indent, marker } if first => {
                return text
                    .strip_prefix(&" ".repeat(indent))?
                    .starts_with(marker)
                    .then_some(indent + marker.len());
            }
            Self::Prompt { indent, .. } => indent + 2,
        };
        if text.bytes().all(|b| b == b' ') {
            return Some(count.min(text.len()));
        }
        text.starts_with(&" ".repeat(count)).then_some(count)
    }
}
struct Frame {
    top: usize,
    bottom: usize,
    begin: usize,
    last: usize,
    composer: Option<String>,
    normal: bool,
    decoration: Decoration,
}

fn codex_footer(text: &str) -> bool {
    let mut words = text.split_whitespace().rev();
    words.next() == Some("left")
        && words.next() == Some("context")
        && words
            .next()
            .and_then(|s| s.strip_suffix('%'))
            .is_some_and(|n| {
                !n.is_empty()
                    && n.len() <= 3
                    && n.bytes().all(|b| b.is_ascii_digit())
                    && n.parse::<u16>().is_ok_and(|n| n <= 100)
            })
}

/// Codex's standard composer has one first-row arrow, a two-column continuation
/// gutter, a blank bottom margin and a context footer. Trailing blank rows beyond
/// the cursor are ambiguous (input versus padding), so omit those from context.
fn codex_frame(
    text: &str,
    rows: &[Row],
    active: usize,
    columns: Columns,
) -> Result<Option<Frame>, ()> {
    let part = |i: usize| pane_text(rows[i].text(text), columns);
    let Some((begin, _)) = (0..=active)
        .filter_map(|i| {
            let row = part(i)?;
            let trimmed = row.trim_start_matches(' ');
            trimmed
                .starts_with("› ")
                .then_some((i, row.len() - trimmed.len()))
        })
        .min_by_key(|(i, indent)| (*indent, std::cmp::Reverse(*i)))
    else {
        return Ok(None);
    };
    let top = begin.checked_sub(1);
    let first = part(begin).ok_or(())?;
    let indent = first.len() - first.trim_start_matches(' ').len();
    let mut footer = None;
    for i in begin + 1..rows.len() {
        let Some(row) = part(i) else { break };
        if row
            .get(indent..)
            .is_some_and(|s| s.starts_with("› ") || s.starts_with("❯ "))
        {
            break;
        }
        if i > active && codex_footer(row) {
            if !part(i - 1).is_some_and(|s| s.trim().is_empty()) {
                if i == begin + 1 {
                    return Ok(None); // Preserve the existing compact single-row adapter.
                }
                return Err(());
            }
            footer = Some(i);
            break;
        }
    }
    let Some(footer) = footer else {
        return Ok(None);
    };
    if active >= footer.saturating_sub(1) {
        return Err(());
    }
    let last = (begin..footer)
        .rev()
        .find(|&i| part(i).is_some_and(|s| !s.trim().is_empty()))
        .ok_or(())?
        .max(active);
    let top = match top.filter(|&i| part(i).is_some_and(|s| s.trim().is_empty())) {
        Some(top) => top,
        None if last > begin => return Err(()),
        None => return Ok(None),
    };
    let decoration = Decoration::Prompt {
        indent,
        marker: "› ",
    };
    for i in begin..=footer {
        let row = part(i).ok_or(())?;
        let lower = row.to_lowercase();
        if [
            "reverse-i-search",
            "esc cancel",
            "enter accept",
            "esc close",
            "select an option",
            "↑/↓ select",
        ]
        .iter()
        .any(|s| lower.contains(s))
        {
            return Err(());
        }
        if i <= last {
            decoration.offset(row, i == begin).ok_or(())?;
            if row.trim_end().ends_with(['↑', '↓']) {
                return Err(());
            }
        }
    }
    Ok(Some(Frame {
        top,
        bottom: footer,
        begin,
        last,
        composer: Some("Codex".into()),
        normal: true,
        decoration,
    }))
}

/// Claude's separator-framed prompt with a two-column continuation gutter.
/// This adapter requires the entire frame and a local idle/mode hint. Custom,
/// clipped, modal and collapsed-paste layouts need separate verification.
fn claude_frame(
    text: &str,
    rows: &[Row],
    active: usize,
    columns: Columns,
) -> Result<Option<Frame>, ()> {
    let part = |i: usize| pane_text(rows[i].text(text), columns);
    let Some((begin, indent)) = (0..=active)
        .filter_map(|i| {
            let row = part(i)?;
            let trimmed = row.trim_start_matches(' ');
            trimmed
                .starts_with("❯ ")
                .then_some((i, row.len() - trimmed.len()))
        })
        .min_by_key(|(i, indent)| (*indent, std::cmp::Reverse(*i)))
    else {
        return Ok(None);
    };
    let border = |i: usize| {
        let row = part(i)?;
        let trimmed = row.trim_matches(' ');
        (trimmed.chars().count() >= 8 && trimmed.chars().all(|c| c == '─')).then_some((
            row.len() - row.trim_start_matches(' ').len(),
            trimmed.width(),
        ))
    };
    let top = begin.checked_sub(1).filter(|&i| border(i).is_some());
    let Some(bottom) =
        (active + 1..rows.len()).find(|&i| border(i).is_some_and(|(column, _)| column == indent))
    else {
        return if top.is_some() { Err(()) } else { Ok(None) };
    };
    let Some(footer) = (bottom + 1..rows.len().min(bottom + 4)).find(|&i| {
        part(i).is_some_and(|s| s.contains("? for shortcuts") || s.contains("(shift+tab to cycle)"))
    }) else {
        return if top.is_some() { Err(()) } else { Ok(None) };
    };
    let last = bottom - 1;
    let Some(top) = top else {
        return if last == begin { Ok(None) } else { Err(()) };
    };
    let (border_indent, width) = border(top).ok_or(())?;
    if border(bottom) != Some((border_indent, width)) || indent != border_indent {
        return Err(());
    }
    let decoration = Decoration::Prompt {
        indent,
        marker: "❯ ",
    };
    for i in begin..=last {
        let row = part(i).ok_or(())?;
        let offset = decoration.offset(row, i == begin).ok_or(())?;
        let input = &row[offset..];
        if row.trim_end().width() > indent + width
            || row.trim_end().ends_with(['↑', '↓'])
            || input.contains("[Pasted text")
            || input.contains("[Image #")
        {
            return Err(());
        }
    }
    for i in bottom + 1..=footer {
        let row = part(i).ok_or(())?.to_lowercase();
        if [
            "reverse-i-search",
            "esc cancel",
            "esc to cancel",
            "enter accept",
            "enter to confirm",
            "esc close",
            "select an option",
            "↑/↓ select",
        ]
        .iter()
        .any(|s| row.contains(s))
            || row.trim_start().starts_with(['❯', '›', '$', '%'])
        {
            return Err(());
        }
    }
    Ok(Some(Frame {
        top,
        bottom: footer,
        begin,
        last,
        composer: Some("Claude".into()),
        normal: true,
        decoration,
    }))
}

/// Project a completely framed, visible multi-row input into prompt text. Row
/// separators are retained as newlines; these terminal APIs cannot distinguish
/// an application's hard newline from its own visual word wrapping.
fn multiline(
    text: &str,
    cursor: usize,
    current: Row,
    columns: Columns,
) -> Option<Result<Prompt, ()>> {
    let mut rows = vec![current];
    let mut first = current.start;
    for _ in 0..WINDOW_ROWS {
        let Some(end) = first.checked_sub(1) else {
            break;
        };
        let start = text[..end].rfind('\n').map_or(0, |i| i + 1);
        rows.push(Row { start, end });
        first = start;
        if start == 0 {
            break;
        }
    }
    rows.reverse();
    let active = rows.len() - 1;
    let mut end = current.end;
    for _ in 0..WINDOW_ROWS {
        if end >= text.len() {
            break;
        }
        let start = end + 1;
        end = text[start..].find('\n').map_or(text.len(), |i| start + i);
        rows.push(Row { start, end });
    }
    let part = |i: usize| pane_text(rows[i].text(text), columns);
    let horizontal = |i: usize| {
        part(i).is_some_and(|s| {
            let s = s.trim_matches(' ');
            s.chars().count() >= 8 && s.chars().all(|c| c == '─')
        })
    };
    let pi = (0..active).rev().find(|&i| horizontal(i)).and_then(|top| {
        let bottom = (active + 1..rows.len()).find(|&i| horizontal(i))?;
        let begin = top + 1;
        let last = bottom.checked_sub(1)?;
        (begin <= active
            && active <= last
            && pi_frame(text, rows[begin].start, rows[last].end, columns))
        .then_some((top, bottom, begin, last, None, true, 0))
    });
    let framed = pi.or_else(|| {
        columns.left?;
        let mut top = active;
        while top > 0 && part(top - 1).is_some() {
            top -= 1;
        }
        let mut bottom = active;
        while bottom + 1 < rows.len() && part(bottom + 1).is_some() {
            bottom += 1;
        }
        let begin = top + 1;
        let last = bottom.checked_sub(2)?;
        if begin > active || active > last {
            return None;
        }
        let (metadata, normal) = opencode_frame(text, rows[begin].start, rows[last].end, columns)?;
        Some((
            top,
            bottom,
            begin,
            last,
            Some(metadata.to_owned()),
            normal,
            2,
        ))
    });
    let frame = if let Some((top, bottom, begin, last, composer, normal, padding)) = framed {
        Frame {
            top,
            bottom,
            begin,
            last,
            composer,
            normal,
            decoration: Decoration::Spaces(padding),
        }
    } else {
        match claude_frame(text, &rows, active, columns).and_then(|frame| match frame {
            Some(frame) => Ok(Some(frame)),
            None => codex_frame(text, &rows, active, columns),
        }) {
            Ok(Some(frame)) => frame,
            Ok(None) => return None,
            Err(()) => return Some(Err(())),
        }
    };
    let count = frame.last.checked_sub(frame.begin)? + 1;
    if count > INPUT_ROWS {
        return Some(Err(()));
    }
    if count == 1 && matches!(frame.decoration, Decoration::Spaces(_)) {
        return None;
    }
    Some(project_frame(text, cursor, &rows, active, columns, frame).ok_or(()))
}

fn project_frame(
    text: &str,
    cursor: usize,
    rows: &[Row],
    active: usize,
    columns: Columns,
    frame: Frame,
) -> Option<Prompt> {
    let Frame {
        top,
        bottom,
        begin,
        last,
        composer,
        normal,
        decoration,
    } = frame;
    let part = |i: usize| pane_text(rows[i].text(text), columns);
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut source_start = None;
    let mut input_column = 0;
    let mut marker = String::new();
    for (i, row) in rows.iter().enumerate().take(last + 1).skip(begin) {
        let raw = row.text(text);
        let (a, b) = segment(raw, columns)?;
        let skip = decoration.offset(&raw[a..b], i == begin)?;
        if i == begin {
            marker = raw[a..a + skip].to_owned();
        }
        let a = a + skip;
        if i == begin {
            source_start = Some(row.start + a);
            input_column = raw[..a].width();
        }
        if i < active {
            prefix.push_str(raw[a..b].trim_end_matches(' '));
            prefix.push('\n');
        } else if i == active {
            let position = cursor.checked_sub(row.start)?;
            if position < a || position > b {
                return None;
            }
            prefix.push_str(&raw[a..position]);
            suffix.push_str(raw[position..b].trim_end_matches(' '));
        } else {
            suffix.push('\n');
            suffix.push_str(raw[a..b].trim_end_matches(' '));
        }
    }
    let first_input = (begin..=last)
        .filter_map(|i| {
            let row = part(i)?;
            Some(&row[decoration.offset(row, i == begin)?..])
        })
        .map(str::trim_start)
        .find(|s| !s.is_empty())
        .unwrap_or("");
    let automatic = normal
        && !first_input.starts_with(['/', '!'])
        && !(prefix
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace())
            && prefix
                .split_whitespace()
                .next_back()
                .is_some_and(|s| s.starts_with('@')));
    let mut proof = Vec::new();
    if columns != Columns::default() {
        // Native geometry requires the caret's row first, then every other row
        // exactly once. This includes the whole frame, not just nearby borders.
        for i in std::iter::once(active).chain((top..=bottom).filter(|i| *i != active)) {
            proof.push(edges(text, rows[i].start, rows[i].text(text), columns)?);
        }
    }
    let source_start = source_start?;
    Some(Prompt {
        scope: Scope {
            row: text[..rows[begin].start]
                .bytes()
                .filter(|b| *b == b'\n')
                .count(),
            input_column,
            columns,
            marker,
            anchor: part(top)?.chars().take(128).collect(),
            composer,
        },
        prefix,
        suffix,
        automatic,
        start_utf16: text[..source_start].encode_utf16().count(),
        edges: proof,
    })
}

/// Extract the prompt around a validated cursor, isolating a bordered pane when
/// present. Every returned edge must also pass the native screen-coordinate check.
pub fn extract(text: &str, cursor_utf16: usize) -> Option<Prompt> {
    if text.len() > 256 * 1024 {
        return None;
    }
    let cursor = byte_at_utf16(text, cursor_utf16)?;
    let start = text[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let end = text[cursor..].find('\n').map_or(text.len(), |i| cursor + i);
    let full_line = &text[start..end];
    if full_line.len() > 8192
        || full_line.chars().any(char::is_control)
        || full_line.chars().filter(|c| vertical(*c)).take(65).count() > 64
    {
        return None;
    }
    let marks = borders(full_line);
    let relative_cursor = cursor - start;
    // A cursor on the border is not an input position.
    if marks.iter().any(|(byte, _)| *byte == relative_cursor) {
        return None;
    }
    let columns = Columns {
        left: marks
            .iter()
            .rev()
            .find(|(byte, _)| *byte < relative_cursor)
            .map(|(_, col)| *col),
        right: marks
            .iter()
            .find(|(byte, _)| *byte > relative_cursor)
            .map(|(_, col)| *col),
    };
    let pane = columns != Columns::default();
    let (left, right) = segment(full_line, columns)?;
    if let Some(prompt) = multiline(text, cursor, Row { start, end }, columns) {
        return prompt.ok();
    }
    let line = &full_line[left..right];
    let mut proof = Vec::new();
    if pane {
        proof.push(edges(text, start, full_line, columns)?);
    }
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    let pi = pi_frame(text, start, end, columns);
    let opencode = opencode_frame(text, start, end, columns);
    let agent_marker = ["› ", "❯ "].iter().find(|p| trimmed.starts_with(**p));
    let marker_end = if pi || opencode.is_some() {
        indent
    } else if let Some(marker) = agent_marker {
        indent + marker.len()
    } else {
        line.char_indices()
            .find(|(i, c)| {
                matches!(c, '$' | '%' | '#' | '>') && line[*i + c.len_utf8()..].starts_with(' ')
            })
            .map_or(indent, |(i, c)| i + c.len_utf8() + 1)
    };
    let input_start = start + left + marker_end;
    if cursor < input_start || cursor > start + right {
        return None;
    }
    let prefix = &text[input_start..cursor];
    let suffix = text[cursor..start + right].trim_end_matches(' ');

    // Only inspect matching columns below this cursor. Crossing a junction,
    // border change, or another prompt ends the footer search.
    let mut footer = String::new();
    let mut offset = (end + 1).min(text.len());
    for next in text[offset..].split('\n').take(4) {
        if next.len() > 8192 {
            break;
        }
        let Some((a, b)) = segment(next, columns) else {
            break;
        };
        let part = &next[a..b];
        if part
            .chars()
            .any(|c| matches!(c, '┬' | '┴' | '┼' | '├' | '┤' | '╬' | '╦' | '╩'))
            || ["› ", "❯ ", "$ ", "% "]
                .iter()
                .any(|p| part.trim_start().starts_with(p))
        {
            break;
        }
        if pane {
            proof.push(edges(text, offset, next, columns)?);
        }
        footer.push_str(part);
        footer.push('\n');
        offset += next.len() + 1;
    }
    // Require an independently aligned neighboring row, not a literal box glyph
    // typed into a standalone prompt. Include only this pane in its identity.
    let previous = if start > 0 {
        let prev_end = start - 1;
        let prev_start = text[..prev_end].rfind('\n').map_or(0, |i| i + 1);
        let prev = &text[prev_start..prev_end];
        if prev.len() <= 8192 {
            segment(prev, columns).map(|(a, b)| (prev_start, prev, &prev[a..b]))
        } else {
            None
        }
    } else {
        None
    };
    if pane {
        let mut before = start;
        for _ in 0..3 {
            if before == 0 {
                break;
            }
            let prev_end = before - 1;
            let prev_start = text[..prev_end].rfind('\n').map_or(0, |i| i + 1);
            let prev = &text[prev_start..prev_end];
            if prev.len() > 8192 || segment(prev, columns).is_none() {
                break;
            }
            proof.push(edges(text, prev_start, prev, columns)?);
            before = prev_start;
        }
        if proof.len() < 2 {
            return None;
        }
    }
    // Use the adjacent logical row, not an arbitrary scrollback tail. A bounded
    // read may drop older rows as the cursor wraps without changing this prompt.
    let anchor = previous
        .map_or("", |(_, _, part)| part)
        .chars()
        .rev()
        .take(128)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let footer = footer.to_lowercase();
    let modal = [
        "reverse-i-search",
        "esc cancel",
        "enter accept",
        "select an option",
    ]
    .iter()
    .any(|s| footer.contains(s));
    if modal {
        return None;
    }
    let automatic = (pi && !trimmed.starts_with(['/', '!']))
        || (opencode.is_some_and(|(_, normal)| normal)
            && !trimmed.starts_with(['/', '!'])
            && !(prefix
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace())
                && prefix
                    .split_whitespace()
                    .next_back()
                    .is_some_and(|s| s.starts_with('@'))))
        || (agent_marker.is_some()
            && (footer.contains("? for shortcuts")
                || (trimmed.starts_with("› ") && footer.lines().any(codex_footer))));
    Some(Prompt {
        scope: Scope {
            row: text[..start].bytes().filter(|b| *b == b'\n').count(),
            input_column: full_line[..left + marker_end].width(),
            columns,
            marker: line[..marker_end].into(),
            anchor,
            composer: opencode.map(|(metadata, _)| metadata.to_owned()),
        },
        prefix: prefix.into(),
        suffix: suffix.into(),
        automatic,
        start_utf16: text[..input_start].encode_utf16().count(),
        edges: proof,
    })
}

/// Absolute UTF-16 coordinates in the terminal accessibility buffer. A window
/// stays small even when the terminal retains megabytes of preceding output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRange {
    pub start: usize,
    pub length: usize,
}
impl TextRange {
    pub fn end(self) -> Option<usize> {
        self.start.checked_add(self.length)
    }
    pub fn contains(self, cursor: usize) -> bool {
        self.end()
            .is_some_and(|end| self.start <= cursor && cursor <= end)
    }
}

pub fn window_from_lines(
    cursor: usize,
    current: TextRange,
    first: TextRange,
    last: TextRange,
) -> Option<TextRange> {
    let end = last.end()?;
    if !current.contains(cursor)
        || first.start > current.start
        || first.end()? > current.end()?
        || last.start < current.start
        || last.end()? < current.end()?
    {
        return None;
    }
    let length = end.checked_sub(first.start)?;
    (length <= 256 * 1024).then_some(TextRange {
        start: first.start,
        length,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowPlan {
    Range(TextRange),
    Legacy,
}

/// Plan a bounded AX read; only an explicitly unsupported current-line API can
/// select the legacy full-value path. Other failures never widen the read.
pub fn plan_window(
    cursor: usize,
    line: usize,
    mut range_for_line: impl FnMut(usize) -> Result<TextRange, i32>,
) -> Option<WindowPlan> {
    let current = match range_for_line(line) {
        Ok(range) => range,
        Err(-25205 | -25212) => return Some(WindowPlan::Legacy),
        Err(_) => return None,
    };
    let first = if line > 0 {
        range_for_line(line.saturating_sub(WINDOW_ROWS)).ok()?
    } else {
        current
    };
    let mut last = current;
    for distance in (1..=WINDOW_ROWS).rev() {
        match range_for_line(line.checked_add(distance)?) {
            Ok(range) => {
                last = range;
                break;
            }
            Err(-25201 | -25212) => continue,
            Err(_) => return None,
        }
    }
    window_from_lines(cursor, current, first, last).map(WindowPlan::Range)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub window: TextRange,
    pub text: String,
}
impl Snapshot {
    pub fn prompt(
        &self,
        cursor: usize,
        line_for_index: impl FnOnce(usize) -> Option<usize>,
    ) -> Option<Prompt> {
        if self.text.len() > 256 * 1024
            || self.text.encode_utf16().count() != self.window.length
            || !self.window.contains(cursor)
        {
            return None;
        }
        let mut prompt = extract(&self.text, cursor.checked_sub(self.window.start)?)?;
        // Anchor to the prompt's beginning, not the cursor's physical row:
        // accepting a word can wrap the cursor onto the next screen row.
        let input_start = self.window.start.checked_add(prompt.start_utf16)?;
        prompt.scope.row = line_for_index(input_start)?;
        prompt.start_utf16 = input_start;
        for edge in &mut prompt.edges {
            if let Some(offset) = edge.left_utf16 {
                edge.left_utf16 = Some(self.window.start.checked_add(offset)?);
            }
            if let Some(offset) = edge.right_utf16 {
                edge.right_utf16 = Some(self.window.start.checked_add(offset)?);
            }
        }
        Some(prompt)
    }
}

pub fn cancels_manual(key: i64, modifiers: u64) -> bool {
    matches!(key, 36 | 76 | 53) // Return, keypad Enter, Escape
        || modifiers & (crate::shortcuts::COMMAND | crate::shortcuts::CONTROL) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(text: &str, before: &str) -> Prompt {
        extract(text, text.find(before).unwrap() + before.len()).unwrap()
    }
    fn split_rows(rows: &[(&str, &str)], width: usize) -> String {
        rows.iter()
            .map(|(l, r)| format!("{l}{}│{r}", " ".repeat(width - l.width())))
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn after(text: &str, needle: &str) -> Prompt {
        let end = text.find(needle).unwrap() + needle.len();
        extract(text, text[..end].encode_utf16().count()).unwrap()
    }
    fn automatic_after(text: &str, needle: &str) -> bool {
        let end = text.find(needle).unwrap() + needle.len();
        extract(text, text[..end].encode_utf16().count()).is_some_and(|p| p.automatic)
    }
    #[test]
    fn advancing_a_window_past_old_output_keeps_the_prompt_anchor() {
        let discarded = "older unrelated output\n";
        let tail = "adjacent output\n› Thank you for your ";
        let text = format!("{discarded}{tail}\n90% context left");
        let start = 500_000;
        let cursor = start + discarded.encode_utf16().count() + tail.encode_utf16().count();
        let first = Snapshot {
            window: TextRange {
                start,
                length: text.encode_utf16().count(),
            },
            text,
        };
        let expected_input = cursor - "Thank you for your ".encode_utf16().count();
        let p = first
            .prompt(cursor, |index| {
                assert_eq!(index, expected_input);
                Some(200)
            })
            .unwrap();
        let text = format!("{tail}help\n90% context left");
        let next = Snapshot {
            window: TextRange {
                start: start + discarded.encode_utf16().count(),
                length: text.encode_utf16().count(),
            },
            text,
        };
        let q = next
            .prompt(cursor + 4, |index| {
                assert_eq!(index, expected_input);
                Some(200)
            })
            .unwrap();
        assert_eq!(p.scope, q.scope);
        assert_eq!(q.prefix, "Thank you for your help");
    }

    #[test]
    fn range_planning_bounds_queries_near_the_first_and_last_screen_lines() {
        let mut queries = Vec::new();
        let plan = plan_window(15, 1, |line| {
            queries.push(line);
            if line > 2 {
                return Err(-25201);
            }
            Ok(TextRange {
                start: line * 10,
                length: 10,
            })
        });
        assert_eq!(
            queries,
            [1, 0].into_iter().chain((2..=21).rev()).collect::<Vec<_>>()
        );
        assert_eq!(
            plan,
            Some(WindowPlan::Range(TextRange {
                start: 0,
                length: 30
            }))
        );
        let mut queries = Vec::new();
        let plan = plan_window(9_000_010, 90_000, |line| {
            queries.push(line);
            Ok(TextRange {
                start: line * 100,
                length: 100,
            })
        });
        assert_eq!(queries, [90_000, 89_980, 90_020]);
        assert_eq!(
            plan,
            Some(WindowPlan::Range(TextRange {
                start: 8_998_000,
                length: 4100
            }))
        );
        assert_eq!(plan_window(0, 0, |_| Err(-25205)), Some(WindowPlan::Legacy));
        assert!(plan_window(0, 0, |_| Err(-25204)).is_none());
        assert!(plan_window(0, 0, |line| if line == 0 {
            Ok(TextRange {
                start: 0,
                length: 10,
            })
        } else {
            Err(-25204)
        })
        .is_none());
        assert!(plan_window(0, usize::MAX, |_| Ok(TextRange {
            start: 0,
            length: 10
        }))
        .is_none());
    }

    #[test]
    fn bounded_window_preserves_large_absolute_unicode_offsets_and_acceptance() {
        use crate::acceptance::{AcceptancePlan, Observation};
        let text = split_rows(
            &[
                ("other", "active output"),
                ("👩‍💻", "› Thank you for your "),
                ("", "90% context left"),
            ],
            24,
        );
        let before =
            &text[..text.find("› Thank you for your ").unwrap() + "› Thank you for your ".len()];
        let start = 4_000_003;
        let cursor = start + before.encode_utf16().count();
        let window = TextRange {
            start,
            length: text.encode_utf16().count(),
        };
        let snapshot = Snapshot {
            window,
            text: text.clone(),
        };
        let prompt = snapshot
            .prompt(cursor, |index| {
                assert_eq!(index, cursor - "Thank you for your ".encode_utf16().count());
                Some(500_000)
            })
            .unwrap();
        assert!(prompt.automatic);
        assert_eq!(prompt.prefix, "Thank you for your ");
        let raw_prompt = extract(&text, cursor - start).unwrap();
        for (absolute, local) in prompt.edges.iter().zip(&raw_prompt.edges) {
            assert_eq!(absolute.left_utf16, local.left_utf16.map(|n| start + n));
            assert_eq!(absolute.right_utf16, local.right_utf16.map(|n| start + n));
        }
        let mut plan =
            AcceptancePlan::new(&prompt.prefix, "", "help today.", false, true, false).unwrap();
        assert_eq!(plan.next_chunk().unwrap(), "help");
        let next_text = text.replace("Thank you for your ", "Thank you for your help");
        let next = Snapshot {
            window: TextRange {
                start,
                length: next_text.encode_utf16().count(),
            },
            text: next_text,
        };
        let observed = next.prompt(cursor + 4, |_| Some(500_000)).unwrap();
        assert_eq!(observed.scope, prompt.scope);
        assert_eq!(
            plan.observe(&observed.prefix, &observed.suffix),
            Observation::Confirmed
        );
        assert_eq!(plan.remainder, " today.");
        assert_ne!(
            next.prompt(cursor + 4, |_| Some(500_001)).unwrap().scope,
            prompt.scope
        );
        let shifted = Snapshot {
            window: TextRange {
                start: start + 2,
                length: window.length,
            },
            text,
        };
        assert_eq!(
            shifted.prompt(cursor + 2, |_| Some(500_000)).unwrap().scope,
            prompt.scope
        );
    }

    #[test]
    fn window_validation_rejects_truncated_strings_bad_ranges_and_surrogate_cursors() {
        let current = TextRange {
            start: 1_000_000,
            length: 30,
        };
        let first = TextRange {
            start: 999_950,
            length: 20,
        };
        let last = TextRange {
            start: 1_000_040,
            length: 20,
        };
        assert_eq!(
            window_from_lines(1_000_010, current, first, last),
            Some(TextRange {
                start: 999_950,
                length: 110
            })
        );
        assert!(window_from_lines(999_999, current, first, last).is_none());
        assert!(window_from_lines(1_000_010, current, last, first).is_none());
        assert!(window_from_lines(
            1_000_010,
            current,
            TextRange {
                start: 0,
                length: 1
            },
            last
        )
        .is_none());
        assert!(window_from_lines(
            1_000_010,
            current,
            first,
            TextRange {
                start: usize::MAX,
                length: 9
            }
        )
        .is_none());
        let text = "output\n› Ask 🦀\n90% context left";
        let range = TextRange {
            start: 999,
            length: text.encode_utf16().count(),
        };
        let snapshot = Snapshot {
            window: range,
            text: text.into(),
        };
        let crab = text.find('🦀').unwrap();
        assert!(snapshot
            .prompt(999 + text[..crab].encode_utf16().count() + 1, |_| Some(2))
            .is_none());
        assert!(snapshot.prompt(998, |_| Some(2)).is_none());
        assert!(snapshot
            .prompt(range.end().unwrap() + 1, |_| Some(2))
            .is_none());
        let truncated = Snapshot {
            window: range,
            text: "output".into(),
        };
        assert!(truncated.prompt(1000, |_| Some(2)).is_none());
    }

    #[test]
    fn split_panes_use_only_their_own_prompt_and_footer() {
        let text = split_rows(
            &[
                ("left output", "right output"),
                ("› Explain left ", "› Explain right "),
                ("", ""),
                ("90% context left", "ordinary output"),
            ],
            28,
        );
        let left = after(&text, "› Explain left ");
        assert_eq!(left.prefix, "Explain left ");
        assert!(left.suffix.is_empty());
        assert!(left.automatic);
        let right = after(&text, "› Explain right ");
        assert_eq!(right.prefix, "Explain right ");
        assert!(!right.automatic); // The neighboring Codex footer grants nothing.
        assert_ne!(left.scope, right.scope);
        assert!(left
            .edges
            .iter()
            .all(|e| e.left_utf16.is_none() && e.right_utf16.is_some()));
        assert!(right
            .edges
            .iter()
            .all(|e| e.left_utf16.is_some() && e.right_utf16.is_none()));
    }
    #[test]
    fn unicode_neighbor_redraws_keep_active_pane_identity_and_remainder() {
        use crate::acceptance::{AcceptancePlan, Observation};
        let render = |neighbor: &str, input: &str| {
            split_rows(
                &[
                    (neighbor, "active output"),
                    (neighbor, input),
                    (neighbor, ""),
                    (neighbor, "90% context left"),
                ],
                24,
            )
        };
        let original = after(
            &render("👩‍💻", "› Thank you for your "),
            "› Thank you for your ",
        );
        let changed = after(
            &render("界e\u{301}", "› Thank you for your "),
            "› Thank you for your ",
        );
        assert_eq!(original.scope, changed.scope);
        assert_eq!(original.prefix, changed.prefix);
        // UTF-16 indices really changed; identity is not an absolute text offset.
        assert_ne!(original.edges, changed.edges);
        let mut plan =
            AcceptancePlan::new(&original.prefix, "", "help today.", false, true, false).unwrap();
        assert_eq!(plan.next_chunk().unwrap(), "help");
        let accepted = after(
            &render("updated output", "› Thank you for your help"),
            "› Thank you for your help",
        );
        assert_eq!(original.scope, accepted.scope);
        assert_eq!(
            plan.observe(&accepted.prefix, &accepted.suffix),
            Observation::Confirmed
        );
        assert_eq!(plan.remainder, " today.");
        let resized = split_rows(
            &[
                ("x", "active output"),
                ("x", "› Thank you for your help"),
                ("x", "90% context left"),
            ],
            25,
        );
        assert_ne!(
            original.scope,
            after(&resized, "› Thank you for your help").scope
        );
    }
    #[test]
    fn nested_borders_and_junctions_do_not_mix_pane_footers() {
        let text = "│output                 │other\n│› Explain this         │› Another prompt\n│                       │\n│90% context left       │";
        let p = after(text, "› Explain this");
        assert!(p.automatic);
        assert!(p
            .edges
            .iter()
            .all(|e| e.left_utf16.is_some() && e.right_utf16.is_some()));
        let junction = "│output                 │\n│› Explain this         │\n├───────────────────────┤\n│90% context left       │";
        assert!(!after(junction, "› Explain this").automatic);
        let above = "│upper                  │\n│───────────┴───────────│\n│› Explain this         │\n│90% context left       │";
        let below_split = after(above, "› Explain this");
        // The screen region must not extend above the horizontal subdivision.
        assert_eq!(below_split.edges.len(), 2);
        assert!(extract(
            "› Explain this\nreverse-i-search: x enter accept · esc cancel\n90% context left",
            "› Explain this".encode_utf16().count()
        )
        .is_none());
    }
    #[test]
    fn ordinary_shells_require_manual_activation_and_strip_prompt() {
        let p = at(
            "prior output\nuser@host % Thank you for your ",
            "Thank you for your ",
        );
        assert_eq!(p.prefix, "Thank you for your ");
        assert!(p.suffix.is_empty());
        assert!(!p.automatic);
        assert!(!at("Codex\n$ explain this ", "explain this ").automatic);
    }
    #[test]
    fn prompt_and_live_footer_are_both_required() {
        // Synthetic text uses the publicly documented composer markers/layout.
        let text = "output\n› Explain this function     \n\n  90% context left";
        let cursor = text[..text.find("function").unwrap()]
            .encode_utf16()
            .count();
        let p = extract(text, cursor).unwrap();
        assert!(p.automatic);
        assert_eq!(p.prefix, "Explain this ");
        assert_eq!(p.suffix, "function");
        let claude = "❯ Explain this\n────────────────\n? for shortcuts";
        assert!(
            extract(claude, "❯ Explain this".encode_utf16().count())
                .unwrap()
                .automatic
        );
        for text in ["› Explain this\n", "$ Explain this\n? for shortcuts"] {
            assert!(
                !extract(
                    text,
                    text.find('\n')
                        .map(|i| text[..i].encode_utf16().count())
                        .unwrap()
                )
                .unwrap()
                .automatic
            );
        }
    }
    fn claude_screen(input: &[&str]) -> String {
        let mut lines = vec!["Earlier output".into(), "─".repeat(70)];
        for (i, text) in input.iter().enumerate() {
            lines.push(format!("{}{text}", if i == 0 { "❯ " } else { "  " }));
        }
        lines.push("─".repeat(70));
        lines.push("  ? for shortcuts".into());
        lines.join("\n")
    }
    #[test]
    fn claude_multiline_projects_cursor_sides_and_preserves_indentation() {
        let text = claude_screen(&[
            "First café 🦀",
            "  second line",
            "",
            "❯ quoted marker",
            "Last line",
        ]);
        let first = after(&text, "First café ");
        assert!(first.automatic);
        assert_eq!(
            first.suffix,
            "🦀\n  second line\n\n❯ quoted marker\nLast line"
        );
        let second = after(&text, "second ");
        assert!(second.automatic);
        assert_eq!(second.prefix, "First café 🦀\n  second ");
        assert_eq!(second.suffix, "line\n\n❯ quoted marker\nLast line");
        assert_eq!(first.scope, second.scope);
        let blank = after(&text, "second line\n  ");
        assert_eq!(blank.prefix, "First café 🦀\n  second line\n");
        assert_eq!(blank.suffix, "\n❯ quoted marker\nLast line");
        assert_eq!(first.scope, blank.scope);
        let last = after(&text, "Last line");
        assert_eq!(last.scope, first.scope);
        assert!(last.suffix.is_empty());
        let separator = claude_screen(&["First", "────────────────", "Last"]);
        assert_eq!(
            after(&separator, "First").suffix,
            "\n────────────────\nLast"
        );
    }
    #[test]
    fn claude_multiline_source_offsets_and_verified_word_acceptance() {
        use crate::acceptance::{AcceptancePlan, Observation};
        let render = |body: &str| claude_screen(&["First café 🦀", body, "Last"]);
        let text = render("Thank you for your ");
        let cursor = text
            [..text.find("Thank you for your ").unwrap() + "Thank you for your ".len()]
            .encode_utf16()
            .count();
        let input_start = text[..text.find("First café").unwrap()]
            .encode_utf16()
            .count();
        let base = 4_000_000;
        let snapshot = Snapshot {
            window: TextRange {
                start: base,
                length: text.encode_utf16().count(),
            },
            text,
        };
        let initial = snapshot
            .prompt(base + cursor, |offset| {
                assert_eq!(offset, base + input_start);
                Some(800)
            })
            .unwrap();
        let mut plan = AcceptancePlan::new(
            &initial.prefix,
            &initial.suffix,
            "thoughtful help",
            false,
            true,
            false,
        )
        .unwrap();
        assert_eq!(plan.next_chunk().unwrap(), "thoughtful");
        let text = render("Thank you for your thoughtful");
        let snapshot = Snapshot {
            window: TextRange {
                start: base,
                length: text.encode_utf16().count(),
            },
            text,
        };
        let next = snapshot.prompt(base + cursor + 10, |_| Some(800)).unwrap();
        assert_eq!(initial.scope, next.scope);
        assert_eq!(
            plan.observe(&next.prefix, &next.suffix),
            Observation::Confirmed
        );
        assert_eq!(plan.remainder, " help");
        let mut rest = AcceptancePlan::new(
            &next.prefix,
            &next.suffix,
            &plan.remainder,
            true,
            true,
            false,
        )
        .unwrap();
        assert_eq!(rest.next_chunk().unwrap(), " help");
        let finished = after(
            &render("Thank you for your thoughtful help"),
            "thoughtful help",
        );
        assert_eq!(
            rest.observe(&finished.prefix, &finished.suffix),
            Observation::Confirmed
        );
    }
    #[test]
    fn claude_clipped_modal_and_collapsed_frames_do_not_gain_automatic_access() {
        let valid = claude_screen(&["First", "Second", "Last"]);
        for invalid in [
            valid.replacen(&format!("{}\n", "─".repeat(70)), "", 1),
            valid.replace(&format!("\n{}\n  ?", "─".repeat(70)), "\n  ?"),
            valid
                .replace("? for shortcuts", "unknown footer")
                .replace("Second", "? for shortcuts"),
            valid.replace("  Second", "Second"),
            valid.replacen(&"─".repeat(70), &"─".repeat(69), 1),
            valid.replace("? for shortcuts", "select an option · ? for shortcuts"),
            valid.replace("? for shortcuts", "esc to cancel · ? for shortcuts"),
            valid.replace("Last", "Last ↓"),
            valid.replace("Second", "[Pasted text #1 +12 lines]"),
            valid.replace("Second", "[Image #1]"),
        ] {
            assert!(!automatic_after(&invalid, "First"), "{invalid}");
        }
        for first in ["/command", "!shell command"] {
            assert!(!automatic_after(
                &claude_screen(&[first, "Second"]),
                "Second"
            ));
        }
        assert!(!automatic_after(
            &claude_screen(&["First", "Current @file"]),
            "@file"
        ));
        let mode = valid.replace("? for shortcuts", "⏵⏵ accept edits on (shift+tab to cycle)");
        assert!(automatic_after(&mode, "Second"));
        let mut lines = vec!["ordinary input"; 16];
        lines[8] = "Active row";
        assert!(automatic_after(&claude_screen(&lines), "Active row"));
        lines.push("extra row");
        assert!(!automatic_after(&claude_screen(&lines), "Active row"));
    }
    #[test]
    fn claude_multiline_panes_require_their_own_footer_and_geometry() {
        let left = claude_screen(&["Left first", "Active left ", "Left last"]);
        let right = claude_screen(&["Neighbor first", "Neighbor draft", "Neighbor last"])
            .replace("? for shortcuts", "unrelated footer");
        let pairs: Vec<_> = left.lines().zip(right.lines()).collect();
        let text = split_rows(&pairs, 80);
        let p = after(&text, "Active left ");
        assert!(p.automatic);
        assert_eq!(p.prefix, "Left first\nActive left ");
        assert_eq!(p.suffix, "\nLeft last");
        assert_eq!(p.edges.len(), 6);
        assert!(!automatic_after(&text, "Neighbor draft"));
        assert_eq!(
            p.scope,
            after(
                &text.replace("Neighbor first", "Changed output"),
                "Active left "
            )
            .scope
        );
    }
    fn pi_screen(input: &str, usage: &str) -> String {
        format!(
            "{0}\n{input}\n{0}\n~/synthetic-project (main)\n{usage}  local-model • medium",
            "─".repeat(64)
        )
    }
    #[test]
    fn pi_frame_recognizes_plain_input_without_eating_shell_like_text() {
        for input in [
            "Explain $ variables ",
            "Review > operators ",
            "› a quoted marker ",
            "Thank you for your ",
            "Ask café 🦀 ",
        ] {
            for usage in ["0.0%/200k (auto)", "↑1.2k ↓500 $0.003 4.3%/1.0M", "?/128k"] {
                let text = pi_screen(input, usage);
                let p = after(&text, input);
                assert!(p.automatic, "{input}: {usage}");
                assert_eq!(p.prefix, input);
                assert!(p.suffix.is_empty());
            }
        }
        let text = pi_screen("Please explain this function", "5.0%/200k");
        let p = after(&text, "Please explain ");
        assert!(p.automatic);
        assert_eq!(p.prefix, "Please explain ");
        assert_eq!(p.suffix, "this function");
        for input in ["/model ", "!echo ", "!!command "] {
            assert!(!after(&pi_screen(input, "0.0%/200k"), input).automatic);
        }
    }
    #[test]
    fn pi_requires_complete_idle_frame_and_well_formed_usage() {
        let valid = pi_screen("Explain this ", "4.0%/200k");
        for invalid in [
            valid.replacen(&"─".repeat(64), "ordinary output", 1),
            valid.replacen("\n~/", "\nautocomplete item\n~/", 1),
            valid.replace("~/synthetic-project (main)", "select an option"),
            valid.replace("4.0%/200k", "-4.0%/200k"),
            valid.replace("4.0%/200k", "4.0%/many"),
            valid.replace("4.0%/200k", "4..0%/200k"),
            valid.replace("  local-model • medium", ""),
        ] {
            let cursor = invalid.find("Explain this ").unwrap() + "Explain this ".len();
            assert!(
                !extract(&invalid, invalid[..cursor].encode_utf16().count())
                    .is_some_and(|p| p.automatic),
                "{invalid}"
            );
        }
    }
    #[test]
    fn pi_frame_isolated_from_neighboring_panes_and_preserves_scope() {
        let rule = "─".repeat(64);
        let render = |input: &str| {
            split_rows(
                &[
                    (&rule, "output"),
                    (input, "unrelated draft "),
                    (&rule, "output"),
                    ("~/synthetic-project", "output"),
                    ("4.0%/200k  local-model", "output"),
                ],
                66,
            )
        };
        let text = render("Explain this ");
        let p = after(&text, "Explain this ");
        assert!(p.automatic);
        assert_eq!(p.prefix, "Explain this ");
        assert!(!after(&text, "unrelated draft ").automatic);
        let updated = render("Explain this function ");
        let q = after(&updated, "Explain this function ");
        assert!(q.automatic);
        assert_eq!(p.scope, q.scope);
        assert_eq!(p.prefix + "function ", q.prefix);
    }
    fn boxed_screen(input: &str, metadata: &str, footer: &str) -> String {
        // Independent text representation of the public OpenCode/Kilo composer:
        // one left border, two columns of padding, metadata, cap and shortcut row.
        let pad = " ".repeat(66usize.saturating_sub(input.width()));
        format!(
            "  ┃\n  ┃  {input}{pad}\n  ┃\n  ┃  {metadata}\n  ╹{}\n{footer}",
            "▀".repeat(70)
        )
    }
    fn multi_screen(boxed: bool, inputs: &[&str]) -> String {
        let padded: Vec<_> = inputs
            .iter()
            .map(|s| format!("{s}{}", " ".repeat(60usize.saturating_sub(s.width()))))
            .collect();
        if boxed {
            format!(
                "  ┃\n  ┃  {}\n  ┃\n  ┃  Build · Local Model\n  ╹{}\ntab agents  ctrl+p commands",
                padded.join("\n  ┃  "),
                "▀".repeat(70)
            )
        } else {
            pi_screen(&padded.join("\n"), "4.0%/200k")
        }
    }
    #[test]
    fn multiline_frames_capture_all_input_rows_and_preserve_cursor_sides() {
        for boxed in [false, true] {
            let text = multi_screen(
                boxed,
                &[
                    "First café 🦀 line",
                    "Please explain this function",
                    "Last line",
                ],
            );
            for (needle, before, after_text) in [
                (
                    "First café 🦀 ",
                    "First café 🦀 ",
                    "line\nPlease explain this function\nLast line",
                ),
                (
                    "Please explain ",
                    "First café 🦀 line\nPlease explain ",
                    "this function\nLast line",
                ),
                (
                    "Last line",
                    "First café 🦀 line\nPlease explain this function\nLast line",
                    "",
                ),
            ] {
                let p = after(&text, needle);
                assert!(p.automatic);
                assert_eq!(p.prefix, before);
                assert_eq!(p.suffix, after_text);
                assert_eq!(
                    p.start_utf16,
                    text[..text.find("First café").unwrap()]
                        .encode_utf16()
                        .count()
                );
                if boxed {
                    assert_eq!(p.edges.len(), 6);
                }
            }
            let blank = multi_screen(boxed, &["First line", "", "Last line"]);
            assert_eq!(after(&blank, "Last line").prefix, "First line\n\nLast line");
        }
    }
    #[test]
    fn multiline_snapshot_uses_source_offsets_and_verifies_partial_and_full_insertion() {
        use crate::acceptance::{AcceptancePlan, Observation};
        for boxed in [false, true] {
            let render =
                |middle: &str| multi_screen(boxed, &["First 🦀 line", middle, "Last line"]);
            let text = render("Thank you for your ");
            let cursor = text
                [..text.find("Thank you for your ").unwrap() + "Thank you for your ".len()]
                .encode_utf16()
                .count();
            let source_start = text[..text.find("First 🦀 line").unwrap()]
                .encode_utf16()
                .count();
            let base = 9_000_000;
            let snapshot = Snapshot {
                window: TextRange {
                    start: base,
                    length: text.encode_utf16().count(),
                },
                text,
            };
            let initial = snapshot
                .prompt(base + cursor, |i| {
                    assert_eq!(i, base + source_start);
                    Some(500)
                })
                .unwrap();
            assert_eq!(initial.start_utf16, base + source_start);
            let mut plan = AcceptancePlan::new(
                &initial.prefix,
                &initial.suffix,
                "help today.",
                false,
                true,
                false,
            )
            .unwrap();
            assert_eq!(plan.next_chunk().unwrap(), "help");
            let text = render("Thank you for your help");
            let next = Snapshot {
                window: TextRange {
                    start: base,
                    length: text.encode_utf16().count(),
                },
                text,
            };
            let observed = next
                .prompt(base + cursor + 4, |i| {
                    assert_eq!(i, base + source_start);
                    Some(500)
                })
                .unwrap();
            assert_eq!(initial.scope, observed.scope);
            assert_eq!(
                plan.observe(&observed.prefix, &observed.suffix),
                Observation::Confirmed
            );
            assert_eq!(plan.remainder, " today.");
            let mut rest = AcceptancePlan::new(
                &observed.prefix,
                &observed.suffix,
                &plan.remainder,
                true,
                true,
                false,
            )
            .unwrap();
            assert_eq!(rest.next_chunk().unwrap(), " today.");
            let final_text = render("Thank you for your help today.");
            let observed = after(&final_text, "Thank you for your help today.");
            assert_eq!(
                rest.observe(&observed.prefix, &observed.suffix),
                Observation::Confirmed
            );
            assert_eq!(observed.suffix, "\nLast line");
        }
    }
    #[test]
    fn multiline_limits_missing_frames_and_commands_do_not_gain_automatic_access() {
        for boxed in [false, true] {
            let mut inputs = vec!["ordinary draft row"; 16];
            inputs[8] = "Active cursor here ";
            let text = multi_screen(boxed, &inputs);
            assert!(after(&text, "Active cursor here ").automatic);
            inputs.push("one row beyond limit");
            assert!(!automatic_after(
                &multi_screen(boxed, &inputs),
                "Active cursor here "
            ));
            let text = multi_screen(boxed, &["First line", "Active cursor here ", "Last line"]);
            for invalid in [
                text.split_once('\n').unwrap().1.to_owned(),
                text.replace("ctrl+p commands", "missing footer")
                    .replace("4.0%/200k", "missing footer"),
            ] {
                let pos =
                    invalid.find("Active cursor here ").unwrap() + "Active cursor here ".len();
                assert!(!extract(&invalid, invalid[..pos].encode_utf16().count())
                    .is_some_and(|p| p.automatic));
            }
            let command = multi_screen(boxed, &["/command", "Active cursor here "]);
            assert!(!after(&command, "Active cursor here ").automatic);
            let command = multi_screen(boxed, &["", "/command", "Active cursor here "]);
            assert!(!after(&command, "Active cursor here ").automatic);
            let reference = multi_screen(boxed, &["First line", "Explain @src/file"]);
            assert!(!after(&reference, "Explain @src/file").automatic);
        }
    }
    #[test]
    fn multiline_panes_exclude_neighbor_text_and_reject_unverified_rows() {
        let left = multi_screen(true, &["First left", "Active left ", "Last left"]);
        let right = multi_screen(
            true,
            &["Private neighbor", "Neighbor draft", "Neighbor end"],
        );
        let rows: Vec<_> = left.lines().zip(right.lines()).collect();
        let text = split_rows(&rows, 80);
        let p = after(&text, "Active left ");
        assert_eq!(p.prefix, "First left\nActive left ");
        assert_eq!(p.suffix, "\nLast left");
        assert!(p.automatic);
        let changed = text.replace("Private neighbor", "Changed neighbor");
        let q = after(&changed, "Active left ");
        assert_eq!(p.scope, q.scope);
        assert_eq!(p.prefix, q.prefix);
        let broken = text.replacen("  ┃  First left", "   ┃ First left", 1);
        let pos = broken.find("Active left ").unwrap() + "Active left ".len();
        assert!(
            !extract(&broken, broken[..pos].encode_utf16().count()).is_some_and(|p| p.automatic)
        );
    }
    fn codex_screen(inputs: &[&str]) -> String {
        let mut rows = vec![" ".repeat(70)];
        for (index, text) in inputs.iter().enumerate() {
            rows.push(format!(
                "{}{text}{}",
                if index == 0 { "› " } else { "  " },
                " ".repeat(68usize.saturating_sub(text.width()))
            ));
        }
        rows.push(" ".repeat(70));
        rows.push(format!("{}100% context left", " ".repeat(52)));
        rows.join("\n")
    }
    #[test]
    fn synthetic_codex_layout_preserves_indentation_and_empty_cursor_rows() {
        let rendered = codex_screen(&["first line", "  second line", " third line", "", "", ""]);
        let text = rendered.as_str();
        let first = after(text, "first ");
        assert!(first.automatic);
        assert_eq!(first.prefix, "first ");
        assert_eq!(first.suffix, "line\n  second line\n third line");
        let second = after(text, "second ");
        assert!(second.automatic);
        assert_eq!(second.prefix, "first line\n  second ");
        assert_eq!(second.suffix, "line\n third line");
        assert_eq!(first.scope, second.scope);
        assert_eq!(first.scope.marker, "› ");
        let empty = text
            .split_inclusive('\n')
            .take(4)
            .map(str::len)
            .sum::<usize>()
            + 2;
        let p = extract(text, text[..empty].encode_utf16().count()).unwrap();
        assert!(p.automatic);
        assert_eq!(p.prefix, "first line\n  second line\n third line\n");
        assert!(p.suffix.is_empty());
        let quoted = codex_screen(&["First line", "", "› quoted marker", "Last line"]);
        let p = after(&quoted, "quoted marker");
        assert_eq!(p.prefix, "First line\n\n› quoted marker");
        assert_eq!(p.suffix, "\nLast line");
    }
    #[test]
    fn codex_multiline_source_offsets_and_word_acceptance_survive_projection() {
        use crate::acceptance::{AcceptancePlan, Observation};
        let render = |body: &str| codex_screen(&["First café 🦀", body, "Last line"]);
        let text = render("  Thank you for your ");
        let cursor = text
            [..text.find("Thank you for your ").unwrap() + "Thank you for your ".len()]
            .encode_utf16()
            .count();
        let start = text[..text.find("First café").unwrap()]
            .encode_utf16()
            .count();
        let base = 5_000_000;
        let snapshot = Snapshot {
            window: TextRange {
                start: base,
                length: text.encode_utf16().count(),
            },
            text,
        };
        let initial = snapshot
            .prompt(base + cursor, |offset| {
                assert_eq!(offset, base + start);
                Some(500)
            })
            .unwrap();
        assert_eq!(initial.prefix, "First café 🦀\n  Thank you for your ");
        let mut plan = AcceptancePlan::new(
            &initial.prefix,
            &initial.suffix,
            "thoughtful help",
            false,
            true,
            false,
        )
        .unwrap();
        assert_eq!(plan.next_chunk().unwrap(), "thoughtful");
        let text = render("  Thank you for your thoughtful");
        let next = Snapshot {
            window: TextRange {
                start: base,
                length: text.encode_utf16().count(),
            },
            text,
        };
        let observed = next.prompt(base + cursor + 10, |_| Some(500)).unwrap();
        assert_eq!(initial.scope, observed.scope);
        assert_eq!(
            plan.observe(&observed.prefix, &observed.suffix),
            Observation::Confirmed
        );
        assert_eq!(plan.remainder, " help");
        let mut rest = AcceptancePlan::new(
            &observed.prefix,
            &observed.suffix,
            &plan.remainder,
            true,
            true,
            false,
        )
        .unwrap();
        assert_eq!(rest.next_chunk().unwrap(), " help");
        let finished = after(
            &render("  Thank you for your thoughtful help"),
            "thoughtful help",
        );
        assert_eq!(
            rest.observe(&finished.prefix, &finished.suffix),
            Observation::Confirmed
        );
    }
    #[test]
    fn codex_clipped_malformed_modal_and_command_frames_do_not_fall_back_to_automatic() {
        let valid = codex_screen(&["First line", "  second line", "Last line"]);
        for invalid in [
            valid.replace("First line", "First line ↑"),
            valid.replace("    second line", "second line"),
            valid.replace("100% context left", "1000% context left"),
            valid.replace(
                "100% context left",
                "reverse-i-search: term 100% context left",
            ),
            valid.replace("100% context left", "esc close 100% context left"),
            valid.split_once('\n').unwrap().1.to_owned(),
            valid.replace(&format!("\n{}\n", " ".repeat(70)), "\n  not a margin\n"),
        ] {
            assert!(!automatic_after(&invalid, "First line"), "{invalid}");
        }
        for inputs in [
            vec!["/command", "Current input"],
            vec!["", "/command", "Current input"],
            vec!["First line", "Current @file"],
        ] {
            let text = codex_screen(&inputs);
            assert!(!automatic_after(&text, inputs.last().unwrap()));
        }
        let mut inputs = vec!["ordinary input"; 16];
        inputs[8] = "Active row";
        assert!(automatic_after(&codex_screen(&inputs), "Active row"));
        inputs.push("too much input");
        assert!(!automatic_after(&codex_screen(&inputs), "Active row"));
    }
    #[test]
    fn codex_multiline_panes_keep_context_footer_and_geometry_proof_separate() {
        let left = codex_screen(&["Left first", "Active left ", "Left last"]);
        let right = codex_screen(&["Neighbor first", "Neighbor draft", "Neighbor last"])
            .replace("100% context left", "unrelated footer");
        let pairs: Vec<_> = left.lines().zip(right.lines()).collect();
        let text = split_rows(&pairs, 80);
        let p = after(&text, "Active left ");
        assert!(p.automatic);
        assert_eq!(p.prefix, "Left first\nActive left ");
        assert_eq!(p.suffix, "\nLeft last");
        assert_eq!(p.edges.len(), 6);
        assert!(!automatic_after(&text, "Neighbor draft"));
        let changed = text.replace("Neighbor first", "Changed output");
        assert_eq!(p.scope, after(&changed, "Active left ").scope);
        let expected_row_start = text[..text.find("Active left ").unwrap()]
            .rfind('\n')
            .unwrap()
            + 1;
        assert!(
            p.edges[0].right_utf16.unwrap() >= text[..expected_row_start].encode_utf16().count()
        );
    }
    #[test]
    fn opencode_and_kilo_extract_prose_and_keep_modes_out_of_automatic_completion() {
        for metadata in [
            "Build · Local Model Ollama",
            "Code  Local Model Ollama · high",
            "Review auto · Local Model",
        ] {
            for footer in [
                "tab agents  ctrl+p commands",
                "ctrl+t variants  alt+a agents  ctrl+x p commands",
                "~/project  20,000 (10%) · $0.05  ctrl+p commands",
            ] {
                let text = boxed_screen("Explain $ variables and > comparisons ", metadata, footer);
                let p = after(&text, "Explain $ variables and > comparisons ");
                assert!(p.automatic, "{metadata} / {footer}");
                assert_eq!(p.prefix, "Explain $ variables and > comparisons ");
                assert!(p.suffix.is_empty());
                assert_eq!(p.edges.len(), 4);
            }
        }
        for input in ["/models ", "!echo ", "Explain @src/ter"] {
            let p = after(
                &boxed_screen(input, "Build · Local Model", "tab agents  ctrl+p commands"),
                input,
            );
            assert!(!p.automatic);
        }
        let text = boxed_screen(
            "Explain @src/terminal.rs ",
            "Build · Local Model",
            "tab agents  ctrl+p commands",
        );
        assert!(after(&text, "Explain @src/terminal.rs ").automatic);
        let transparent = text.replace(&format!("  ╹{}", "▀".repeat(70)), "");
        assert!(after(&transparent, "Explain @src/terminal.rs ").automatic);
        let p = after(
            &boxed_screen("echo $ variable ", "Shell", "esc exit shell mode"),
            "echo $ variable ",
        );
        assert!(!p.automatic);
        assert_eq!(p.prefix, "echo $ variable ");
    }
    #[test]
    fn boxed_composer_requires_its_own_intact_layout_and_footer() {
        let valid = boxed_screen(
            "Explain this ",
            "Build · Local Model",
            "tab agents  ctrl+p commands",
        );
        for invalid in [
            valid.replacen("  ┃\n", "ordinary output\n", 1),
            valid.replace("  ╹", "   ╹"),
            valid.replace('▀', "─"),
            valid.replace("tab agents", "other output"),
            valid.replace("ctrl+p commands", "exit shell mode"),
            valid.replace("ctrl+p commands", "commands"),
            valid.replace("Build · Local Model", "Shell"),
            valid.replace("Build · Local Model", "Build"),
            valid.replace("\ntab agents", "\nselect an option\ntab agents"),
        ] {
            let cursor = invalid.find("Explain this ").unwrap() + "Explain this ".len();
            assert!(
                !extract(&invalid, invalid[..cursor].encode_utf16().count())
                    .is_some_and(|p| p.automatic),
                "{invalid}"
            );
        }
    }
    #[test]
    fn boxed_composer_footer_stays_inside_its_outer_pane() {
        let left = boxed_screen(
            "Left input ",
            "Code Local Model",
            "tab agents  ctrl+p commands",
        );
        let right = boxed_screen("Right input ", "Build · Local Model", "nothing here");
        let rows: Vec<_> = left.lines().zip(right.lines()).collect();
        let text = split_rows(&rows, 80);
        let p = after(&text, "Left input ");
        assert!(p.automatic);
        assert!(!after(&text, "Right input ").automatic);
        assert_eq!(p.prefix, "Left input ");
        let text = text.replace("nothing here", "tab agents  ctrl+p commands");
        assert!(after(&text, "Right input ").automatic);
    }
    #[test]
    fn boxed_composer_unicode_midline_acceptance_keeps_identity_until_agent_changes() {
        use crate::acceptance::{AcceptancePlan, Observation};
        let render =
            |input: &str, model: &str| boxed_screen(input, model, "tab agents  ctrl+p commands");
        let initial = after(
            &render("Ask café 🦀 about Rust", "Build · Local Model"),
            "Ask café 🦀 ",
        );
        assert!(initial.automatic);
        assert_eq!(initial.suffix, "about Rust");
        let mut plan = AcceptancePlan::new(
            &initial.prefix,
            &initial.suffix,
            "politely ",
            false,
            true,
            false,
        )
        .unwrap();
        let updated = after(
            &render("Ask café 🦀 politely about Rust", "Build · Local Model"),
            "Ask café 🦀 politely ",
        );
        assert_eq!(initial.scope, updated.scope);
        assert!(updated.automatic);
        let chunk = plan.next_chunk().unwrap();
        assert_eq!(chunk, "politely ");
        assert_eq!(
            plan.observe(&updated.prefix, &updated.suffix),
            Observation::Confirmed
        );
        let changed = after(
            &render("Ask café 🦀 politely about Rust", "Plan · Other Model"),
            "Ask café 🦀 politely ",
        );
        assert_ne!(initial.scope, changed.scope);
    }
    #[test]
    fn unicode_offsets_padding_and_scope_survive_verified_insertion() {
        let text = "😀 output\n› Ask café 🦀    \n\n80% context left";
        let before = "😀 output\n› Ask café 🦀 ";
        let p = extract(text, before.encode_utf16().count()).unwrap();
        assert_eq!(p.prefix, "Ask café 🦀 ");
        assert!(p.suffix.is_empty());
        let updated = text.replace("🦀    ", "🦀 about Rust ");
        let q = extract(
            &updated,
            "😀 output\n› Ask café 🦀 about Rust "
                .encode_utf16()
                .count(),
        )
        .unwrap();
        assert_eq!(p.scope, q.scope);
        assert!(extract(text, 1).is_none()); // Half a surrogate pair.
        assert_ne!(
            p.scope,
            extract(
                &text.replace("output", "edited"),
                before.encode_utf16().count()
            )
            .unwrap()
            .scope
        );
    }
    #[test]
    fn rejects_unproven_panes_controls_and_unbounded_input() {
        for text in ["❯ left │ right", "│ ❯ input │", "input\ttext", "\u{1b}[0m"] {
            assert!(extract(text, text.encode_utf16().count()).is_none());
        }
        assert!(extract(&"a".repeat(8193), 8193).is_none());
        assert!(extract("abc", 4).is_none());
        assert!(cancels_manual(36, 0));
        assert!(cancels_manual(8, crate::shortcuts::CONTROL));
        assert!(!cancels_manual(48, crate::shortcuts::SHIFT));
    }
    #[test]
    fn verified_chunks_keep_terminal_padding_out_of_the_remainder() {
        use crate::acceptance::{AcceptancePlan, Observation};
        let mut input = "Thank you for your ".to_owned();
        let render = |input: &str| format!("synthetic output\n› {input}      \n\n95% context left");
        let cursor = |input: &str| {
            "synthetic output\n› ".encode_utf16().count() + input.encode_utf16().count()
        };
        let initial = extract(&render(&input), cursor(&input)).unwrap();
        let mut plan = AcceptancePlan::new(
            &initial.prefix,
            &initial.suffix,
            "thoughtful contribution to the discussion.",
            false,
            true,
            false,
        )
        .unwrap();
        while let Some(chunk) = plan.next_chunk() {
            input.push_str(&chunk);
            let current = extract(&render(&input), cursor(&input)).unwrap();
            assert_eq!(current.scope, initial.scope);
            assert_eq!(
                plan.observe(&current.prefix, &current.suffix),
                Observation::Confirmed
            );
        }
        assert_eq!(input, "Thank you for your thoughtful");
        assert_eq!(plan.remainder, " contribution to the discussion.");
        assert_eq!(plan.take_verified_words(), 1);
        let mut plan = AcceptancePlan::new(&input, "", &plan.remainder, true, true, false).unwrap();
        while let Some(chunk) = plan.next_chunk() {
            input.push_str(&chunk);
            let current = extract(&render(&input), cursor(&input)).unwrap();
            assert_eq!(
                plan.observe(&current.prefix, &current.suffix),
                Observation::Confirmed
            );
        }
        assert_eq!(
            input,
            "Thank you for your thoughtful contribution to the discussion."
        );
        assert!(plan.remainder.is_empty());
        // A concurrent terminal redraw cannot be mistaken for our typed chunk.
        let mut plan = AcceptancePlan::new(&input, "", " More", true, true, false).unwrap();
        plan.next_chunk().unwrap();
        let changed = extract(&render("unrelated output"), cursor("unrelated output")).unwrap();
        assert_eq!(
            plan.observe(&changed.prefix, &changed.suffix),
            Observation::Diverged
        );
    }
}
