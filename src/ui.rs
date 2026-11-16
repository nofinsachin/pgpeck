//! Rendering only. Reads state and draws it. No I/O, no state changes.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, Pane};
use crate::keymap::{Action, Scope};
use crate::theme::Theme;

/// Provisional minimum size. The real value is an open question in the
/// requirements. Below it, one pane is shown at a time (R20.5).
pub const MIN_SIZE: (u16, u16) = (60, 15);

/// Width below which the context strip collapses to a single line (R10.7).
const NARROW_WIDTH: u16 = 80;

const HINT_ACTIONS: &[Action] = &[
    Action::HelpToggle,
    Action::FocusNext,
    Action::FocusEditor,
    Action::ToggleContext,
    Action::AppQuit,
];

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let hint_height = u16::from(app.config.ui.show_hint_bar);
    let [body, hint, status] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(hint_height),
        Constraint::Length(1),
    ])
    .areas(area);

    if area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1 {
        draw_too_small(frame, app, theme, body);
    } else {
        draw_panes(frame, app, theme, body);
    }
    if app.config.ui.show_hint_bar {
        draw_hint_bar(frame, app, theme, hint);
    }
    draw_status_bar(frame, app, theme, status);
    if app.state.help_open {
        draw_help(frame, app, theme, area);
    }
}

fn draw_panes(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let st = &app.state;
    let context_height = match (st.show_context, area.width < NARROW_WIDTH) {
        (false, _) => 0,
        (true, true) => 3,
        (true, false) => 7,
    };
    let results_constraint = if st.show_results {
        Constraint::Percentage(40)
    } else {
        Constraint::Length(0)
    };
    let [editor, context, results] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(context_height),
        results_constraint,
    ])
    .areas(area);

    draw_pane(frame, app, theme, Pane::Editor, editor);
    if st.show_context {
        draw_pane(frame, app, theme, Pane::Context, context);
    }
    if st.show_results {
        draw_pane(frame, app, theme, Pane::Results, results);
    }
}

fn draw_too_small(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let [message, pane] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let text = format!("Terminal too small. Need {}x{}", MIN_SIZE.0, MIN_SIZE.1);
    frame.render_widget(
        Paragraph::new(text).style(Style::new().fg(theme.error).add_modifier(Modifier::BOLD)),
        message,
    );
    draw_pane(frame, app, theme, app.state.focus, pane);
}

fn placeholder(pane: Pane) -> &'static str {
    match pane {
        Pane::Editor => "SQL editor (phase 2)",
        Pane::Context => "Objects used by the current statement (phase 6)",
        Pane::Results => "Query results (phase 4)",
    }
}

fn draw_pane(frame: &mut Frame, app: &App, theme: &Theme, pane: Pane, area: Rect) {
    let focused = app.state.focus == pane;
    // Focus shows as a marker and bold title as well as color (R20.9).
    let (title, style) = if focused {
        (
            format!(" ▶ {} ", pane.label()),
            Style::new().fg(theme.focus).add_modifier(Modifier::BOLD),
        )
    } else {
        (format!(" {} ", pane.label()), Style::new().fg(theme.dimmed))
    };
    let block = Block::bordered()
        .title(title)
        .border_style(style)
        .title_style(style);
    let body = Paragraph::new(placeholder(pane))
        .style(Style::new().fg(theme.dimmed))
        .block(block);
    frame.render_widget(body, area);
}

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
    let mut text = format!(" no connection │ focus: {}", st.focus.label());
    if let Some(notice) = &st.notice {
        text.push_str(" │ ");
        text.push_str(notice);
    }
    frame.render_widget(
        Paragraph::new(text).style(Style::new().fg(theme.status_fg).bg(theme.status_bg)),
        area,
    );
}

/// Keyboard reference: bindings of the focused scope and the global scope (R5.7).
fn draw_help(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let focus_scope = app.state.focus.scope();
    let mut lines = Vec::new();
    for scope in [focus_scope, Scope::Global] {
        let bindings = app.keymap.bindings(scope);
        if bindings.is_empty() {
            continue;
        }
        lines.push(Line::styled(
            format!("{} keys", scope.name()),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        for (key, action) in bindings {
            lines.push(Line::from(vec![
                Span::raw(format!("  {:<12}", key.to_string())),
                Span::raw(format!("{:<22}", action.description())),
                Span::styled(action.name(), Style::new().fg(theme.dimmed)),
            ]));
        }
        lines.push(Line::raw(""));
    }

    let width = 64.min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = centered(area, width, height);
    let block = Block::bordered()
        .title(" Help ")
        .title_alignment(Alignment::Center)
        .border_style(Style::new().fg(theme.focus));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::events::Event;
    use crate::keymap::Keymap;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
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

    #[test]
    fn default_screen_80x24() {
        assert_snapshot("default_80x24", &render(&app(80, 24), 80, 24));
    }

    #[test]
    fn results_focused_with_help_open() {
        let mut a = app(100, 30);
        key(&mut a, KeyCode::Char('3'), KeyModifiers::ALT);
        key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
        assert_snapshot("results_help_100x30", &render(&a, 100, 30));
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
}
