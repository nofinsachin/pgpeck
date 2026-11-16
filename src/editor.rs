//! The SQL editor buffer: text, cursor, selection, undo and bracket matching.
//! Pure logic with no terminal or database dependency.
//!
//! Every change goes through `Buffer::edit`, which records an undo entry.
//! Commands that change several places (indenting a selection) share one
//! group, so one undo reverts the whole command.

use unicode_width::UnicodeWidthChar;

use crate::sql::{self, TokenKind};

/// A position in the text. `col` counts characters, not bytes or cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    WordLeft,
    WordRight,
    PageUp,
    PageDown,
    BufferStart,
    BufferEnd,
}

/// An editing command. Each has an action name in the keymap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditCmd {
    Move(Motion),
    Select(Motion),
    SelectAll,
    Backspace,
    Delete,
    Newline,
    Indent,
    Outdent,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
}

impl Motion {
    fn description(self) -> &'static str {
        match self {
            Motion::Left => "cursor left",
            Motion::Right => "cursor right",
            Motion::Up => "cursor up",
            Motion::Down => "cursor down",
            Motion::LineStart => "start of line",
            Motion::LineEnd => "end of line",
            Motion::WordLeft => "word left",
            Motion::WordRight => "word right",
            Motion::PageUp => "page up",
            Motion::PageDown => "page down",
            Motion::BufferStart => "start of text",
            Motion::BufferEnd => "end of text",
        }
    }
}

impl EditCmd {
    /// Short text for the hint bar and help overlay.
    pub fn description(self) -> &'static str {
        match self {
            EditCmd::Move(m) => m.description(),
            EditCmd::Select(Motion::Left) => "select left",
            EditCmd::Select(Motion::Right) => "select right",
            EditCmd::Select(Motion::Up) => "select up",
            EditCmd::Select(Motion::Down) => "select down",
            EditCmd::Select(Motion::LineStart) => "select to line start",
            EditCmd::Select(Motion::LineEnd) => "select to line end",
            EditCmd::Select(Motion::WordLeft) => "select word left",
            EditCmd::Select(Motion::WordRight) => "select word right",
            EditCmd::Select(Motion::PageUp) => "select page up",
            EditCmd::Select(Motion::PageDown) => "select page down",
            EditCmd::Select(Motion::BufferStart) => "select to text start",
            EditCmd::Select(Motion::BufferEnd) => "select to text end",
            EditCmd::SelectAll => "select all",
            EditCmd::Backspace => "delete backward",
            EditCmd::Delete => "delete forward",
            EditCmd::Newline => "new line",
            EditCmd::Indent => "indent",
            EditCmd::Outdent => "outdent",
            EditCmd::Undo => "undo",
            EditCmd::Redo => "redo",
            EditCmd::Cut => "cut",
            EditCmd::Copy => "copy",
            EditCmd::Paste => "paste",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub indent_width: usize,
    pub auto_pair: bool,
    pub auto_indent: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            indent_width: 4,
            auto_pair: true,
            auto_indent: true,
        }
    }
}

