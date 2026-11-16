//! Rendering only. Reads state and draws it. No I/O, no state changes.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, HelpLine, Pane};
use crate::editor::{cell_width, display_col, Pos};
use crate::keymap::{Action, Scope};
use crate::layout;
use crate::sql::{self, Token, TokenKind};
use crate::theme::Theme;

const HINT_ACTIONS: &[Action] = &[
    Action::HelpToggle,
    Action::FocusNext,
    Action::FocusEditor,
    Action::ToggleContext,
    Action::AppQuit,
];

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let areas = layout::compute(area, &app.state, &app.config);

    if let Some(message) = areas.message {
        let text = format!(
            "Terminal too small. Need {}x{}",
            layout::MIN_SIZE.0,
            layout::MIN_SIZE.1
        );
        frame.render_widget(
            Paragraph::new(text).style(Style::new().fg(theme.error).add_modifier(Modifier::BOLD)),
            message,
        );
    }
    if let Some(rect) = areas.editor {
        draw_editor(frame, app, theme, rect);
    }
    if let Some(rect) = areas.context {
        draw_placeholder(frame, app, theme, Pane::Context, rect);
    }
    if let Some(rect) = areas.results {
        draw_placeholder(frame, app, theme, Pane::Results, rect);
    }
    if app.config.ui.show_hint_bar {
        draw_hint_bar(frame, app, theme, areas.hint);
    }
    draw_status_bar(frame, app, theme, areas.status);
    if app.state.help_open {
        draw_help(frame, app, theme, area);
    }
}

/// Focus shows as a marker and bold title as well as color (R20.9).
fn pane_block<'a>(app: &App, theme: &Theme, pane: Pane) -> Block<'a> {
    let (title, style) = if app.state.focus == pane {
        (
            format!(" ▶ {} ", pane.label()),
            Style::new().fg(theme.focus).add_modifier(Modifier::BOLD),
        )
    } else {
        (format!(" {} ", pane.label()), Style::new().fg(theme.dimmed))
    };
    Block::bordered()
        .title(title)
        .border_style(style)
        .title_style(style)
}

fn draw_placeholder(frame: &mut Frame, app: &App, theme: &Theme, pane: Pane, area: Rect) {
    let text = match pane {
        Pane::Editor => "",
        Pane::Context => "Objects used by the current statement (phase 6)",
        Pane::Results => "Query results (phase 4)",
    };
    let body = Paragraph::new(text)
        .style(Style::new().fg(theme.dimmed))
        .block(pane_block(app, theme, pane));
    frame.render_widget(body, area);
}

// ---------------------------------------------------------------------------
// Editor
// ---------------------------------------------------------------------------

fn token_style(kind: TokenKind, theme: &Theme) -> Style {
    match kind {
        TokenKind::Keyword => Style::new().fg(theme.keyword).add_modifier(Modifier::BOLD),
        TokenKind::String | TokenKind::DollarString => Style::new().fg(theme.string),
        TokenKind::Comment => Style::new().fg(theme.dimmed).add_modifier(Modifier::ITALIC),
        TokenKind::Number | TokenKind::Parameter => Style::new().fg(theme.number),
        _ => Style::new(),
    }
}

/// Collects consecutive characters with the same style into one span.
#[derive(Default)]
struct SpanBuilder<'a> {
    spans: Vec<Span<'a>>,
    text: String,
    style: Style,
}

impl<'a> SpanBuilder<'a> {
    fn push(&mut self, c: char, style: Style) {
        if style != self.style {
            self.flush();
            self.style = style;
        }
        self.text.push(c);
    }

    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.spans
                .push(Span::styled(std::mem::take(&mut self.text), self.style));
        }
    }

    fn finish(mut self) -> Vec<Span<'a>> {
        self.flush();
        self.spans
    }
}

fn draw_editor(frame: &mut Frame, app: &App, theme: &Theme, pane: Rect) {
    let st = &app.state;
    let buffer = &st.editor;
    frame.render_widget(pane_block(app, theme, Pane::Editor), pane);

    let area = layout::editor_area(pane, buffer.line_count());
    if area.text.width == 0 || area.text.height == 0 {
        return;
    }
    let text = buffer.text();
    let tokens = sql::lex(&text);
    let selection = buffer.selection();
    let brackets = buffer.matching_bracket();
    let cursor = buffer.cursor();
    let scroll_left = st.scroll_left;
    let text_width = usize::from(area.text.width);

    // Byte offset where each line starts.
    let mut line_starts = Vec::with_capacity(buffer.line_count());
    let mut offset = 0;
    for line in buffer.lines() {
        line_starts.push(offset);
        offset += line.len() + 1;
    }

    let mut rows = Vec::new();
    for row in 0..usize::from(area.text.height) {
        let index = st.scroll_top + row;
        let Some(line) = buffer.lines().get(index) else {
            break;
        };

        let number = format!(
            "{:>width$} ",
            index + 1,
            width = usize::from(area.gutter_width) - 1
        );
        let number_style = if index == cursor.line {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.dimmed)
        };

        let mut builder = SpanBuilder::default();
        let mut spans = vec![Span::styled(number, number_style)];

        let mut token_index = tokens.partition_point(|t: &Token| t.end <= line_starts[index]);
        let mut cell = 0usize;
        for (col, (byte, ch)) in line.char_indices().enumerate() {
            let width = cell_width(ch);
            let absolute = line_starts[index] + byte;
            while tokens.get(token_index).is_some_and(|t| t.end <= absolute) {
                token_index += 1;
            }
            let mut style = tokens
                .get(token_index)
                .filter(|t| t.start <= absolute)
                .map_or(Style::new(), |t| token_style(t.kind, theme));
            if is_selected(selection, index, col, line.chars().count()) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            if brackets.is_some_and(|(a, b)| a == Pos::new(index, col) || b == Pos::new(index, col))
            {
                style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
            }

            if cell + width <= scroll_left {
                cell += width;
                continue;
            }
            if cell < scroll_left {
                // A wide character cut by the left edge shows as blanks.
                for _ in 0..(cell + width - scroll_left) {
                    builder.push(' ', style);
                }
                cell += width;
                continue;
            }
            if cell + width - scroll_left > text_width {
                break;
            }
            builder.push(ch, style);
            cell += width;
        }
        spans.extend(builder.finish());
        rows.push(Line::from(spans));
    }

    let body = Rect {
        x: area.text.x - area.gutter_width,
        width: area.text.width + area.gutter_width,
        ..area.text
    };
    frame.render_widget(Paragraph::new(rows), body);

    if st.focus == Pane::Editor && !st.help_open {
        let line = buffer.lines().get(cursor.line).map_or("", String::as_str);
        let x = display_col(line, cursor.col);
        if cursor.line >= st.scroll_top && x >= scroll_left {
            let (dx, dy) = (x - scroll_left, cursor.line - st.scroll_top);
            if dx < usize::from(area.text.width) && dy < usize::from(area.text.height) {
                frame.set_cursor_position((area.text.x + dx as u16, area.text.y + dy as u16));
            }
        }
    }
}

/// Is column `col` of `line` inside the selection?
fn is_selected(selection: Option<(Pos, Pos)>, line: usize, col: usize, line_len: usize) -> bool {
    let Some((start, end)) = selection else {
        return false;
    };
    if line < start.line || line > end.line {
        return false;
    }
    let from = if line == start.line { start.col } else { 0 };
    let to = if line == end.line { end.col } else { line_len };
    col >= from && col < to
}

// ---------------------------------------------------------------------------
// Hint bar, status bar, help
// ---------------------------------------------------------------------------