/// Maximum number of undo steps kept.
const UNDO_LIMIT: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Edit {
    start: Pos,
    removed: String,
    inserted: String,
    before: (Pos, Option<Pos>),
    after: (Pos, Option<Pos>),
    group: u64,
    /// Plain typed characters merge into one undo step.
    typing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffer {
    lines: Vec<String>,
    cursor: Pos,
    anchor: Option<Pos>,
    /// Column to return to when moving vertically through shorter lines.
    want_col: Option<usize>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    group: u64,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

fn byte_index(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
}

fn char_len(line: &str) -> usize {
    line.chars().count()
}

/// Where the cursor ends up after inserting `text` at `start`.
fn advance(start: Pos, text: &str) -> Pos {
    let mut parts = text.split('\n');
    let first = parts.next().unwrap_or("");
    match parts.next_back() {
        None => Pos::new(start.line, start.col + char_len(first)),
        Some(last) => Pos::new(start.line + text.matches('\n').count(), char_len(last)),
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Width of a character in terminal cells.
pub fn cell_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Terminal cells taken by the first `col` characters of `line`.
pub fn display_col(line: &str, col: usize) -> usize {
    line.chars().take(col).map(cell_width).sum()
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: Pos::default(),
            anchor: None,
            want_col: None,
            undo: Vec::new(),
            redo: Vec::new(),
            group: 0,
        }
    }

    #[cfg(test)]
    pub fn with_text(text: &str) -> Self {
        let mut buffer = Self::new();
        buffer.lines = text.split('\n').map(str::to_string).collect();
        buffer
    }

    // -- reading ------------------------------------------------------------

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn cursor(&self) -> Pos {
        self.cursor
    }

    /// The selection as an ordered range, if it is not empty.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let anchor = self.anchor?;
        if anchor == self.cursor {
            return None;
        }
        Some((anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        Some(self.text_between(start, end))
    }

    fn line_len(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, |l| char_len(l))
    }

    fn end_pos(&self) -> Pos {
        let last = self.lines.len() - 1;
        Pos::new(last, self.line_len(last))
    }

    fn char_at(&self, pos: Pos) -> Option<char> {
        self.lines.get(pos.line)?.chars().nth(pos.col)
    }

    fn char_before(&self, pos: Pos) -> Option<char> {
        let col = pos.col.checked_sub(1)?;
        self.lines.get(pos.line)?.chars().nth(col)
    }

    fn clamp(&self, pos: Pos) -> Pos {
        let line = pos.line.min(self.lines.len() - 1);
        Pos::new(line, pos.col.min(self.line_len(line)))
    }

    fn text_between(&self, start: Pos, end: Pos) -> String {
        if start.line == end.line {
            let line = &self.lines[start.line];
            return line[byte_index(line, start.col)..byte_index(line, end.col)].to_string();
        }
        let first = &self.lines[start.line];
        let mut out = first[byte_index(first, start.col)..].to_string();
        for line in &self.lines[start.line + 1..end.line] {
            out.push('\n');
            out.push_str(line);
        }
        let last = &self.lines[end.line];
        out.push('\n');
        out.push_str(&last[..byte_index(last, end.col)]);
        out
    }

    // -- the single mutation primitive --------------------------------------

    /// Replaces `start..end` with `text` without recording anything.
    fn raw_replace(&mut self, start: Pos, end: Pos, text: &str) {
        let head =
            self.lines[start.line][..byte_index(&self.lines[start.line], start.col)].to_string();
        let tail = self.lines[end.line][byte_index(&self.lines[end.line], end.col)..].to_string();
        let combined = format!("{head}{text}{tail}");
        let new_lines: Vec<String> = combined.split('\n').map(str::to_string).collect();
        self.lines.splice(start.line..=end.line, new_lines);
    }

    fn state(&self) -> (Pos, Option<Pos>) {
        (self.cursor, self.anchor)
    }

    fn begin_command(&mut self) {
        self.group += 1;
    }

    /// Replaces `start..end` with `text`, records it for undo, and puts the
    /// cursor at `cursor_after`.
    fn edit(&mut self, start: Pos, end: Pos, text: &str, cursor_after: Pos, typing: bool) {
        let (start, end) = (self.clamp(start), self.clamp(end));
        let before = self.state();
        let removed = self.text_between(start, end);
        self.raw_replace(start, end, text);
        self.cursor = cursor_after;
        self.anchor = None;
        self.want_col = None;
        self.redo.clear();

        // Merge consecutive typed characters into the previous step.
        if typing && removed.is_empty() {
            if let Some(prev) = self.undo.last_mut() {
                if prev.typing
                    && prev.removed.is_empty()
                    && advance(prev.start, &prev.inserted) == start
                {
                    prev.inserted.push_str(text);
                    prev.after = (cursor_after, None);
                    return;
                }
            }
        }
        self.undo.push(Edit {
            start,
            removed,
            inserted: text.to_string(),
            before,
            after: (cursor_after, None),
            group: self.group,
            typing: typing && !text.contains('\n'),
        });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    /// Multi-edit commands call this after moving the cursor and selection
    /// themselves, so redo restores the final state.
    fn finish_command(&mut self) {
        let state = self.state();
        if let Some(last) = self.undo.last_mut() {
            if last.group == self.group {
                last.after = state;
            }
        }
    }

    fn delete_range(&mut self, start: Pos, end: Pos) {
        self.edit(start, end, "", start, false);
    }

    // -- commands -----------------------------------------------------------

    pub fn apply(&mut self, cmd: EditCmd, opts: &Options, page: usize, register: &mut String) {
        match cmd {
            EditCmd::Move(m) => self.move_cursor(m, false, page),
            EditCmd::Select(m) => self.move_cursor(m, true, page),
            EditCmd::SelectAll => self.select_all(),
            EditCmd::Backspace => self.backspace(opts),
            EditCmd::Delete => self.delete(),
            EditCmd::Newline => self.newline(opts),
            EditCmd::Indent => self.indent(opts),
            EditCmd::Outdent => self.outdent(opts),
            EditCmd::Undo => self.undo(),
            EditCmd::Redo => self.redo(),
            EditCmd::Cut => {
                if let Some(text) = self.cut() {
                    *register = text;
                }
            }
            EditCmd::Copy => {
                if let Some(text) = self.selected_text() {
                    *register = text;
                }
            }
            EditCmd::Paste => self.insert_str(&register.clone(), opts),
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(Pos::default());
        self.cursor = self.end_pos();
        self.want_col = None;
    }

    pub fn cut(&mut self) -> Option<String> {
        self.begin_command();
        let (start, end) = self.selection()?;
        let text = self.text_between(start, end);
        self.delete_range(start, end);
        Some(text)
    }

    /// Types one character, with auto-pairing when enabled.
    pub fn insert_char(&mut self, c: char, opts: &Options) {
        self.begin_command();
        if let Some((start, end)) = self.selection() {
            let text = c.to_string();
            self.edit(start, end, &text, advance(start, &text), false);
            return;
        }
        let at = self.cursor;
        let (prev, next) = (self.char_before(at), self.char_at(at));

        if opts.auto_pair {
            // Typing a closing character over the one already there.
            if matches!(c, ')' | ']' | '\'' | '"') && next == Some(c) {
                self.cursor = Pos::new(at.line, at.col + 1);
                self.want_col = None;
                return;
            }
            let closer = match c {
                '(' => Some(')'),
                '[' => Some(']'),
                '\'' | '"' => Some(c),
                _ => None,
            };
            if let Some(closer) = closer {
                let next_is_open =
                    next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | ',' | ';'));
                // No pairing after a word character, so `don't` and `''` stay as typed.
                let quote_ok =
                    !matches!(c, '\'' | '"') || !prev.is_some_and(|p| is_word(p) || p == c);
                if next_is_open && quote_ok {
                    let text = format!("{c}{closer}");
                    self.edit(at, at, &text, Pos::new(at.line, at.col + 1), false);
                    return;
                }
            }
        }
        let text = c.to_string();
        self.edit(
            at,
            at,
            &text,
            Pos::new(at.line, at.col + 1),
            !c.is_whitespace(),
        );
    }

    /// Inserts pasted or inserted text. Line endings are normalized, tabs
    /// become spaces and other control characters are dropped. Nothing is
    /// ever executed (R20.10).
    pub fn insert_str(&mut self, text: &str, opts: &Options) {
        self.begin_command();
        let spaces = " ".repeat(opts.indent_width);
        let clean: String = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', &spaces)
            .chars()
            .filter(|c| *c == '\n' || !c.is_control())
            .collect();
        if clean.is_empty() {
            return;
        }
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        self.edit(start, end, &clean, advance(start, &clean), false);
    }

    pub fn newline(&mut self, opts: &Options) {
        self.begin_command();
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        let indent: String = if opts.auto_indent {
            let line = &self.lines[start.line];
            line.chars()
                .take(start.col)
                .take_while(|c| *c == ' ')
                .collect()
        } else {
            String::new()
        };
        let (prev, next) = (self.char_before(start), self.char_at(end));
        let between_brackets = opts.auto_pair
            && matches!(
                (prev, next),
                (Some('('), Some(')')) | (Some('['), Some(']'))
            );
        if between_brackets {
            let inner = format!("{indent}{}", " ".repeat(opts.indent_width));
            let text = format!("\n{inner}\n{indent}");
            self.edit(
                start,
                end,
                &text,
                Pos::new(start.line + 1, char_len(&inner)),
                false,
            );
        } else {
            let text = format!("\n{indent}");
            self.edit(
                start,
                end,
                &text,
                Pos::new(start.line + 1, char_len(&indent)),
                false,
            );
        }
    }

    pub fn backspace(&mut self, opts: &Options) {
        self.begin_command();
        if let Some((start, end)) = self.selection() {
            return self.delete_range(start, end);
        }
        let at = self.cursor;
        if at.col == 0 {
            if at.line > 0 {
                let join = Pos::new(at.line - 1, self.line_len(at.line - 1));
                self.delete_range(join, at);
            }
            return;
        }
        let from = Pos::new(at.line, at.col - 1);
        let empty_pair = opts.auto_pair
            && matches!(
                (self.char_before(at), self.char_at(at)),
                (Some('('), Some(')'))
                    | (Some('['), Some(']'))
                    | (Some('\''), Some('\''))
                    | (Some('"'), Some('"'))
            );
        let to = if empty_pair {
            Pos::new(at.line, at.col + 1)
        } else {
            at
        };
        self.delete_range(from, to);
    }

    pub fn delete(&mut self) {
        self.begin_command();
        if let Some((start, end)) = self.selection() {
            return self.delete_range(start, end);
        }
        let at = self.cursor;
        let end = if at.col < self.line_len(at.line) {
            Pos::new(at.line, at.col + 1)
        } else if at.line + 1 < self.lines.len() {
            Pos::new(at.line + 1, 0)
        } else {
            return;
        };
        self.delete_range(at, end);
    }

    /// First and last line a block command acts on. A selection that ends at
    /// column 0 does not include that last line.
    fn block_lines(&self) -> (usize, usize) {
        match self.selection() {
            None => (self.cursor.line, self.cursor.line),
            Some((start, end)) => {
                let last = if end.col == 0 && end.line > start.line {
                    end.line - 1
                } else {
                    end.line
                };
                (start.line, last)
            }
        }
    }

    pub fn indent(&mut self, opts: &Options) {
        self.begin_command();
        let width = opts.indent_width.max(1);
        if self.selection().is_none() {
            let at = self.cursor;
            let to_stop = width - (at.col % width);
            let text = " ".repeat(to_stop);
            self.edit(at, at, &text, Pos::new(at.line, at.col + to_stop), false);
            return;
        }
        let (first, last) = self.block_lines();
        let (cursor, anchor) = (self.cursor, self.anchor);
        let pad = " ".repeat(width);
        for line in first..=last {
            let at = Pos::new(line, 0);
            self.edit(at, at, &pad, at, false);
        }
        let shift = |p: Pos| {
            if (first..=last).contains(&p.line) {
                Pos::new(p.line, p.col + width)
            } else {
                p
            }
        };
        (self.cursor, self.anchor) = (shift(cursor), anchor.map(shift));
        self.finish_command();
    }

    pub fn outdent(&mut self, opts: &Options) {
        self.begin_command();
        let width = opts.indent_width.max(1);
        let (first, last) = self.block_lines();
        let (cursor, anchor) = (self.cursor, self.anchor);
        let mut removed_per_line = Vec::new();
        for line in first..=last {
            let n = self.lines[line]
                .chars()
                .take(width)
                .take_while(|c| *c == ' ')
                .count();
            removed_per_line.push(n);
            if n > 0 {
                let at = Pos::new(line, 0);
                self.edit(at, Pos::new(line, n), "", at, false);
            }
        }
        let shift = |p: Pos| match p
            .line
            .checked_sub(first)
            .and_then(|i| removed_per_line.get(i))
        {
            Some(n) => Pos::new(p.line, p.col.saturating_sub(*n)),
            None => p,
        };
        (self.cursor, self.anchor) = (shift(cursor), anchor.map(shift));
        self.finish_command();
    }

    // -- undo ---------------------------------------------------------------

    pub fn undo(&mut self) {
        let Some(group) = self.undo.last().map(|e| e.group) else {
            return;
        };
        while self.undo.last().is_some_and(|e| e.group == group) {
            let Some(edit) = self.undo.pop() else { break };
            let end = advance(edit.start, &edit.inserted);
            self.raw_replace(edit.start, end, &edit.removed);
            (self.cursor, self.anchor) = edit.before;
            self.redo.push(edit);
        }
        self.want_col = None;
    }

    pub fn redo(&mut self) {
        let Some(group) = self.redo.last().map(|e| e.group) else {
            return;
        };
        while self.redo.last().is_some_and(|e| e.group == group) {
            let Some(edit) = self.redo.pop() else { break };
            let end = advance(edit.start, &edit.removed);
            self.raw_replace(edit.start, end, &edit.inserted);
            (self.cursor, self.anchor) = edit.after;
            self.undo.push(edit);
        }
        self.want_col = None;
    }

    // -- movement -----------------------------------------------------------

    pub fn move_cursor(&mut self, motion: Motion, extend: bool, page: usize) {
        // Left and right collapse an existing selection to its edge.
        if !extend {
            if let (Some((start, end)), Motion::Left | Motion::Right) = (self.selection(), motion) {
                self.cursor = if motion == Motion::Left { start } else { end };
                self.anchor = None;
                self.want_col = None;
                return;
            }
        }
        let vertical = matches!(
            motion,
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown
        );
        let want = self.want_col.unwrap_or(self.cursor.col);
        let target = self.target(motion, want, page.max(1));

        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = target;
        self.want_col = vertical.then_some(want);
        if self.anchor == Some(self.cursor) {
            self.anchor = None;
        }
    }

    fn target(&self, motion: Motion, want: usize, page: usize) -> Pos {
        let at = self.cursor;
        let vertical = |line: usize| Pos::new(line, want.min(self.line_len(line)));
        match motion {
            Motion::Left if at.col > 0 => Pos::new(at.line, at.col - 1),
            Motion::Left if at.line > 0 => Pos::new(at.line - 1, self.line_len(at.line - 1)),
            Motion::Right if at.col < self.line_len(at.line) => Pos::new(at.line, at.col + 1),
            Motion::Right if at.line + 1 < self.lines.len() => Pos::new(at.line + 1, 0),
            Motion::Left | Motion::Right => at,
            Motion::Up if at.line > 0 => vertical(at.line - 1),
            Motion::Down if at.line + 1 < self.lines.len() => vertical(at.line + 1),
            Motion::Up | Motion::Down => at,
            Motion::PageUp => vertical(at.line.saturating_sub(page)),
            Motion::PageDown => vertical((at.line + page).min(self.lines.len() - 1)),
            Motion::LineStart => Pos::new(at.line, 0),
            Motion::LineEnd => Pos::new(at.line, self.line_len(at.line)),
            Motion::BufferStart => Pos::default(),
            Motion::BufferEnd => self.end_pos(),
            Motion::WordLeft => self.word_left(at),
            Motion::WordRight => self.word_right(at),
        }
    }

    fn word_left(&self, at: Pos) -> Pos {
        if at.col == 0 {
            return if at.line > 0 {
                Pos::new(at.line - 1, self.line_len(at.line - 1))
            } else {
                at
            };
        }
        let chars: Vec<char> = self.lines[at.line].chars().collect();
        let mut col = at.col;
        while col > 0 && !is_word(chars[col - 1]) {
            col -= 1;
        }
        while col > 0 && is_word(chars[col - 1]) {
            col -= 1;
        }
        Pos::new(at.line, col)
    }

    fn word_right(&self, at: Pos) -> Pos {
        let chars: Vec<char> = self.lines[at.line].chars().collect();
        if at.col >= chars.len() {
            return if at.line + 1 < self.lines.len() {
                Pos::new(at.line + 1, 0)
            } else {
                at
            };
        }
        let mut col = at.col;
        while col < chars.len() && is_word(chars[col]) {
            col += 1;
        }
        while col < chars.len() && !is_word(chars[col]) {
            col += 1;
        }
        Pos::new(at.line, col)
    }

    /// Byte offset of the cursor in `text()`.
    pub fn cursor_offset(&self) -> usize {
        self.offset_of(self.cursor)
    }

    // -- completion ---------------------------------------------------------

    /// The word being typed at the cursor and where it starts. `None` inside
    /// strings, comments and quoted names, in the middle of a word, after a
    /// dot (a qualified name) and when text is selected.
    pub fn completion_prefix(&self) -> Option<(Pos, String)> {
        if self.selection().is_some() {
            return None;
        }
        let text = self.text();
        let at = self.offset_of(self.cursor);
        let token = sql::lex(&text).into_iter().find(|t| {
            t.end == at && matches!(t.kind, TokenKind::Identifier | TokenKind::Keyword)
        })?;
        if text[..token.start].ends_with('.') {
            return None;
        }
        Some((self.pos_of(token.start), text[token.start..at].to_string()))
    }

    /// Replaces the text from `start` to the cursor with `text`.
    pub fn complete(&mut self, start: Pos, text: &str) {
        self.begin_command();
        let end = self.cursor;
        self.edit(start, end, text, advance(start, text), false);
    }

    // -- brackets -----------------------------------------------------------

    fn offset_of(&self, pos: Pos) -> usize {
        let before: usize = self.lines[..pos.line].iter().map(|l| l.len() + 1).sum();
        before + byte_index(&self.lines[pos.line], pos.col)
    }

    fn pos_of(&self, offset: usize) -> Pos {
        let mut rest = offset;
        for (line, text) in self.lines.iter().enumerate() {
            if rest <= text.len() {
                return Pos::new(line, text[..rest].chars().count());
            }
            rest -= text.len() + 1;
        }
        self.end_pos()
    }

    /// The bracket next to the cursor and its partner, when both exist.
    /// Brackets inside strings and comments do not count.
    pub fn matching_bracket(&self) -> Option<(Pos, Pos)> {
        let text = self.text();
        let tokens = sql::lex(&text);
        let at = self.offset_of(self.cursor);
        let is_bracket = |offset: usize| {
            tokens
                .iter()
                .find(|t| t.start == offset && t.kind == TokenKind::Punct && t.end - t.start == 1)
        };
        let index = [Some(at), at.checked_sub(1)]
            .into_iter()
            .flatten()
            .find_map(|offset| {
                let token = is_bracket(offset)?;
                matches!(text.as_bytes()[token.start], b'(' | b')' | b'[' | b']')
                    .then_some(token.start)
            })?;
        let (open, close, forward) = match text.as_bytes()[index] {
            b'(' => (b'(', b')', true),
            b'[' => (b'[', b']', true),
            b')' => (b'(', b')', false),
            _ => (b'[', b']', false),
        };

        let brackets: Vec<usize> = tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Punct && t.end - t.start == 1)
            .map(|t| t.start)
            .filter(|i| matches!(text.as_bytes()[*i], b if b == open || b == close))
            .collect();
        let at_index = brackets.iter().position(|i| *i == index)?;
        let mut depth = 0i32;
        let ordered: Box<dyn Iterator<Item = &usize>> = if forward {
            Box::new(brackets[at_index..].iter())
        } else {
            Box::new(brackets[..=at_index].iter().rev())
        };
        for &i in ordered {
            let b = text.as_bytes()[i];
            depth += if (b == open) == forward { 1 } else { -1 };
            if depth == 0 {
                return Some((self.pos_of(index), self.pos_of(i)));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> Options {
        Options::default()
    }

    fn typed(text: &str) -> Buffer {
        let mut b = Buffer::new();
        for c in text.chars() {
            if c == '\n' {
                b.newline(&opts());
            } else {
                b.insert_char(c, &opts());
            }
        }
        b
    }

    fn at(b: &Buffer) -> (usize, usize) {
        (b.cursor().line, b.cursor().col)
    }

    // typing and basic editing

    #[test]
    fn typing_inserts_characters_and_moves_cursor() {
        let b = typed("select 1");
        assert_eq!(b.text(), "select 1");
        assert_eq!(at(&b), (0, 8));
    }

    #[test]
    fn newline_splits_the_line_and_keeps_indent() {
        let mut b = Buffer::with_text("  select a, b");
        b.move_cursor(Motion::LineEnd, false, 1);
        b.newline(&opts());
        assert_eq!(b.text(), "  select a, b\n  ");
        assert_eq!(at(&b), (1, 2));
    }

    #[test]
    fn auto_indent_can_be_disabled() {
        let mut b = Buffer::with_text("  a");
        b.move_cursor(Motion::LineEnd, false, 1);
        b.newline(&Options {
            auto_indent: false,
            ..opts()
        });
        assert_eq!(b.text(), "  a\n");
    }

    #[test]
    fn backspace_and_delete_join_lines() {
        let mut b = Buffer::with_text("ab\ncd");
        b.move_cursor(Motion::Down, false, 1);
        b.move_cursor(Motion::LineStart, false, 1);
        b.backspace(&opts());
        assert_eq!(b.text(), "abcd");
        assert_eq!(at(&b), (0, 2));

        let mut b = Buffer::with_text("ab\ncd");
        b.move_cursor(Motion::LineEnd, false, 1);
        b.delete();
        assert_eq!(b.text(), "abcd");
    }

    #[test]
    fn backspace_at_start_and_delete_at_end_do_nothing() {
        let mut b = Buffer::with_text("ab");
        b.backspace(&opts());
        b.move_cursor(Motion::LineEnd, false, 1);
        b.delete();
        assert_eq!(b.text(), "ab");
    }

    #[test]
    fn multibyte_text_is_edited_by_character() {
        let mut b = typed("naïve 日本");
        b.backspace(&opts());
        assert_eq!(b.text(), "naïve 日");
        b.move_cursor(Motion::WordLeft, false, 1);
        b.delete();
        assert_eq!(b.text(), "naïve ");
    }

    // auto-pairing

    #[test]
    fn brackets_and_quotes_pair() {
        let b = typed("f(");
        assert_eq!(b.text(), "f()");
        assert_eq!(at(&b), (0, 2));
        let b = typed("x = '");
        assert_eq!(b.text(), "x = ''");
    }

    #[test]
    fn typing_the_closer_overtypes() {
        let b = typed("f(1)");
        assert_eq!(b.text(), "f(1)");
        assert_eq!(at(&b), (0, 4));
        let b = typed("'a'");
        assert_eq!(b.text(), "'a'");
    }

    #[test]
    fn apostrophe_in_a_word_does_not_pair() {
        let b = typed("don't");
        assert_eq!(b.text(), "don't");
    }

    #[test]
    fn no_pairing_before_a_word_character() {
        let mut b = Buffer::with_text("abc");
        b.insert_char('(', &opts());
        assert_eq!(b.text(), "(abc");
    }

    #[test]
    fn auto_pair_can_be_disabled() {
        let mut b = Buffer::new();
        b.insert_char(
            '(',
            &Options {
                auto_pair: false,
                ..opts()
            },
        );
        assert_eq!(b.text(), "(");
    }

    #[test]
    fn backspace_inside_an_empty_pair_removes_both() {
        let mut b = typed("f(");
        b.backspace(&opts());
        assert_eq!(b.text(), "f");
        let mut b = typed("f(1");
        b.backspace(&opts());
        assert_eq!(b.text(), "f()", "a non-empty pair is not removed");
    }

    #[test]
    fn enter_between_brackets_opens_an_indented_block() {
        let mut b = typed("f(");
        b.newline(&opts());
        assert_eq!(b.text(), "f(\n    \n)");
        assert_eq!(at(&b), (1, 4));
    }

    // selection and clipboard

    #[test]
    fn shift_motion_selects_and_typing_replaces() {
        let mut b = Buffer::with_text("hello world");
        b.move_cursor(Motion::WordRight, true, 1);
        assert_eq!(b.selected_text().as_deref(), Some("hello "));
        b.insert_char('X', &opts());
        assert_eq!(b.text(), "Xworld");
        assert_eq!(b.selection(), None);
    }

    #[test]
    fn selection_across_lines() {
        let mut b = Buffer::with_text("ab\ncd\nef");
        b.move_cursor(Motion::Right, false, 1);
        b.move_cursor(Motion::Down, true, 1);
        b.move_cursor(Motion::Down, true, 1);
        assert_eq!(b.selected_text().as_deref(), Some("b\ncd\ne"));
        b.backspace(&opts());
        assert_eq!(b.text(), "af");
    }

    #[test]
    fn plain_left_and_right_collapse_the_selection() {
        let mut b = Buffer::with_text("abcd");
        b.move_cursor(Motion::Right, false, 1);
        b.move_cursor(Motion::Right, true, 1);
        b.move_cursor(Motion::Right, true, 1);
        b.move_cursor(Motion::Left, false, 1);
        assert_eq!((at(&b), b.selection()), ((0, 1), None));
    }

    #[test]
    fn select_all_cut_copy_paste() {
        let mut b = Buffer::with_text("one\ntwo");
        let mut register = String::new();
        b.select_all();
        b.apply(EditCmd::Copy, &opts(), 1, &mut register);
        assert_eq!(register, "one\ntwo");
        assert_eq!(b.text(), "one\ntwo");
        b.apply(EditCmd::Cut, &opts(), 1, &mut register);
        assert_eq!(b.text(), "");
        b.apply(EditCmd::Paste, &opts(), 1, &mut register);
        assert_eq!(b.text(), "one\ntwo");
        assert_eq!(at(&b), (1, 3));
    }

    #[test]
    fn copy_without_selection_keeps_the_register() {
        let mut b = Buffer::with_text("x");
        let mut register = "keep".to_string();
        b.apply(EditCmd::Copy, &opts(), 1, &mut register);
        b.apply(EditCmd::Cut, &opts(), 1, &mut register);
        assert_eq!(register, "keep");
    }

    #[test]
    fn pasted_text_is_normalized() {
        let mut b = Buffer::new();
        b.insert_str("a\r\nb\rc\td\u{7}e", &opts());
        assert_eq!(b.text(), "a\nb\nc    de");
    }

    #[test]
    fn pasted_multiline_text_moves_cursor_to_its_end() {
        let mut b = Buffer::with_text("ab");
        b.move_cursor(Motion::Right, false, 1);
        b.insert_str("1\n22\n333", &opts());
        assert_eq!(b.text(), "a1\n22\n333b");
        assert_eq!(at(&b), (2, 3));
    }

    // movement

    #[test]
    fn vertical_movement_remembers_the_column() {
        let mut b = Buffer::with_text("abcdef\nab\nabcdef");
        b.move_cursor(Motion::LineEnd, false, 1);
        b.move_cursor(Motion::Down, false, 1);
        assert_eq!(at(&b), (1, 2));
        b.move_cursor(Motion::Down, false, 1);
        assert_eq!(at(&b), (2, 6));
    }

    #[test]
    fn left_and_right_wrap_across_lines() {
        let mut b = Buffer::with_text("a\nb");
        b.move_cursor(Motion::Right, false, 1);
        b.move_cursor(Motion::Right, false, 1);
        assert_eq!(at(&b), (1, 0));
        b.move_cursor(Motion::Left, false, 1);
        assert_eq!(at(&b), (0, 1));
    }

    #[test]
    fn word_motions() {
        let mut b = Buffer::with_text("foo_bar  baz.qux");
        b.move_cursor(Motion::WordRight, false, 1);
        assert_eq!(at(&b), (0, 9));
        b.move_cursor(Motion::WordRight, false, 1);
        assert_eq!(at(&b), (0, 13));
        b.move_cursor(Motion::WordLeft, false, 1);
        assert_eq!(at(&b), (0, 9));
        b.move_cursor(Motion::WordLeft, false, 1);
        assert_eq!(at(&b), (0, 0));
    }

    #[test]
    fn page_and_buffer_motions() {
        let text = (0..50)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut b = Buffer::with_text(&text);
        b.move_cursor(Motion::PageDown, false, 20);
        assert_eq!(b.cursor().line, 20);
        b.move_cursor(Motion::BufferEnd, false, 20);
        assert_eq!(at(&b), (49, 2));
        b.move_cursor(Motion::PageUp, false, 100);
        assert_eq!(b.cursor().line, 0);
        b.move_cursor(Motion::BufferEnd, false, 1);
        b.move_cursor(Motion::BufferStart, false, 1);
        assert_eq!(at(&b), (0, 0));
    }

    #[test]
    fn movement_at_the_edges_stays_put() {
        let mut b = Buffer::with_text("a");
        b.move_cursor(Motion::Left, false, 1);
        b.move_cursor(Motion::Up, false, 1);
        assert_eq!(at(&b), (0, 0));
        b.move_cursor(Motion::BufferEnd, false, 1);
        b.move_cursor(Motion::Right, false, 1);
        b.move_cursor(Motion::Down, false, 1);
        assert_eq!(at(&b), (0, 1));
    }

    // indent

    #[test]
    fn tab_inserts_spaces_to_the_next_stop() {
        let mut b = Buffer::with_text("ab");
        b.move_cursor(Motion::Right, false, 1);
        b.move_cursor(Motion::Right, false, 1);
        b.indent(&opts());
        assert_eq!(b.text(), "ab  ");
        b.indent(&opts());
        assert_eq!(b.text(), "ab      ");
    }

    #[test]
    fn indent_and_outdent_a_selection_in_one_undo_step() {
        let mut b = Buffer::with_text("a\nb\nc");
        b.move_cursor(Motion::Down, true, 1);
        b.move_cursor(Motion::Down, true, 1);
        b.indent(&opts());
        assert_eq!(b.text(), "    a\n    b\nc");
        assert_eq!(b.selected_text().as_deref(), Some("a\n    b\n"));
        b.outdent(&opts());
        assert_eq!(b.text(), "a\nb\nc");
        b.undo();
        assert_eq!(b.text(), "    a\n    b\nc");
        b.undo();
        assert_eq!(b.text(), "a\nb\nc");
        b.redo();
        assert_eq!(b.text(), "    a\n    b\nc");
    }

    #[test]
    fn outdent_removes_only_what_is_there() {
        let mut b = Buffer::with_text("  x");
        b.outdent(&opts());
        assert_eq!(b.text(), "x");
        b.outdent(&opts());
        assert_eq!(b.text(), "x");
    }

    // undo and redo

    #[test]
    fn typing_undoes_in_word_sized_steps() {
        let mut b = typed("select 1");
        b.undo();
        assert_eq!(b.text(), "select ");
        b.undo();
        assert_eq!(b.text(), "select");
        b.undo();
        assert_eq!(b.text(), "");
        b.undo();
        assert_eq!(b.text(), "");
    }

    #[test]
    fn undo_restores_cursor_and_selection() {
        let mut b = Buffer::with_text("hello world");
        b.move_cursor(Motion::WordRight, true, 1);
        let before = (b.cursor(), b.selection());
        b.backspace(&opts());
        assert_eq!(b.text(), "world");
        b.undo();
        assert_eq!(b.text(), "hello world");
        assert_eq!((b.cursor(), b.selection()), before);
        b.redo();
        assert_eq!(b.text(), "world");
        assert_eq!(at(&b), (0, 0));
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut b = typed("ab");
        b.undo();
        b.insert_char('z', &opts());
        b.redo();
        assert_eq!(b.text(), "z");
    }

    #[test]
    fn undo_of_a_multiline_paste() {
        let mut b = Buffer::with_text("ab");
        b.move_cursor(Motion::Right, false, 1);
        b.insert_str("1\n2", &opts());
        b.undo();
        assert_eq!(b.text(), "ab");
        assert_eq!(at(&b), (0, 1));
        b.redo();
        assert_eq!(b.text(), "a1\n2b");
    }

    #[test]
    fn undo_depth_is_bounded() {
        let mut b = Buffer::new();
        for _ in 0..UNDO_LIMIT + 50 {
            b.insert_char(' ', &opts());
        }
        assert_eq!(b.undo.len(), UNDO_LIMIT);
    }

    // brackets

    #[test]
    fn matching_bracket_forward_and_backward() {
        let mut b = Buffer::with_text("f(a, (b))");
        b.move_cursor(Motion::Right, false, 1);
        b.move_cursor(Motion::Right, false, 1);
        // Cursor is just after `(`.
        assert_eq!(b.matching_bracket(), Some((Pos::new(0, 1), Pos::new(0, 8))));
        b.move_cursor(Motion::LineEnd, false, 1);
        assert_eq!(b.matching_bracket(), Some((Pos::new(0, 8), Pos::new(0, 1))));
    }

    #[test]
    fn matching_bracket_spans_lines() {
        let mut b = Buffer::with_text("f(\n  1\n)");
        b.move_cursor(Motion::Right, false, 1);
        assert_eq!(b.matching_bracket(), Some((Pos::new(0, 1), Pos::new(2, 0))));
    }

    #[test]
    fn brackets_in_strings_and_comments_are_ignored() {
        let b = Buffer::with_text("(')' -- )\n)");
        let mut b2 = b.clone();
        b2.move_cursor(Motion::Right, false, 1);
        assert_eq!(
            b2.matching_bracket(),
            Some((Pos::new(0, 0), Pos::new(1, 0)))
        );
        let mut inside = b;
        for _ in 0..2 {
            inside.move_cursor(Motion::Right, false, 1);
        }
        assert_eq!(inside.matching_bracket(), None, "the `'` is not a bracket");
    }

    #[test]
    fn unmatched_bracket_has_no_partner() {
        let mut b = Buffer::with_text("f(a");
        b.move_cursor(Motion::Right, false, 1);
        b.move_cursor(Motion::Right, false, 1);
        assert_eq!(b.matching_bracket(), None);
    }

    // completion

    fn prefix_at_end(text: &str) -> Option<(Pos, String)> {
        let mut b = Buffer::with_text(text);
        b.move_cursor(Motion::BufferEnd, false, 1);
        b.completion_prefix()
    }

    #[test]
    fn prefix_is_the_word_before_the_cursor() {
        assert_eq!(
            prefix_at_end("select * fr"),
            Some((Pos::new(0, 9), "fr".to_string()))
        );
        assert_eq!(
            prefix_at_end("a\n  whe"),
            Some((Pos::new(1, 2), "whe".to_string()))
        );
        assert_eq!(
            prefix_at_end("x_1"),
            Some((Pos::new(0, 0), "x_1".to_string()))
        );
    }

    #[test]
    fn no_prefix_in_strings_comments_quotes_numbers_or_after_a_dot() {
        for text in [
            "'sel",
            "-- sel",
            "/* sel",
            "\"sel",
            "$$ sel",
            "t.col",
            "select 12",
            "select ",
            "f(",
            "x ",
        ] {
            assert_eq!(prefix_at_end(text), None, "{text:?}");
        }
    }

    #[test]
    fn no_prefix_in_the_middle_of_a_word_or_with_a_selection() {
        let mut b = Buffer::with_text("select");
        b.move_cursor(Motion::Right, false, 1);
        b.move_cursor(Motion::Right, false, 1);
        assert_eq!(b.completion_prefix(), None);
        b.move_cursor(Motion::LineEnd, false, 1);
        assert!(b.completion_prefix().is_some());
        b.move_cursor(Motion::WordLeft, true, 1);
        assert_eq!(b.completion_prefix(), None);
    }

    #[test]
    fn complete_replaces_the_prefix_and_undoes_to_it() {
        let mut b = typed("select a fr");
        let (start, prefix) = b.completion_prefix().expect("prefix");
        assert_eq!(prefix, "fr");
        b.complete(start, "from");
        assert_eq!(b.text(), "select a from");
        assert_eq!(at(&b), (0, 13));
        b.undo();
        assert_eq!(b.text(), "select a fr");
    }

    #[test]
    fn display_columns_account_for_wide_characters() {
        assert_eq!(display_col("a日b", 2), 3);
        assert_eq!(display_col("abc", 10), 3);
    }
}