fn draw_hint_bar(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for action in HINT_ACTIONS {
        let Some(key) = app.hint_key(*action) else {
            continue;
        };
        let key = key.to_string();
        let label = format!(" {}", action.description());
        let separator = if spans.is_empty() { 0 } else { 2 };
        let needed = separator + key.chars().count() + label.chars().count();
        // Drop hints that do not fit, instead of cutting one in half.
        if used + needed > usize::from(area.width) {
            break;
        }
        used += needed;
        if separator > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key, Style::new().add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(label, Style::new().fg(theme.dimmed)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_status_bar(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let st = &app.state;
    let cursor = st.editor.cursor();
    let mut text = format!(
        " no connection │ focus: {} │ Ln {}, Col {}",
        st.focus.label(),
        cursor.line + 1,
        cursor.col + 1
    );
    if let Some(notice) = &st.notice {
        text.push_str(" │ ");
        text.push_str(notice);
    }
    frame.render_widget(
        Paragraph::new(text).style(Style::new().fg(theme.status_fg).bg(theme.status_bg)),
        area,
    );
}

fn draw_help(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let all = app.help_lines();
    let popup = layout::help_popup(area, all.len());
    let visible = usize::from(popup.height.saturating_sub(2));
    let max_scroll = all.len().saturating_sub(visible);
    let scroll = app.state.help_scroll.min(max_scroll);

    let lines: Vec<Line> = all
        .iter()
        .skip(scroll)
        .take(visible)
        .map(|line| match line {
            HelpLine::Header(text) => {
                Line::styled(text.clone(), Style::new().add_modifier(Modifier::BOLD))
            }
            HelpLine::Binding {
                key,
                description,
                name,
            } => Line::from(vec![
                Span::raw(format!("  {key:<18}")),
                Span::raw(format!("{description:<22}")),
                Span::styled(*name, Style::new().fg(theme.dimmed)),
            ]),
            HelpLine::Blank => Line::raw(""),
        })
        .collect();

    let key_of = |action| {
        app.keymap
            .key_for(action, &[Scope::Dialog], |_| true)
            .map_or(String::new(), |k| k.to_string())
    };
    let mut block = Block::bordered()
        .title(" Help ")
        .title_alignment(Alignment::Center)
        .border_style(Style::new().fg(theme.focus));
    if max_scroll > 0 {
        let footer = format!(
            " {}/{} scroll  {} close  ({}/{}) ",
            key_of(Action::HelpScrollUp),
            key_of(Action::HelpScrollDown),
            key_of(Action::HelpToggle),
            scroll + 1,
            max_scroll + 1
        );
        block = block
            .title_bottom(footer)
            .title_alignment(Alignment::Center);
    }
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::events::Event;
    use crate::keymap::Keymap;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn terminal(width: u16, height: u16) -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(width, height)).expect("test terminal")
    }

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal = terminal(width, height);
        terminal
            .draw(|f| draw(f, app, &Theme::default()))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..height {
            let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }

    /// Compares with `tests/snapshots/<name>.txt`. Run with `UPDATE_SNAPSHOTS=1`
    /// to write the file.
    fn assert_snapshot(name: &str, actual: &str) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/snapshots")
            .join(format!("{name}.txt"));
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::write(&path, actual).expect("write snapshot");
            return;
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing snapshot {path:?}. Run with UPDATE_SNAPSHOTS=1"));
        assert_eq!(actual, expected, "snapshot {name} differs");
    }

    fn app(width: u16, height: u16) -> App {
        let mut app = App::new(Config::default(), Keymap::default_keymap());
        app.update(Event::Resize(width, height));
        app
    }

    fn key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        app.update(Event::Key(KeyEvent::new(code, mods)));
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            let code = if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            };
            key(app, code, KeyModifiers::NONE);
        }
    }

    const SAMPLE: &str =
        "-- users per day\nselect id, name\n  from users\n where name = 'a;b' and n > 10;";

    #[test]
    fn default_screen_80x24() {
        assert_snapshot("default_80x24", &render(&app(80, 24), 80, 24));
    }

    #[test]
    fn editor_with_sql_text() {
        let mut a = app(80, 24);
        type_text(&mut a, SAMPLE);
        assert_snapshot("editor_sql_80x24", &render(&a, 80, 24));
    }

    #[test]
    fn results_focused_with_help_open() {
        let mut a = app(100, 30);
        key(&mut a, KeyCode::Char('3'), KeyModifiers::ALT);
        key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
        assert_snapshot("results_help_100x30", &render(&a, 100, 30));
    }

    #[test]
    fn editor_help_is_scrollable_on_a_small_screen() {
        let mut a = app(80, 24);
        key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
        let top = render(&a, 80, 24);
        assert!(top.contains("editor keys") && top.contains("scroll"));
        assert!(
            !top.contains("global keys"),
            "global keys are below the fold"
        );
        for _ in 0..10 {
            key(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
        }
        assert!(render(&a, 80, 24).contains("global keys"));
        assert_snapshot("editor_help_top_80x24", &top);
    }

    #[test]
    fn narrow_terminal_collapses_context_strip() {
        assert_snapshot("narrow_70x20", &render(&app(70, 20), 70, 20));
    }

    #[test]
    fn too_small_terminal_shows_one_pane_and_a_message() {
        let mut a = app(40, 10);
        key(&mut a, KeyCode::Char('3'), KeyModifiers::ALT);
        let out = render(&a, 40, 10);
        assert!(out.contains("Terminal too small"));
        assert!(out.contains("Results"));
        assert!(!out.contains("Editor"));
        assert_snapshot("too_small_40x10", &out);
    }

    #[test]
    fn too_small_terminal_still_shows_the_editor_when_focused() {
        let mut a = app(40, 10);
        type_text(&mut a, "select 1");
        let out = render(&a, 40, 10);
        assert!(out.contains("select 1") && out.contains("Terminal too small"));
    }

    #[test]
    fn hidden_panes_are_not_drawn() {
        let mut a = app(80, 24);
        key(&mut a, KeyCode::F(3), KeyModifiers::NONE);
        key(&mut a, KeyCode::F(4), KeyModifiers::NONE);
        let out = render(&a, 80, 24);
        assert!(!out.contains("Context") && !out.contains("Results"));
    }

    #[test]
    fn hint_bar_does_not_advertise_tab_while_editor_keeps_it() {
        let out = render(&app(80, 24), 80, 24);
        let hint = out.lines().rev().nth(1).expect("hint line");
        assert!(!hint.contains("Tab"), "{hint}");
        assert!(hint.contains("F6 next pane"), "{hint}");
    }

    #[test]
    fn status_bar_shows_cursor_position() {
        let mut a = app(80, 24);
        type_text(&mut a, "ab\ncde");
        let out = render(&a, 80, 24);
        assert!(out
            .lines()
            .last()
            .is_some_and(|l| l.contains("Ln 2, Col 4")));
    }

    /// The style of the cell at (x, y) after drawing.
    fn cell_style(app: &App, width: u16, height: u16, x: u16, y: u16) -> Style {
        let mut t = terminal(width, height);
        t.draw(|f| draw(f, app, &Theme::default())).expect("draw");
        t.backend().buffer()[(x, y)].style()
    }

    // The editor text starts at x=3 (border + 2 column gutter), y=1.
    #[test]
    fn syntax_is_highlighted() {
        let theme = Theme::default();
        let mut a = app(80, 24);
        type_text(&mut a, SAMPLE);
        // Line 1 is a comment.
        let comment = cell_style(&a, 80, 24, 3, 1);
        assert_eq!(comment.fg, Some(theme.dimmed));
        assert!(comment.add_modifier.contains(Modifier::ITALIC));
        // Line 2: `select` is a keyword, `id` is not.
        let keyword = cell_style(&a, 80, 24, 3, 2);
        assert_eq!(keyword.fg, Some(theme.keyword));
        assert!(keyword.add_modifier.contains(Modifier::BOLD));
        assert_ne!(cell_style(&a, 80, 24, 10, 2).fg, Some(theme.keyword));
        // Line 4: the string, including its semicolon, is one string token.
        let string_col = 3 + " where name = ".len() as u16 + 4;
        assert_eq!(cell_style(&a, 80, 24, string_col, 4).fg, Some(theme.string));
    }

    #[test]
    fn selection_is_reversed_and_not_color_only() {
        let mut a = app(80, 24);
        type_text(&mut a, "abc");
        key(&mut a, KeyCode::Left, KeyModifiers::SHIFT);
        assert!(cell_style(&a, 80, 24, 5, 1)
            .add_modifier
            .contains(Modifier::REVERSED));
        assert!(!cell_style(&a, 80, 24, 4, 1)
            .add_modifier
            .contains(Modifier::REVERSED));
    }

    #[test]
    fn matching_brackets_are_marked() {
        let mut a = app(80, 24);
        type_text(&mut a, "f(1)");
        key(&mut a, KeyCode::Left, KeyModifiers::NONE); // cursor between `1` and `)`
        let open = cell_style(&a, 80, 24, 4, 1);
        let close = cell_style(&a, 80, 24, 6, 1);
        for style in [open, close] {
            assert!(style.add_modifier.contains(Modifier::UNDERLINED));
        }
        assert!(!cell_style(&a, 80, 24, 5, 1)
            .add_modifier
            .contains(Modifier::UNDERLINED));
    }

    #[test]
    fn terminal_cursor_follows_the_editor_cursor() {
        let mut a = app(80, 24);
        type_text(&mut a, "ab\nxyz");
        let mut t = terminal(80, 24);
        t.draw(|f| draw(f, &a, &Theme::default())).expect("draw");
        let pos = t.get_cursor_position().expect("cursor");
        assert_eq!((pos.x, pos.y), (3 + 3, 2));
    }

    #[test]
    fn cursor_is_hidden_when_the_editor_is_not_focused() {
        let mut a = app(80, 24);
        key(&mut a, KeyCode::Char('3'), KeyModifiers::ALT);
        let mut t = terminal(80, 24);
        t.draw(|f| draw(f, &a, &Theme::default())).expect("draw");
        // ratatui hides the cursor when no position was set during the frame.
        assert!(t.backend().buffer()[(0, 0)].symbol() == "┌");
        assert_eq!(t.get_cursor_position().expect("cursor").x, 0);
    }

    #[test]
    fn long_lines_scroll_horizontally_without_breaking_the_gutter() {
        let mut a = app(80, 24);
        type_text(&mut a, &format!("{}END", "x".repeat(150)));
        let out = render(&a, 80, 24);
        let row = out.lines().nth(1).expect("first row");
        assert!(row.contains("END"), "{row}");
        assert!(row.starts_with("│1 "), "{row}");
        assert!(!row.contains(&"x".repeat(100)));
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let mut a = app(80, 24);
        type_text(&mut a, "日本x");
        let mut t = terminal(80, 24);
        t.draw(|f| draw(f, &a, &Theme::default())).expect("draw");
        let pos = t.get_cursor_position().expect("cursor");
        assert_eq!(pos.x, 3 + 5);
    }

    #[test]
    fn long_buffer_scrolls_and_shows_line_numbers() {
        let mut a = app(80, 24);
        for _ in 0..40 {
            key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        }
        let out = render(&a, 80, 24);
        assert!(out.contains("41"), "last line number is visible");
        assert!(!out.lines().nth(1).is_some_and(|l| l.starts_with("│ 1 ")));
    }

    // Keep the `Color` import used even if themes change.
    #[test]
    fn default_theme_uses_real_colors() {
        assert_ne!(Theme::default().keyword, Color::Reset);
    }
}
