//! Rendering only. Reads state and draws it. No I/O, no state changes.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, ConnStatus, Connection, Dialog, HelpLine, Pane};
use crate::catalog;
use crate::db::{DbError, ResultSet, SessionStats, TxState};
use crate::editor::{cell_width, display_col, Pos};
use crate::keymap::{Action, KeyChord, Scope};
use crate::layout;
use crate::picker::{self, Item};
use crate::results::Tab;
use crate::sql::{self, Token, TokenKind};
use crate::theme::Theme;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    // Paint the whole screen first. Text without its own color uses `value`,
    // so it stays readable whatever the terminal's own colors are.
    frame.render_widget(
        Block::new().style(Style::new().fg(theme.value).bg(theme.background)),
        area,
    );
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
        draw_context(frame, app, theme, rect);
    }
    if let Some(rect) = areas.trends {
        draw_trends(frame, app, theme, rect);
    }
    if let Some(rect) = areas.session {
        draw_session(frame, app, theme, rect);
    }
    if let Some(rect) = areas.results {
        draw_results(frame, app, theme, rect);
    }
    if let (Some(rect), Some(_)) = (areas.editor, &app.state.completion) {
        draw_completion(frame, app, theme, rect);
    }
    if app.config.ui.show_hint_bar {
        draw_hint_bar(frame, app, theme, areas.hint);
    }
    draw_status_bar(frame, app, theme, areas.status);
    if app.state.help_open {
        draw_help(frame, app, theme, area);
    }
    match &app.state.dialog {
        Some(Dialog::Picker(picker)) => draw_picker(frame, app, theme, picker, area),
        Some(Dialog::Find(find)) => draw_find(frame, app, theme, find, area),
        Some(Dialog::Form(form)) => draw_form(frame, app, theme, form, area),
        Some(Dialog::Session(form)) => draw_session_form(frame, app, theme, form, area),
        Some(Dialog::Password(_)) => draw_password(frame, app, theme, area),
        Some(Dialog::Confirm(confirm)) => draw_confirm(frame, app, theme, confirm, area),
        None => {}
    }
}

/// Focus shows as a marker and bold title as well as color (R20.9).
fn pane_block<'a>(app: &App, theme: &Theme, pane: Pane) -> Block<'a> {
    let st = &app.state;
    let mut label = pane.label().to_string();
    if pane == Pane::Editor && st.docs.len() > 1 {
        label = format!(
            "{label}: {} ({}/{})",
            st.doc().name,
            st.active + 1,
            st.docs.len()
        );
    }
    let (title, style) = if st.focus == pane {
        (
            format!(" ▶ {label} "),
            Style::new().fg(theme.focus).add_modifier(Modifier::BOLD),
        )
    } else {
        (format!(" {label} "), Style::new().fg(theme.dimmed))
    };
    Block::bordered()
        .title(title)
        .border_style(style)
        .title_style(style)
}

/// How a key is shown. A plain letter is shown as a capital, so `C connect`
/// reads as a key and not as a word.
fn key_label(key: KeyChord) -> String {
    let text = key.to_string();
    if text.len() == 1 && text.chars().all(|c| c.is_ascii_lowercase()) {
        text.to_uppercase()
    } else {
        text
    }
}

/// Keys shown in messages come from the keymap, never from the code.
fn key_text(app: &App, action: Action) -> String {
    app.hint_key(action)
        .map_or_else(|| "(unbound)".to_string(), key_label)
}

/// A key for `action` that works whichever pane has focus.
fn global_key(app: &App, action: Action) -> String {
    app.keymap
        .key_for(action, &[Scope::Global], |_| true)
        .map_or_else(|| "(unbound)".to_string(), key_label)
}

/// The key for `action` in the commands pane, wherever focus is now.
fn command_key(app: &App, action: Action) -> String {
    app.keymap
        .key_for(action, &[Scope::Context, Scope::Global], |_| true)
        .map_or_else(|| "(unbound)".to_string(), key_label)
}

// ---------------------------------------------------------------------------
// Context pane (connection details until the explorer arrives)
// ---------------------------------------------------------------------------

fn draw_context(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let dim = Style::new().fg(theme.dimmed);
    let bold = Style::new().fg(theme.value).add_modifier(Modifier::BOLD);
    let label = Style::new().fg(theme.label);
    let value = Style::new().fg(theme.value);
    let st = &app.state;
    let lines: Vec<Line> = match st.active_connection() {
        None => {
            let mut lines = vec![
                Line::styled("Welcome to oxpecker.", bold),
                Line::styled(
                    format!(
                        "Press {} to connect to a database.",
                        command_key(app, Action::ConnectionOpen)
                    ),
                    value,
                ),
            ];
            if st.focus != Pane::Context {
                lines.push(Line::styled(
                    format!(
                        "Press {} for commands.",
                        key_text(app, Action::FocusCommands)
                    ),
                    dim,
                ));
            }
            lines
        }
        Some(_) => {
            let buffer = if st.docs.len() > 1 {
                format!("{} ({}/{})", st.doc().name, st.active + 1, st.docs.len())
            } else {
                st.doc().name.clone()
            };
            let editor = st.editor();
            let all = editor.text();
            let statement = sql::statement_at(&all, editor.cursor_offset())
                .map(|s| {
                    s.text(&all)
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|s| !s.is_empty());
            vec![
                Line::from(vec![
                    Span::styled("Buffer     ", label),
                    Span::styled(buffer, bold),
                ]),
                match statement {
                    Some(text) => Line::from(vec![
                        Span::styled("Statement  ", label),
                        Span::styled(text, value),
                    ]),
                    None => Line::from(vec![
                        Span::styled("Statement  ", label),
                        Span::styled("none under the cursor", dim),
                    ]),
                },
            ]
        }
    };
    let body = Paragraph::new(lines).block(pane_block(app, theme, Pane::Context));
    frame.render_widget(body, area);
}

/// The session pane: who you are connected as, and how busy this database is.
/// The numbers are for the connected database only. CPU and memory are shared
/// by every database on the server and PostgreSQL cannot report them, so the
/// shared resource shown is the server's connection limit.
fn draw_session(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let dim = Style::new().fg(theme.dimmed);
    let block = pane_block(app, theme, Pane::Session);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(conn) = app.state.active_connection() else {
        frame.render_widget(Paragraph::new(Line::styled("Not connected", dim)), inner);
        return;
    };
    let rollback_key = app
        .keymap
        .key_for(Action::TransactionRollback, &[Scope::Global], |_| true)
        .map_or_else(|| "(unbound)".to_string(), key_label);
    let view = SessionView::new(theme, conn, &rollback_key);
    if inner.width >= TWO_COLUMNS {
        let [left, right] = ratatui::layout::Layout::horizontal([
            ratatui::layout::Constraint::Length(IDENTITY_WIDTH),
            ratatui::layout::Constraint::Min(1),
        ])
        .areas(inner);
        frame.render_widget(Paragraph::new(view.identity()), left);
        frame.render_widget(Paragraph::new(view.server()), right);
    } else {
        frame.render_widget(Paragraph::new(view.compact()), inner);
    }
}

/// The trends pane: the largest tables of the connected database, as bars.
/// It is read-only and never takes focus.
fn draw_trends(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let dim = Style::new().fg(theme.dimmed);
    let value = Style::new().fg(theme.value);
    let block = Block::bordered()
        .title(" Trends: largest tables ")
        .border_style(dim)
        .title_style(dim);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(conn) = app.state.active_connection() else {
        frame.render_widget(Paragraph::new(Line::styled("Not connected", dim)), inner);
        return;
    };
    let Some(stats) = &conn.stats else {
        let text = if conn.status == ConnStatus::Connected {
            "Reading…"
        } else {
            "Not connected"
        };
        frame.render_widget(Paragraph::new(Line::styled(text, dim)), inner);
        return;
    };
    // Five rows fit exactly, so the title does the work of a heading.
    let mut lines = Vec::new();
    if stats.top_tables.is_empty() {
        lines.push(Line::styled("No tables to show", dim));
    }
    let largest = stats.top_tables.iter().map(|t| t.bytes).max().unwrap_or(0);
    let width = usize::from(inner.width);
    let size_width = 9;
    let bar_width = (width / 4).clamp(4, 12);
    let name_width = width.saturating_sub(bar_width + size_width + 2).max(6);
    for table in &stats.top_tables {
        let name = if table.schema == "public" {
            table.name.clone()
        } else {
            format!("{}.{}", table.schema, table.name)
        };
        // At least one block, so a small table is still visible.
        let filled = if largest == 0 {
            0
        } else {
            ((table.bytes as f64 / largest as f64) * bar_width as f64).ceil() as usize
        };
        let filled = filled.clamp(1, bar_width);
        lines.push(Line::from(vec![
            Span::styled(fit(&clean_cell(&name), name_width), value),
            Span::raw(" "),
            Span::styled("█".repeat(filled), Style::new().fg(theme.focus)),
            Span::styled(
                "░".repeat(bar_width - filled),
                Style::new().fg(theme.dimmed),
            ),
            Span::styled(format!("{:>size_width$}", format_bytes(table.bytes)), dim),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Inside the session pane, wide enough for identity and server side by side.
const TWO_COLUMNS: u16 = 70;
const IDENTITY_WIDTH: u16 = 22;

struct SessionView<'a> {
    theme: &'a Theme,
    conn: &'a Connection,
    rollback_key: &'a str,
    host: String,
    port: u16,
    database: String,
    user: String,
}

impl<'a> SessionView<'a> {
    fn new(theme: &'a Theme, conn: &'a Connection, rollback_key: &'a str) -> Self {
        // Before the server answers, show what the profile asks for.
        let (host, port, database, user) = match &conn.info {
            Some(i) => (i.host.clone(), i.port, i.database.clone(), i.user.clone()),
            None => (
                conn.profile.host.clone(),
                conn.profile.port,
                conn.profile.database.clone().unwrap_or_default(),
                conn.profile.user.clone().unwrap_or_default(),
            ),
        };
        Self {
            theme,
            conn,
            rollback_key,
            host,
            port,
            database,
            user,
        }
    }

    fn dim(&self) -> Style {
        Style::new().fg(self.theme.dimmed)
    }

    fn label(&self) -> Style {
        Style::new().fg(self.theme.label)
    }

    fn value(&self) -> Style {
        Style::new().fg(self.theme.value)
    }

    fn warn(&self) -> Style {
        Style::new()
            .fg(self.theme.warn)
            .add_modifier(Modifier::BOLD)
    }

    fn status_style(&self) -> Style {
        match self.conn.status {
            ConnStatus::Connected => Style::new().fg(self.theme.ok),
            ConnStatus::Connecting | ConnStatus::AwaitingPassword => {
                Style::new().fg(self.theme.warn)
            }
            ConnStatus::Failed(_) | ConnStatus::Closed(_) => Style::new().fg(self.theme.error),
        }
    }

    fn alert(&self) -> Style {
        Style::new()
            .fg(self.theme.error)
            .add_modifier(Modifier::BOLD)
    }

    fn head(&self) -> Line<'static> {
        let mut spans = vec![
            Span::styled(
                self.conn.profile.shown_name().to_string(),
                self.value().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", status_word(&self.conn.status)),
                self.status_style(),
            ),
        ];
        if self.conn.profile.production {
            spans.push(Span::styled("  PROD", self.alert()));
        }
        Line::from(spans)
    }

    fn failure(&self) -> Option<Line<'static>> {
        match &self.conn.status {
            ConnStatus::Failed(m) | ConnStatus::Closed(m) => {
                Some(Line::styled(m.clone(), Style::new().fg(self.theme.error)))
            }
            _ => None,
        }
    }

    fn labelled(&self, label: &str, value: String) -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("{label:<12}"), self.label()),
            Span::styled(value, self.value_style(label)),
        ])
    }

    fn value_style(&self, _label: &str) -> Style {
        self.value()
    }

    /// A transaction left open is flagged until it ends (R16.2).
    fn transaction(&self, with_key: bool) -> Option<Line<'static>> {
        let text = match (self.conn.tx, with_key) {
            (TxState::Idle, _) => return None,
            (TxState::Open, _) => "open".to_string(),
            (TxState::Failed, true) => format!("FAILED. {} rolls back", self.rollback_key),
            (TxState::Failed, false) => "FAILED".to_string(),
        };
        Some(Line::from(vec![
            Span::styled(format!("{:<12}", "Transaction"), self.label()),
            Span::styled(text, self.alert()),
        ]))
    }

    fn host_line(&self) -> Line<'static> {
        let mut spans = vec![Span::styled(
            match (&self.conn.status, &self.conn.info) {
                (ConnStatus::Connected, Some(i)) => {
                    format!(
                        "{}:{}  PostgreSQL {}",
                        self.host, self.port, i.server_version
                    )
                }
                _ => format!("{}:{}", self.host, self.port),
            },
            self.dim(),
        )];
        if self.conn.info.as_ref().is_some_and(|i| !i.tls)
            && self.conn.status == ConnStatus::Connected
        {
            spans.push(Span::styled("  TLS off", self.warn()));
        }
        Line::from(spans)
    }

    /// Why there are no numbers, or `None` when there are.
    fn no_stats(&self) -> Option<Line<'static>> {
        if self.conn.status != ConnStatus::Connected {
            return None;
        }
        match (&self.conn.stats, &self.conn.stats_error) {
            (Some(_), _) => None,
            (None, Some(e)) => Some(Line::styled(
                format!("Statistics unavailable: {e}"),
                Style::new().fg(self.theme.error),
            )),
            (None, None) => Some(Line::styled("Reading statistics…", self.dim())),
        }
    }

    fn connections_text(s: &SessionStats) -> String {
        format!(
            "{} here · {} of {} on server",
            s.connections, s.server_connections, s.max_connections
        )
    }

    fn connections_short(s: &SessionStats) -> String {
        format!(
            "{} here · {}/{} on server",
            s.connections, s.server_connections, s.max_connections
        )
    }

    /// The server is near its connection limit, which every database shares.
    fn crowded(s: &SessionStats) -> bool {
        s.max_connections > 0 && s.server_connections * 100 >= s.max_connections * 80
    }

    fn activity_text(s: &SessionStats) -> String {
        match (s.active, s.idle_in_transaction) {
            (Some(a), Some(i)) if s.sees_all_sessions => {
                format!("{a} active · {i} idle in transaction")
            }
            _ => "needs the pg_read_all_stats role".to_string(),
        }
    }

    fn size_text(s: &SessionStats) -> String {
        let cache = s
            .cache_hit_percent
            .map_or_else(|| "n/a".to_string(), |p| format!("{p:.1}%"));
        format!("{} · cache hit {cache}", format_bytes(s.size_bytes))
    }

    fn age_text(s: &SessionStats) -> String {
        let oldest = match (s.oldest_transaction_secs, s.sees_all_sessions) {
            (Some(secs), _) => format_age(secs),
            (None, true) => "none".to_string(),
            (None, false) => "n/a".to_string(),
        };
        format!(
            "oldest tx {oldest} · up {} · deadlocks {}",
            format_age(s.uptime_secs),
            s.deadlocks
        )
    }

    /// Left column: who you are.
    fn identity(&self) -> Vec<Line<'static>> {
        let mut lines = vec![self.head()];
        lines.extend(self.failure());
        lines.push(self.labelled("Database", self.database.clone()));
        lines.push(self.labelled("User", self.user.clone()));
        lines.extend(self.transaction(false));
        lines
    }

    /// Right column: the server and how busy this database is.
    fn server(&self) -> Vec<Line<'static>> {
        let mut lines = vec![self.host_line()];
        if let Some(s) = &self.conn.stats {
            let style = if Self::crowded(s) {
                self.warn()
            } else {
                Style::new()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{:<12}", "Connections"), self.label()),
                Span::styled(Self::connections_text(s), style),
            ]));
            lines.push(self.labelled("Activity", Self::activity_text(s)));
            lines.push(self.labelled("Size", Self::size_text(s)));
            lines.push(self.labelled("Health", Self::age_text(s)));
        }
        lines.extend(self.no_stats());
        lines
    }

    /// One column, for a narrow pane. Most useful first, so a short pane
    /// cuts the least important.
    fn compact(&self) -> Vec<Line<'static>> {
        let mut lines = vec![self.head()];
        lines.extend(self.failure());
        lines.push(Line::from(vec![
            Span::styled(self.database.clone(), self.value()),
            Span::styled(" · ", self.dim()),
            Span::styled(self.user.clone(), self.value()),
        ]));
        lines.extend(self.transaction(true));
        if let Some(s) = &self.conn.stats {
            let style = if Self::crowded(s) {
                self.warn()
            } else {
                Style::new()
            };
            lines.push(Line::styled(
                format!("Conns {}", Self::connections_short(s)),
                style,
            ));
            lines.push(Line::styled(Self::size_text(s), Style::new()));
        }
        lines.extend(self.no_stats());
        lines.push(self.host_line());
        lines
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A span of time in its two largest units: `12s`, `5m`, `3h 20m`, `2d 4h`.
fn format_age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
        _ => format!("{}d {}h", secs / 86400, secs % 86400 / 3600),
    }
}

fn status_word(status: &ConnStatus) -> &'static str {
    match status {
        ConnStatus::Connecting => "connecting",
        ConnStatus::AwaitingPassword => "waiting for a password",
        ConnStatus::Connected => "connected",
        ConnStatus::Failed(_) => "failed",
        ConnStatus::Closed(_) => "disconnected",
    }
}

// ---------------------------------------------------------------------------
// Results pane
// ---------------------------------------------------------------------------

fn format_elapsed(d: std::time::Duration) -> String {
    if d.as_millis() < 1000 {
        format!("{} ms", d.as_millis())
    } else {
        format!("{:.2} s", d.as_secs_f64())
    }
}

/// One line of cell text: control characters are made visible or removed so
/// a value cannot break the grid.
fn clean_cell(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\n' | '\r' => Some('↵'),
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// `text` cut or padded to exactly `width` terminal cells.
fn fit(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    let total: usize = text.chars().map(cell_width).sum();
    let limit = if total > width {
        width.saturating_sub(1)
    } else {
        width
    };
    for c in text.chars() {
        let w = cell_width(c);
        if used + w > limit {
            break;
        }
        out.push(c);
        used += w;
    }
    if total > width && width > 0 {
        out.push('…');
        used += 1;
    }
    out.extend(std::iter::repeat_n(' ', width.saturating_sub(used)));
    out
}

const MAX_COLUMN_WIDTH: usize = 40;
const WIDTH_SAMPLE_ROWS: usize = 200;

fn column_widths(set: &ResultSet) -> Vec<usize> {
    let mut widths: Vec<usize> = set
        .columns
        .iter()
        .map(|c| clean_cell(c).chars().map(cell_width).sum())
        .collect();
    for row in set.rows.iter().take(WIDTH_SAMPLE_ROWS) {
        for (width, cell) in widths.iter_mut().zip(row) {
            let text = cell
                .as_deref()
                .map_or(4, |t| clean_cell(t).chars().map(cell_width).sum());
            *width = (*width).max(text);
        }
    }
    widths
        .iter()
        .map(|w| (*w).clamp(1, MAX_COLUMN_WIDTH))
        .collect()
}

fn draw_results(frame: &mut Frame, app: &App, theme: &Theme, pane: Rect) {
    frame.render_widget(pane_block(app, theme, Pane::Results), pane);
    let st = &app.state;
    let dim = Style::new().fg(theme.dimmed);
    let results = st.doc().results.as_ref();
    let tab_count = results.map_or(0, |r| r.tab_count());
    let body = layout::results_body(pane, tab_count > 1);

    let running_here = st.running.as_ref().filter(|r| r.doc == st.doc().id);
    let summary = if let Some(running) = running_here {
        Line::styled(
            format!(
                "Running… {}  ({} cancels)",
                format_elapsed(running.started.elapsed()),
                key_text(app, Action::QueryCancel)
            ),
            Style::new().add_modifier(Modifier::BOLD),
        )
    } else if let Some(r) = results {
        summary_line(r.current(), r.output.elapsed, theme)
    } else {
        Line::styled(
            format!(
                "No results yet. Run a statement with {}.",
                global_key(app, Action::QueryRun)
            ),
            dim,
        )
    };
    frame.render_widget(Paragraph::new(summary), body.summary);

    let Some(results) = results else { return };
    if let Some(tabs_area) = body.tabs {
        let mut spans = Vec::new();
        for index in 0..results.tab_count() {
            let label = match results.tab(index) {
                Some(Tab::Set(set)) => set.tag.clone(),
                _ => "Error".to_string(),
            };
            let style = if index == results.tab {
                Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::new()
            };
            spans.push(Span::styled(format!(" {} {label} ", index + 1), style));
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!(
                "({} {})",
                key_text(app, Action::ResultsPrevTab),
                key_text(app, Action::ResultsNextTab)
            ),
            dim,
        ));
        frame.render_widget(Paragraph::new(Line::from(spans)), tabs_area);
    }

    match results.current() {
        Some(Tab::Set(set)) if set.returns_rows() => {
            draw_grid(frame, theme, set, results.row, results.col, &body)
        }
        Some(Tab::Error(error)) => {
            draw_error_details(frame, theme, error, body.header.union(body.rows))
        }
        _ => {}
    }
}

fn summary_line<'a>(tab: Option<Tab<'_>>, elapsed: std::time::Duration, theme: &Theme) -> Line<'a> {
    let took = format_elapsed(elapsed);
    match tab {
        Some(Tab::Set(set)) if set.returns_rows() => {
            let rows = if set.truncated() {
                format!(
                    "showing the first {} of {} rows (row limit)",
                    set.rows.len(),
                    set.total_rows
                )
            } else {
                format!(
                    "{} row{}",
                    set.total_rows,
                    if set.total_rows == 1 { "" } else { "s" }
                )
            };
            Line::from(format!("{}  ·  {rows}  ·  {took}", set.tag))
        }
        Some(Tab::Set(set)) => Line::from(format!("{}  ·  {took}", set.tag)),
        Some(Tab::Error(error)) => {
            let code = error
                .sqlstate
                .as_deref()
                .map(|c| format!(" {c}"))
                .unwrap_or_default();
            Line::styled(
                format!("ERROR{code}: {}", clean_cell(&error.message)),
                Style::new().fg(theme.error).add_modifier(Modifier::BOLD),
            )
        }
        None => Line::raw(""),
    }
}

fn draw_error_details(frame: &mut Frame, theme: &Theme, error: &DbError, area: Rect) {
    let mut lines = Vec::new();
    if let Some(detail) = &error.detail {
        lines.push(Line::raw(format!("Detail: {}", clean_cell(detail))));
    }
    if let Some(hint) = &error.hint {
        lines.push(Line::raw(format!("Hint: {}", clean_cell(hint))));
    }
    if let Some(position) = error.position {
        lines.push(Line::styled(
            format!("Position: character {position} of the statement"),
            Style::new().fg(theme.dimmed),
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_grid(
    frame: &mut Frame,
    theme: &Theme,
    set: &ResultSet,
    first_row: usize,
    first_col: usize,
    body: &layout::ResultsBody,
) {
    let widths = column_widths(set);
    let total_width = usize::from(body.rows.width);
    if total_width == 0 {
        return;
    }
    // Which columns fit, starting at `first_col`. The last may be cut.
    let mut visible: Vec<(usize, usize)> = Vec::new();
    let mut used = 0;
    for (index, width) in widths.iter().enumerate().skip(first_col) {
        let separator = if visible.is_empty() { 0 } else { 3 };
        if used + separator >= total_width {
            break;
        }
        let shown = (*width).min(total_width - used - separator);
        visible.push((index, shown));
        used += separator + shown;
    }

    let separator = || Span::styled(" │ ", Style::new().fg(theme.dimmed));
    // Bold on a filled bar, so the header stands out even without color.
    let header_style = Style::new()
        .fg(theme.header_fg)
        .bg(theme.header_bg)
        .add_modifier(Modifier::BOLD);
    let mut header = Vec::new();
    for (n, (index, width)) in visible.iter().enumerate() {
        if n > 0 {
            header.push(Span::styled(" │ ", header_style));
        }
        header.push(Span::styled(
            fit(&clean_cell(&set.columns[*index]), *width),
            header_style,
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(header)).style(header_style),
        body.header,
    );

    let null_style = Style::new()
        .fg(theme.null)
        .add_modifier(Modifier::ITALIC | Modifier::DIM);
    let lines: Vec<Line> = set
        .rows
        .iter()
        .skip(first_row)
        .take(usize::from(body.rows.height))
        .enumerate()
        .map(|(line_no, row)| {
            // Alternate row shading helps the eye follow a row across columns.
            let band = if (first_row + line_no) % 2 == 1 {
                Style::new().bg(theme.row_alt_bg)
            } else {
                Style::new()
            };
            let mut spans = Vec::new();
            for (n, (index, width)) in visible.iter().enumerate() {
                if n > 0 {
                    spans.push(separator());
                }
                // NULL is shown as a word in its own style, so it cannot be
                // mistaken for an empty string or for a text value (R12.3).
                spans.push(match row.get(*index).and_then(Option::as_deref) {
                    Some(text) => Span::raw(fit(&clean_cell(text), *width)),
                    None => Span::styled(fit("NULL", *width), null_style),
                });
            }
            Line::from(spans).style(band)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body.rows);
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
    let buffer = st.editor();
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
    let scroll_left = st.doc().scroll_left;
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
        let index = st.doc().scroll_top + row;
        let Some(line) = buffer.lines().get(index) else {
            break;
        };

        let number = format!(
            "{:>width$} ",
            index + 1,
            width = usize::from(area.gutter_width) - 1
        );
        let number_style = if index == cursor.line {
            Style::new().fg(theme.focus).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.line_number)
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
            let token = tokens.get(token_index).filter(|t| t.start <= absolute);
            let mut style = token.map_or(Style::new(), |t| token_style(t.kind, theme));
            // Keywords are shown in capitals. The buffer itself is not changed.
            let mut ch = ch;
            if token.is_some_and(|t| t.kind == TokenKind::Keyword) {
                let mut upper = ch.to_uppercase();
                if let (Some(u), None) = (upper.next(), upper.next()) {
                    ch = u;
                }
            }
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
        let mut line = Line::from(spans);
        if index == cursor.line {
            line = line.style(Style::new().bg(theme.cursor_line_bg));
        }
        rows.push(line);
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
        if cursor.line >= st.doc().scroll_top && x >= scroll_left {
            let (dx, dy) = (x - scroll_left, cursor.line - st.doc().scroll_top);
            if dx < usize::from(area.text.width) && dy < usize::from(area.text.height) {
                frame.set_cursor_position((area.text.x + dx as u16, area.text.y + dy as u16));
            }
        }
    }
}

/// The suggestion popup, below the word being completed (above it when
/// there is no room).
fn draw_completion(frame: &mut Frame, app: &App, theme: &Theme, pane: Rect) {
    let st = &app.state;
    let Some(completion) = &st.completion else {
        return;
    };
    let doc = st.doc();
    let area = layout::editor_area(pane, doc.buffer.line_count());
    let cursor = doc.buffer.cursor();
    if cursor.line < doc.scroll_top || completion.start.line < doc.scroll_top {
        return;
    }
    let start_line = doc
        .buffer
        .lines()
        .get(completion.start.line)
        .map_or("", String::as_str);
    let word_x = display_col(start_line, completion.start.col).saturating_sub(doc.scroll_left);
    let row = cursor.line - doc.scroll_top;

    let shown = completion.items.len().min(MAX_SUGGESTIONS);
    let label_width = completion
        .items
        .iter()
        .map(|c| c.text.chars().count())
        .max()
        .unwrap_or(0);
    // marker, label, space, tag, plus the border
    let width = (label_width + 2 + 1 + 2 + 2) as u16;
    let height = shown as u16 + 2;
    let frame_area = frame.area();

    let x = (area.text.x + word_x as u16).min(frame_area.right().saturating_sub(width));
    let below = area.text.y + row as u16 + 1;
    let y = if below + height <= frame_area.bottom() {
        below
    } else {
        (area.text.y + row as u16).saturating_sub(height)
    };
    let popup = Rect::new(
        x,
        y,
        width.min(frame_area.width),
        height.min(frame_area.height),
    );

    // Keep the selected item in view.
    let top = (completion.selected + 1).saturating_sub(shown);
    let lines: Vec<Line> = completion
        .items
        .iter()
        .enumerate()
        .skip(top)
        .take(shown)
        .map(|(i, item)| {
            let selected = i == completion.selected;
            let marker = if selected { "▶ " } else { "  " };
            let text = format!("{marker}{:<label_width$} {}", item.text, item.kind.tag());
            let style = if selected {
                Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::new()
            };
            Line::styled(text, style)
        })
        .collect();
    let block = Block::bordered().border_style(Style::new().fg(theme.focus));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// Suggestions shown at once.
const MAX_SUGGESTIONS: usize = 8;

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
    for action in app.hint_actions() {
        let Some(key) = app.hint_key(*action) else {
            continue;
        };
        let key = key_label(key);
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

/// Black or white, whichever reads better on `bg`.
fn readable_on(bg: Color, theme: &Theme) -> Color {
    let light = match bg {
        Color::Rgb(r, g, b) => {
            // Perceived brightness, 0 to 255.
            (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000 > 140
        }
        Color::White
        | Color::Gray
        | Color::Yellow
        | Color::LightYellow
        | Color::Cyan
        | Color::LightCyan
        | Color::LightGreen
        | Color::Green
        | Color::LightBlue
        | Color::LightMagenta
        | Color::LightRed => true,
        _ => false,
    };
    if light {
        Color::Black
    } else if bg == theme.status_bg {
        theme.status_fg
    } else {
        Color::White
    }
}

/// The connection part of the status bar (R3.1).
fn connection_text(app: &App) -> String {
    let Some(conn) = app.state.active_connection() else {
        return "no connection".to_string();
    };
    let label = conn.label();
    match (&conn.status, &conn.info) {
        (ConnStatus::Connected, Some(info)) => {
            let prod = if conn.profile.production { "PROD " } else { "" };
            let tls = if info.tls { "TLS" } else { "no TLS" };
            format!("{prod}{label} │ {} │ {} │ {tls}", info.database, info.user)
        }
        (ConnStatus::Connecting, _) => format!("connecting to {label}…"),
        (ConnStatus::AwaitingPassword, _) => format!("{label}: waiting for a password"),
        (ConnStatus::Failed(_), _) => format!("{label}: connection failed"),
        (ConnStatus::Closed(_), _) => format!("{label}: disconnected"),
        (ConnStatus::Connected, None) => label.to_string(),
    }
}

fn draw_status_bar(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let st = &app.state;
    let cursor = st.editor().cursor();
    let mut text = format!(
        " {} │ focus: {} │ Ln {}, Col {}",
        connection_text(app),
        st.focus.label(),
        cursor.line + 1,
        cursor.col + 1
    );
    match st.active_connection().map(|c| c.tx) {
        Some(TxState::Open) => text.push_str(" │ TX OPEN"),
        Some(TxState::Failed) => text.push_str(&format!(
            " │ TX FAILED ({} rolls back)",
            global_key(app, Action::TransactionRollback)
        )),
        _ => {}
    }
    if let Some(running) = st.running.as_ref().filter(|r| r.doc == st.doc().id) {
        text.push_str(&format!(
            " │ running {}",
            format_elapsed(running.started.elapsed())
        ));
    } else if let Some(results) = &st.doc().results {
        text.push_str(&format!(
            " │ last run {}",
            format_elapsed(results.output.elapsed)
        ));
    }
    // The notice is temporary, so it gets whatever room is left and is cut
    // first. The connection and focus must always stay visible (R3.1, R4.2).
    if let Some(notice) = &st.notice {
        let used = text.chars().count() + 3;
        let room = usize::from(area.width).saturating_sub(used);
        if room >= 12 {
            text.push_str(" │ ");
            text.push_str(fit(notice, room).trim_end());
        }
    }

    // A production connection gets its own color and the word PROD, so it is
    // never marked by color alone (R1.6, R20.9). A profile color tints the bar (R3.5).
    let conn = st
        .active_connection()
        .filter(|c| c.status == ConnStatus::Connected);
    let bg = match conn {
        Some(c) if c.profile.production => theme.status_prod,
        Some(c) => c.profile.color_value().unwrap_or(theme.status_bg),
        None => theme.status_bg,
    };
    let mut style = Style::new().fg(readable_on(bg, theme)).bg(bg);
    if conn.is_some_and(|c| c.profile.production) {
        style = style.add_modifier(Modifier::BOLD);
    }
    frame.render_widget(Paragraph::new(text).style(style), area);
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

/// Hides the password inside a connection URI before it is shown (R2.3).
fn redact_uri(text: &str) -> String {
    let Some(start) = text.find("://").map(|i| i + 3) else {
        return text.to_string();
    };
    let rest = &text[start..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return text.to_string();
    };
    let Some(colon) = rest[..at].find(':') else {
        return text.to_string();
    };
    format!(
        "{}{}{}",
        &text[..start + colon + 1],
        "•".repeat(rest[colon + 1..at].chars().count().min(12)),
        &rest[at..]
    )
}

fn draw_picker(frame: &mut Frame, app: &App, theme: &Theme, picker: &picker::Picker, area: Rect) {
    let dim = Style::new().fg(theme.dimmed);
    let reconnect = app.state.doc().connection.and_then(|id| {
        let c = app.state.connection(id)?;
        matches!(c.status, ConnStatus::Failed(_) | ConnStatus::Closed(_))
            .then(|| c.label().to_string())
    });
    let items = picker.items(&app.state.profiles, reconnect.as_deref());
    let visible = items.len().clamp(1, 8);
    let popup = layout::centered(area, 72, visible as u16 + 6);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Connect ")
        .title_alignment(Alignment::Center)
        .border_style(Style::new().fg(theme.focus));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 3 {
        return;
    }

    let shown = redact_uri(&picker.input);
    let prompt = format!("> {shown}");
    let input_row = Rect { height: 1, ..inner };
    if picker.input.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("> "),
                Span::styled("type to search, or paste a postgres:// address", dim),
            ])),
            input_row,
        );
    } else {
        frame.render_widget(Paragraph::new(prompt.clone()), input_row);
    }
    let cursor_x = inner.x
        + prompt
            .chars()
            .map(cell_width)
            .sum::<usize>()
            .min(usize::from(inner.width.saturating_sub(1))) as u16;
    frame.set_cursor_position((cursor_x, inner.y));

    let list = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(3),
        ..inner
    };
    let top = (picker.selected + 1).saturating_sub(usize::from(list.height));
    let mut lines: Vec<Line> = Vec::new();
    for (index, item) in items
        .iter()
        .enumerate()
        .skip(top)
        .take(usize::from(list.height))
    {
        let selected = index == picker.selected;
        let marker = if selected { "▶ " } else { "  " };
        let text = match item {
            Item::Uri(_) => "Connect to this address".to_string(),
            Item::Reconnect(label) => format!("Reconnect to {label}"),
            Item::New => "New connection: enter the details".to_string(),
            Item::Profile(name) => match app.state.profiles.get(name) {
                Some(p) => {
                    let prod = if p.production { "  [PROD]" } else { "" };
                    format!(
                        "{:<20} {}:{}/{}{prod}",
                        p.display_name(),
                        p.host,
                        p.port,
                        p.database.as_deref().unwrap_or("")
                    )
                }
                None => name.clone(),
            },
        };
        let style = if selected {
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::new()
        };
        lines.push(Line::styled(format!("{marker}{text}"), style));
    }
    if items.is_empty() {
        lines.push(Line::styled("Nothing to show.", dim));
    }
    frame.render_widget(Paragraph::new(lines), list);

    let key = |a| {
        app.keymap
            .key_for(a, &[Scope::Picker], |_| true)
            .map_or(String::new(), |k| k.to_string())
    };
    let footer = format!(
        "{} connect  {} edit  {} close  {} save address as profile",
        key(Action::DialogSubmit),
        key(Action::PickerEditProfile),
        key(Action::DialogCancel),
        key(Action::PickerSaveProfile)
    );
    let footer_area = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    frame.render_widget(Paragraph::new(Line::styled(footer, dim)), footer_area);
}

/// The find overlay: a search box over results grouped by kind (R9).
fn draw_find(frame: &mut Frame, app: &App, theme: &Theme, find: &picker::Picker, area: Rect) {
    let dim = Style::new().fg(theme.dimmed);
    let label = Style::new().fg(theme.label);
    let value = Style::new().fg(theme.value);
    let popup = layout::centered(area, 90, area.height.saturating_sub(4).clamp(8, 22));
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(match (&find.columns_of, find.kind) {
            (Some((schema, relation)), _) => format!(" Find: columns of {schema}.{relation} "),
            (None, Some(kind)) => format!(" Find: {} only ", kind.plural().to_lowercase()),
            (None, None) => " Find ".to_string(),
        })
        .title_alignment(Alignment::Center)
        .border_style(Style::new().fg(theme.focus));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 3 {
        return;
    }

    let prompt = format!("> {}", find.input);
    let input_row = Rect { height: 1, ..inner };
    if find.input.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("> "),
                Span::styled("type a table, view, column, function, type or schema", dim),
            ])),
            input_row,
        );
    } else {
        frame.render_widget(Paragraph::new(prompt.clone()), input_row);
    }
    let cursor_x = inner.x
        + prompt
            .chars()
            .map(cell_width)
            .sum::<usize>()
            .min(usize::from(inner.width.saturating_sub(1))) as u16;
    frame.set_cursor_position((cursor_x, inner.y));

    let list = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(3),
        ..inner
    };
    let width = usize::from(list.width);
    let connection = app
        .state
        .doc()
        .connection
        .and_then(|id| app.state.connection(id));
    let items = app.find_items(find);

    // Group headings are lines of their own. Remember which line holds the
    // selected item so it stays in view.
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_line = 0;
    let mut last_kind = None;
    for (index, object) in items.iter().enumerate() {
        if last_kind != Some(object.kind) {
            last_kind = Some(object.kind);
            lines.push(Line::styled(
                object.kind.plural(),
                label.add_modifier(Modifier::BOLD),
            ));
        }
        let selected = index == find.selected;
        if selected {
            selected_line = lines.len();
        }
        let marker = if selected { "▶ " } else { "  " };
        let path = object.path();
        let summary = object.summary();
        let path_width = width.saturating_sub(2 + summary.chars().count() + 2);
        let mut line = Line::from(vec![
            Span::styled(marker, value),
            Span::styled(fit(&path, path_width.max(8)), value),
            Span::raw("  "),
            Span::styled(summary, dim),
        ]);
        if selected {
            line = line.style(Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD));
        }
        lines.push(line);
    }
    if items.is_empty() {
        let message = match connection {
            Some(c) if c.catalog_error.is_some() => {
                format!(
                    "Could not read the schema: {}",
                    c.catalog_error.as_deref().unwrap_or("")
                )
            }
            Some(c) if c.catalog.is_none() => "Reading the schema…".to_string(),
            Some(c) if c.catalog.as_ref().is_some_and(|k| k.is_empty()) => {
                "This database has no tables, views or functions you can see".to_string()
            }
            _ if find.columns_of.is_some() => "No column matches".to_string(),
            _ => match find.kind {
                Some(kind) => format!("No {} matches", kind.label()),
                None => "Nothing matches".to_string(),
            },
        };
        lines.push(Line::styled(message, dim));
    }
    let top = (selected_line + 1).saturating_sub(usize::from(list.height));
    let visible: Vec<Line> = lines
        .into_iter()
        .skip(top)
        .take(usize::from(list.height))
        .collect();
    frame.render_widget(Paragraph::new(visible), list);

    let key = |a| {
        app.keymap
            .key_for(a, &[Scope::Find], |_| true)
            .map_or(String::new(), |k| k.to_string())
    };
    // The columns key is offered only where it works: on a table or view,
    // or to leave a column list.
    let selected_is_relation = items
        .get(find.selected)
        .is_some_and(|o| o.kind.is_relation());
    let columns_hint = if find.columns_of.is_some() {
        format!("  {} all objects", key(Action::FindColumns))
    } else if selected_is_relation {
        format!("  {} columns of this one", key(Action::FindColumns))
    } else {
        String::new()
    };
    let footer = format!(
        "{} insert name  {} select sample rows{columns_hint}  {} close",
        key(Action::DialogSubmit),
        key(Action::FindSample),
        key(Action::DialogCancel)
    );
    // The jump keys share a modifier by default, so it is shown once.
    let jumps: Vec<(String, &str)> = catalog::Kind::ORDER
        .iter()
        .filter(|kind| **kind != catalog::Kind::Column)
        .filter_map(|kind| {
            let key = app
                .keymap
                .key_for(Action::FindJump(*kind), &[Scope::Find], |_| true)?;
            Some((key.to_string(), kind.short()))
        })
        .collect();
    let prefix = jumps
        .first()
        .and_then(|(k, _)| k.rsplit_once('+'))
        .map(|(p, _)| format!("{p}+"))
        .filter(|p| jumps.iter().all(|(k, _)| k.starts_with(p.as_str())));
    let mut jump_line = String::from("Only: ");
    for (n, (key, label)) in jumps.iter().enumerate() {
        let shown = match (&prefix, n) {
            (Some(p), 1..) => key.strip_prefix(p.as_str()).unwrap_or(key),
            _ => key,
        };
        if n > 0 {
            jump_line.push_str("  ");
        }
        jump_line.push_str(&format!("{shown} {label}"));
    }
    let footer_area = Rect {
        y: inner.y + inner.height - 2,
        height: 2,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(footer, dim),
            Line::styled(jump_line, dim),
        ]),
        footer_area,
    );
}

fn draw_form(
    frame: &mut Frame,
    app: &App,
    theme: &Theme,
    form: &crate::form::ConnectForm,
    area: Rect,
) {
    let dim = Style::new().fg(theme.dimmed);
    let popup = layout::centered(area, 70, 15);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(if form.editing.is_some() {
            " Edit connection "
        } else {
            " New connection "
        })
        .title_alignment(Alignment::Center)
        .border_style(Style::new().fg(theme.focus));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 10 {
        return;
    }
    let row = |n: u16| Rect {
        y: inner.y + n,
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            "Passwords are never saved. You are asked for it next.",
            dim,
        )),
        row(0),
    );

    let hints = [
        "",
        "",
        "(empty: same as the user)",
        "",
        "(empty: do not save this connection)",
        "(Space to change. TLS is not supported yet)",
        "(Space to switch)",
    ];
    let label_width = crate::form::LABELS
        .iter()
        .map(|l| l.len())
        .max()
        .unwrap_or(0);
    for (index, label) in crate::form::LABELS.iter().enumerate() {
        let focused = form.focus == index;
        let marker = if focused { "▶ " } else { "  " };
        let mut spans = vec![Span::styled(
            format!("{marker}{label:<label_width$}  "),
            if focused {
                Style::new().add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            },
        )];
        match form.text(index) {
            Some("") => spans.push(Span::styled(hints[index], dim)),
            Some(text) => spans.push(Span::styled(
                text.to_string(),
                if focused {
                    Style::new().add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::new()
                },
            )),
            None => {
                let mark = if index == 5 {
                    form.ssl_mode.name()
                } else if form.production {
                    "[x] yes, mark as production"
                } else {
                    "[ ] no"
                };
                spans.push(Span::raw(mark));
                spans.push(Span::styled(format!("  {}", hints[index]), dim));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row(2 + index as u16));
    }

    // The cursor sits at the end of the text being edited.
    if let Some(text) = form.text(form.focus) {
        let x =
            inner.x + (2 + label_width + 2 + text.chars().map(cell_width).sum::<usize>()) as u16;
        frame.set_cursor_position((
            x.min(inner.right().saturating_sub(1)),
            inner.y + 2 + form.focus as u16,
        ));
    }

    if let Some(error) = &form.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::new().fg(theme.error).add_modifier(Modifier::BOLD)),
            row(9),
        );
    }
    let key = |a| {
        app.keymap
            .key_for(a, &[Scope::Form], |_| true)
            .map_or(String::new(), key_label)
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "{} next field  {} {}  {} back",
                key(Action::DialogNext),
                key(Action::DialogSubmit),
                if form.editing.is_some() {
                    "save"
                } else {
                    "connect"
                },
                key(Action::DialogCancel)
            ),
            dim,
        )),
        row(inner.height - 1),
    );
}

fn draw_session_form(
    frame: &mut Frame,
    app: &App,
    theme: &Theme,
    form: &crate::form::SessionForm,
    area: Rect,
) {
    let dim = Style::new().fg(theme.dimmed);
    let popup = layout::centered(area, 60, 9);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Change session ")
        .title_alignment(Alignment::Center)
        .border_style(Style::new().fg(theme.focus));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 7 {
        return;
    }
    let row = |n: u16| Rect {
        y: inner.y + n,
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            "Saving connects again. You may be asked for the password.",
            dim,
        )),
        row(0),
    );
    let label_width = crate::form::SESSION_LABELS
        .iter()
        .map(|l| l.len())
        .max()
        .unwrap_or(0);
    for (index, label) in crate::form::SESSION_LABELS.iter().enumerate() {
        let focused = form.focus == index;
        let marker = if focused { "▶ " } else { "  " };
        let text = form.text(index);
        let mut spans = vec![Span::styled(
            format!("{marker}{label:<label_width$}  "),
            if focused {
                Style::new().add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            },
        )];
        if text.is_empty() {
            spans.push(Span::styled("(empty: same as the user)", dim));
        } else {
            spans.push(Span::raw(text.to_string()));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row(2 + index as u16));
    }
    let text = form.text(form.focus);
    let x = inner.x + (2 + label_width + 2 + text.chars().map(cell_width).sum::<usize>()) as u16;
    frame.set_cursor_position((
        x.min(inner.right().saturating_sub(1)),
        inner.y + 2 + form.focus as u16,
    ));
    if let Some(error) = &form.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::new().fg(theme.error).add_modifier(Modifier::BOLD)),
            row(5),
        );
    }
    let key = |a| {
        app.keymap
            .key_for(a, &[Scope::Form], |_| true)
            .map_or(String::new(), key_label)
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "{} next field  {} reconnect  {} cancel",
                key(Action::DialogNext),
                key(Action::DialogSubmit),
                key(Action::DialogCancel)
            ),
            dim,
        )),
        row(inner.height - 1),
    );
}

fn draw_confirm(
    frame: &mut Frame,
    app: &App,
    theme: &Theme,
    confirm: &crate::app::Confirm,
    area: Rect,
) {
    let dim = Style::new().fg(theme.dimmed);
    let danger = Style::new().fg(theme.error).add_modifier(Modifier::BOLD);
    let reasons = confirm.reasons();
    // Statement lines, at most 6, each cut to the box.
    let statement: Vec<&str> = confirm
        .sql
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let shown = statement.len().min(6);
    let height = (shown + reasons.len() + 8) as u16;
    let popup = layout::centered(area, 72, height);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Confirm ")
        .title_alignment(Alignment::Center)
        .border_style(Style::new().fg(theme.error));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let width = usize::from(inner.width);
    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled("Connection  ", dim),
        Span::styled(
            confirm.label.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Span::styled(if confirm.production { "  PROD" } else { "" }, danger),
    ])];
    lines.push(Line::raw(""));
    for line in statement.iter().take(shown) {
        lines.push(Line::raw(format!(
            "  {}",
            fit(line.trim_end(), width.saturating_sub(2))
        )));
    }
    if statement.len() > shown {
        lines.push(Line::styled(
            format!("  … {} more lines", statement.len() - shown),
            dim,
        ));
    }
    lines.push(Line::raw(""));
    for reason in &reasons {
        lines.push(Line::styled(fit(reason, width), danger));
    }
    lines.push(Line::raw(""));
    let prompt = format!("Run it? Type y or yes: {}", confirm.input);
    let prompt_row = lines.len() as u16;
    let prompt_width: usize = prompt.chars().map(cell_width).sum();
    lines.push(Line::raw(prompt));
    if let Some(error) = &confirm.error {
        lines.push(Line::styled(fit(error, width), danger));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    frame.set_cursor_position((
        (inner.x + prompt_width as u16).min(inner.right().saturating_sub(1)),
        inner.y + prompt_row,
    ));
    let key = |a| {
        app.keymap
            .key_for(a, &[Scope::Form], |_| true)
            .map_or(String::new(), key_label)
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "{} run (after y or yes)  {} cancel",
                key(Action::DialogSubmit),
                key(Action::DialogCancel)
            ),
            dim,
        )),
        Rect {
            y: inner.bottom().saturating_sub(1),
            height: 1,
            ..inner
        },
    );
}

fn draw_password(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let Some(Dialog::Password(prompt)) = &app.state.dialog else {
        return;
    };
    let dim = Style::new().fg(theme.dimmed);
    let conn = app.state.connection(prompt.conn);
    let who = conn.map_or(String::new(), |c| {
        let user = c
            .profile
            .user
            .clone()
            .unwrap_or_else(crate::runtime::default_user);
        format!("{user}@{}:{}", c.profile.host, c.profile.port)
    });
    let popup = layout::centered(area, 60, 8);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Password ")
        .title_alignment(Alignment::Center)
        .border_style(Style::new().fg(theme.focus));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 5 {
        return;
    }
    let row = |n: u16| Rect {
        y: inner.y + n,
        height: 1,
        ..inner
    };
    frame.render_widget(Paragraph::new(format!("Password for {who}")), row(0));
    if let Some(error) = &prompt.error {
        frame.render_widget(
            Paragraph::new(format!("Rejected: {}", clean_cell(error)))
                .style(Style::new().fg(theme.error).add_modifier(Modifier::BOLD)),
            row(1),
        );
    }
    // The typed password is never shown, only how many characters it has.
    let masked = "•".repeat(prompt.input.expose().chars().count().min(40));
    let line = format!("> {masked}");
    frame.render_widget(Paragraph::new(line.clone()), row(2));
    frame.set_cursor_position((inner.x + line.chars().count() as u16, inner.y + 2));
    let key = |a| {
        app.keymap
            .key_for(a, &[Scope::Prompt], |_| true)
            .map_or(String::new(), |k| k.to_string())
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "{} connect (empty: no password)  {} cancel",
                key(Action::DialogSubmit),
                key(Action::DialogCancel)
            ),
            dim,
        )),
        row(4),
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

    /// An app as it is at startup, on the commands pane.
    fn landing(width: u16, height: u16) -> App {
        let mut app = App::new(Config::default(), Keymap::default_keymap());
        app.update(Event::Resize(width, height));
        app
    }

    /// An app focused on the editor, so tests can type at once.
    fn app(width: u16, height: u16) -> App {
        let mut app = landing(width, height);
        app.state.focus = Pane::Editor;
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

    /// Screen row of the editor's top border. The context row above it is 7 rows.
    const EDITOR_TOP: u16 = 7;

    /// The style at a position inside the editor text (row 0 is its first line).
    fn editor_style(app: &App, width: u16, height: u16, x: u16, row: u16) -> Style {
        cell_style(app, width, height, x, EDITOR_TOP + 1 + row)
    }

    const SAMPLE: &str =
        "-- users per day\nselect id, name\n  from users\n where name = 'a;b' and n > 10;";

    #[test]
    fn the_screen_at_startup_invites_you_to_connect() {
        let out = render(&landing(80, 24), 80, 24);
        for expected in [
            "Welcome to oxpecker.",
            "Press C to connect to a database.",
            "C connect  F find  R run  E edit  N new buffer  Q quit  ? help",
            "┌ ▶ Context",
            "Session",
            "Not connected",
            "No results yet. Run a statement with Ctrl+R.",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        // The panes run top to bottom: where you are, what you write, the results.
        let at = |title: &str| out.find(title).unwrap_or_else(|| panic!("no {title}"));
        assert!(
            at("Context") < at("Session")
                && at("Session") < at("Editor")
                && at("Editor") < at("Results"),
            "{out}"
        );
        assert_snapshot("landing_80x24", &out);
    }

    fn with_profiles(mut a: App) -> App {
        for (name, host, db, prod) in [
            ("billing", "billing.example.com", "ledger", true),
            ("local dev", "localhost", "dev", false),
        ] {
            let mut p = crate::profile::Profile::new(name);
            p.host = host.to_string();
            p.database = Some(db.to_string());
            p.production = prod;
            a.state.profiles.add(p).expect("add");
        }
        a
    }

    #[test]
    fn the_connect_window_lists_profiles_then_new_connection() {
        let mut a = with_profiles(landing(90, 24));
        key(&mut a, KeyCode::Char('c'), KeyModifiers::NONE);
        let out = render(&a, 90, 24);
        for expected in [
            "type to search, or paste a postgres:// address",
            "▶ billing",
            "[PROD]",
            "local dev",
            "New connection: enter the details",
            "Enter connect  Ctrl+E edit  Esc close",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        assert_snapshot("connect_window_90x24", &out);
    }

    #[test]
    fn typing_in_the_connect_window_narrows_the_list() {
        let mut a = with_profiles(landing(90, 24));
        key(&mut a, KeyCode::Char('c'), KeyModifiers::NONE);
        type_text(&mut a, "loc");
        let out = render(&a, 90, 24);
        assert!(
            out.contains("▶ local dev") && !out.contains("billing"),
            "{out}"
        );
    }

    #[test]
    fn the_details_form() {
        let mut a = landing(90, 26);
        // The suggested user is the OS login, so fix it to keep the snapshot portable.
        a.state.dialog = Some(Dialog::Form(crate::form::ConnectForm::new("alice")));
        let out = render(&a, 90, 26);
        for expected in [
            "New connection",
            "Passwords are never saved",
            "▶ Host",
            "localhost",
            "5432",
            "(empty: same as the user)",
            "(empty: do not save this connection)",
            "[ ] no",
            "Tab next field  Enter connect  Esc back",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        assert_snapshot("connect_form_90x26", &out);
    }

    #[test]
    fn a_form_error_is_shown_in_the_form() {
        let mut a = landing(90, 26);
        key(&mut a, KeyCode::Char('c'), KeyModifiers::NONE);
        key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        key(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(render(&a, 90, 26).contains("The port must be a number from 1 to 65535"));
    }

    /// A landing app with an open connection to alice@app.
    fn connected_app(width: u16, height: u16) -> App {
        let mut a = landing(width, height);
        key(&mut a, KeyCode::Char('c'), KeyModifiers::NONE);
        type_text(&mut a, "postgres://alice@127.0.0.1:5432/app");
        let commands = a.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let crate::command::Command::Connect { conn, .. } = commands[0].clone() else {
            panic!("connect")
        };
        a.update(Event::Background(crate::events::Background::Connected {
            conn,
            info: crate::db::ConnectionInfo {
                host: "127.0.0.1".to_string(),
                port: 5432,
                database: "app".to_string(),
                user: "alice".to_string(),
                server_version: "16.2".to_string(),
                tls: false,
            },
        }));
        a
    }

    #[test]
    fn the_session_pane_shows_the_database_and_user() {
        let a = connected_app(90, 24);
        let out = render(&a, 90, 24);
        for expected in [
            "Session",
            "app · alice",
            "127.0.0.1:5432  PostgreSQL 16.2",
            "Buffer     Buffer 1",
            "Statement  none under the cursor",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        assert!(
            !out.contains("other connection"),
            "keys live in the hint bar"
        );
        assert_snapshot("connected_90x24", &out);
    }

    #[test]
    fn the_focused_session_pane_and_its_form() {
        let mut a = connected_app(90, 24);
        key(&mut a, KeyCode::Char('4'), KeyModifiers::ALT);
        let out = render(&a, 90, 24);
        assert!(out.contains("┌ ▶ Session"), "{out}");
        assert!(out.contains("D change database  U change user"), "{out}");
        key(&mut a, KeyCode::Char('u'), KeyModifiers::NONE);
        let out = render(&a, 90, 24);
        assert!(
            out.contains("Change session") && out.contains("▶ User      alice"),
            "{out}"
        );
        assert_snapshot("session_form_90x24", &out);
    }

    #[test]
    fn a_risky_statement_shows_the_connection_the_statement_and_why() {
        let mut a = connected_app(90, 24);
        if let Some(c) = a.state.active_connection().map(|c| c.id) {
            let conn = a.state.connection_mut_for_tests(c);
            conn.profile.production = true;
        }
        key(&mut a, KeyCode::Char('2'), KeyModifiers::ALT);
        type_text(&mut a, "delete from orders");
        key(&mut a, KeyCode::Char('r'), KeyModifiers::CONTROL);
        let out = render(&a, 90, 24);
        for expected in [
            "Confirm",
            "Connection  127.0.0.1  PROD",
            "delete from orders",
            "production connection",
            "without WHERE",
            "Run it? Type y or yes:",
            "Enter run (after y or yes)  Esc cancel",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        assert_snapshot("confirm_90x24", &out);
    }

    #[test]
    fn a_failed_transaction_is_flagged_in_the_session_pane_and_status_bar() {
        let mut a = connected_app(90, 24);
        let conn = a.state.active_connection().map(|c| c.id).expect("conn");
        a.state.connection_mut_for_tests(conn).tx = crate::db::TxState::Failed;
        let out = render(&a, 90, 24);
        assert!(
            out.contains("Transaction FAILED. Alt+Z rolls back"),
            "{out}"
        );
        assert!(out.contains("TX FAILED"), "{out}");
        assert_snapshot("tx_failed_90x24", &out);
    }

    fn sample_stats() -> SessionStats {
        SessionStats {
            connections: 3,
            server_connections: 12,
            max_connections: 100,
            active: Some(1),
            idle_in_transaction: Some(0),
            oldest_transaction_secs: Some(12),
            size_bytes: 1_288_490_188,
            cache_hit_percent: Some(99.2),
            deadlocks: 0,
            uptime_secs: 3 * 86400 + 4 * 3600,
            sees_all_sessions: true,
            top_tables: Vec::new(),
        }
    }

    fn with_stats(width: u16, height: u16, stats: SessionStats) -> App {
        let mut a = connected_app(width, height);
        let id = a.state.active_connection().map(|c| c.id).expect("conn");
        a.state.connection_mut_for_tests(id).stats = Some(stats);
        a
    }

    #[test]
    fn a_wide_session_pane_has_identity_on_the_left_and_the_server_on_the_right() {
        let a = with_stats(130, 24, sample_stats());
        let out = render(&a, 130, 24);
        for expected in [
            "Database    app",
            "User        alice",
            "127.0.0.1:5432  PostgreSQL 16.2",
            "Connections 3 here · 12 of 100 on server",
            "Activity    1 active · 0 idle in transaction",
            "Size        1.2 GB · cache hit 99.2%",
            "Health      oldest tx 12s · up 3d 4h · deadlocks 0",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        assert_snapshot("session_wide_130x24", &out);
    }

    #[test]
    fn a_narrow_session_pane_keeps_the_numbers_that_matter() {
        let a = with_stats(90, 24, sample_stats());
        let out = render(&a, 90, 24);
        for expected in [
            "app · alice",
            "Conns 3 here · 12/100 on server",
            "1.2 GB · cache hit 99.2%",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        assert_snapshot("session_compact_90x24", &out);
    }

    #[test]
    fn the_session_pane_is_wider_than_the_context_pane() {
        for width in [80u16, 90, 120, 160] {
            let a = connected_app(width, 24);
            let areas = layout::compute(Rect::new(0, 0, width, 24), &a.state, &a.config);
            let (c, s) = (areas.context.expect("c"), areas.session.expect("s"));
            assert!(s.width > c.width, "{width}: {c:?} {s:?}");
            let trends = areas.trends.map_or(0, |t| t.width);
            assert_eq!(c.width + s.width + trends, width);
        }
    }

    #[test]
    fn a_crowded_server_and_a_role_without_rights_are_shown_plainly() {
        let mut stats = sample_stats();
        stats.server_connections = 90;
        stats.sees_all_sessions = false;
        stats.active = Some(0);
        stats.oldest_transaction_secs = None;
        let a = with_stats(130, 24, stats);
        let out = render(&a, 130, 24);
        assert!(out.contains("90 of 100 on server"), "{out}");
        assert!(out.contains("needs the pg_read_all_stats role"), "{out}");
        assert!(out.contains("oldest tx n/a"), "{out}");
    }

    #[test]
    fn statistics_that_cannot_be_read_say_so() {
        let mut a = connected_app(130, 24);
        let id = a.state.active_connection().map(|c| c.id).expect("conn");
        a.state.connection_mut_for_tests(id).stats_error = Some("permission denied".to_string());
        let out = render(&a, 130, 24);
        assert!(
            out.contains("Statistics unavailable: permission denied"),
            "{out}"
        );
    }

    #[test]
    fn sizes_and_ages_read_naturally() {
        assert_eq!(format_bytes(900), "900 B");
        assert_eq!(format_bytes(1536), "1.5 kB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
        assert_eq!(format_age(7), "7s");
        assert_eq!(format_age(300), "5m");
        assert_eq!(format_age(3 * 3600 + 20 * 60), "3h 20m");
        assert_eq!(format_age(2 * 86400 + 5 * 3600), "2d 5h");
    }

    #[test]
    fn the_password_prompt_masks_what_is_typed() {
        let mut a = landing(90, 24);
        key(&mut a, KeyCode::Char('c'), KeyModifiers::NONE);
        type_text(&mut a, "postgres://alice@127.0.0.1:5432/app");
        let commands = a.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let crate::command::Command::Connect { conn, .. } = commands[0].clone() else {
            panic!("connect")
        };
        a.update(Event::Background(
            crate::events::Background::PasswordNeeded {
                conn,
                notes: Vec::new(),
            },
        ));
        type_text(&mut a, "hunter22");
        let out = render(&a, 90, 24);
        assert!(out.contains("Password for alice@127.0.0.1:5432"), "{out}");
        assert!(
            out.contains("> ••••••••") && !out.contains("hunter22"),
            "{out}"
        );
        assert_snapshot("password_prompt_90x24", &out);
    }

    #[test]
    fn the_editor_screen_80x24() {
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
        let bottom = render(&a, 80, 24);
        assert!(!bottom.contains("editor keys"), "scrolled past the top");
        assert!(
            bottom.contains("focus.next"),
            "the last global binding is visible"
        );
        assert_snapshot("editor_help_top_80x24", &top);
    }

    #[test]
    fn suggestion_popup_is_drawn_under_the_word() {
        let mut a = app(80, 24);
        type_text(&mut a, "select 1\nfrom t\nwhere x\nde");
        key(&mut a, KeyCode::Down, KeyModifiers::NONE);
        let out = render(&a, 80, 24);
        assert!(out.contains("▶ delete"), "{out}");
        assert!(out.contains("  desc") && out.contains("declare"));
        // The popup sits under the word and the hint bar shows its keys.
        let hint = out.lines().rev().nth(1).expect("hint line");
        assert!(hint.contains("Enter accept suggestion"), "{hint}");
        assert_snapshot("completion_80x24", &out);
    }

    #[test]
    fn suggestion_popup_flips_above_when_there_is_no_room() {
        let mut a = app(80, 24);
        for _ in 0..3 {
            type_text(&mut a, "x\n");
        }
        type_text(&mut a, "de");
        // Shrink so that the cursor row is the last one in the editor pane.
        a.update(Event::Resize(80, 16));
        let out = render(&a, 80, 16);
        assert!(out.contains("declare"), "{out}");
        let popup_row = out
            .lines()
            .position(|l| l.contains("declare"))
            .expect("row");
        let cursor_row = out
            .lines()
            .position(|l| l.contains("4 de"))
            .expect("cursor row");
        assert!(popup_row != cursor_row);
    }

    #[test]
    fn the_selected_suggestion_is_not_marked_by_color_alone() {
        let mut a = app(80, 24);
        type_text(&mut a, "de");
        // The cursor is on row 1. Row 2 is the popup border, row 3 the first
        // item, which is selected, and row 4 the second.
        let selected = cell_style(&a, 80, 24, 5, EDITOR_TOP + 3);
        assert!(selected.add_modifier.contains(Modifier::REVERSED));
        let other = cell_style(&a, 80, 24, 5, EDITOR_TOP + 4);
        assert!(!other.add_modifier.contains(Modifier::REVERSED));
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
        assert!(!out.contains("Editor") && !out.contains("Context"));
        assert_snapshot("too_small_40x10", &out);
    }

    #[test]
    fn too_small_terminal_still_shows_the_editor_when_focused() {
        let mut a = app(40, 10);
        type_text(&mut a, "select 1");
        let out = render(&a, 40, 10);
        assert!(out.contains("SELECT 1") && out.contains("Terminal too small"));
    }

    #[test]
    fn editor_title_names_the_buffer_only_when_there_are_several() {
        let mut a = app(80, 24);
        assert!(render(&a, 80, 24).contains("┌ ▶ Editor ─"));
        key(&mut a, KeyCode::Char('n'), KeyModifiers::CONTROL);
        type_text(&mut a, "x");
        let out = render(&a, 80, 24);
        assert!(out.contains("┌ ▶ Editor: Buffer 2 (2/2) ─"), "{out}");
        assert!(
            out.lines().nth(8).is_some_and(|l| l.starts_with("│1 x")),
            "{out}"
        );
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
        // The editor hint is short: how to run and how to leave.
        assert!(
            hint.contains("Ctrl+R run") && hint.contains("Esc commands"),
            "{hint}"
        );
        // No function keys on the default screen.
        assert!(
            !hint.contains('F')
                || !hint
                    .chars()
                    .zip(hint.chars().skip(1))
                    .any(|(a, b)| a == 'F' && b.is_ascii_digit()),
            "{hint}"
        );
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
        let comment = editor_style(&a, 80, 24, 3, 0);
        assert_eq!(comment.fg, Some(theme.dimmed));
        assert!(comment.add_modifier.contains(Modifier::ITALIC));
        // Line 2: `select` is a keyword, `id` is not.
        let keyword = editor_style(&a, 80, 24, 3, 1);
        assert_eq!(keyword.fg, Some(theme.keyword));
        assert!(keyword.add_modifier.contains(Modifier::BOLD));
        assert_ne!(editor_style(&a, 80, 24, 10, 1).fg, Some(theme.keyword));
        // Line 4: the string, including its semicolon, is one string token.
        let string_col = 3 + " where name = ".len() as u16 + 4;
        assert_eq!(
            editor_style(&a, 80, 24, string_col, 3).fg,
            Some(theme.string)
        );
    }

    #[test]
    fn selection_is_reversed_and_not_color_only() {
        let mut a = app(80, 24);
        type_text(&mut a, "abc");
        key(&mut a, KeyCode::Left, KeyModifiers::SHIFT);
        assert!(editor_style(&a, 80, 24, 5, 0)
            .add_modifier
            .contains(Modifier::REVERSED));
        assert!(!editor_style(&a, 80, 24, 4, 0)
            .add_modifier
            .contains(Modifier::REVERSED));
    }

    #[test]
    fn matching_brackets_are_marked() {
        let mut a = app(80, 24);
        type_text(&mut a, "f(1)");
        key(&mut a, KeyCode::Left, KeyModifiers::NONE); // cursor between `1` and `)`
        let open = editor_style(&a, 80, 24, 4, 0);
        let close = editor_style(&a, 80, 24, 6, 0);
        for style in [open, close] {
            assert!(style.add_modifier.contains(Modifier::UNDERLINED));
        }
        assert!(!editor_style(&a, 80, 24, 5, 0)
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
        assert_eq!((pos.x, pos.y), (3 + 3, EDITOR_TOP + 2));
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
        let row = out.lines().nth(8).expect("first editor row");
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
        assert!(!out.lines().nth(8).is_some_and(|l| l.starts_with("│ 1 ")));
    }

    // Keep the `Color` import used even if themes change.
    #[test]
    fn default_theme_uses_real_colors() {
        assert_ne!(Theme::default().keyword, Color::Reset);
    }

    #[test]
    fn the_find_window_groups_results_and_shows_the_jump_keys() {
        use crate::catalog::{Catalog, Kind, Object};
        let mut a = connected_app(90, 24);
        let object = |kind, schema: &str, name: &str, parent: Option<&str>, detail: &str| Object {
            kind,
            schema: schema.to_string(),
            name: name.to_string(),
            parent: parent.map(str::to_string),
            detail: detail.to_string(),
        };
        let conn = a.state.connections[0].id;
        a.update(Event::Background(crate::events::Background::Catalog {
            conn,
            result: Ok(Catalog {
                objects: vec![
                    object(Kind::Table, "public", "orders", None, ""),
                    object(Kind::Table, "archive", "orders", None, ""),
                    object(Kind::View, "public", "order_totals", None, ""),
                    object(
                        Kind::Column,
                        "public",
                        "order_id",
                        Some("orders"),
                        "integer",
                    ),
                    object(
                        Kind::Function,
                        "public",
                        "refresh_orders",
                        None,
                        "since date",
                    ),
                ],
            }),
        }));
        key(&mut a, KeyCode::Char('p'), KeyModifiers::CONTROL);
        type_text(&mut a, "order");
        let out = render(&a, 90, 24);
        for expected in [
            "Find",
            "> order",
            "Tables",
            "archive.orders",
            "Views",
            "Columns",
            "public.orders.order_id",
            "Functions",
            "(since date)",
            "Enter insert name",
            "Only: Alt+T table  V view",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        assert_snapshot("find_90x24", &out);
    }

    fn table_size(schema: &str, name: &str, bytes: u64) -> crate::db::TableSize {
        crate::db::TableSize {
            schema: schema.to_string(),
            name: name.to_string(),
            bytes,
            rows_estimate: Some(1000),
        }
    }

    #[test]
    fn a_wide_terminal_shows_the_largest_tables_beside_the_session() {
        let mut stats = sample_stats();
        stats.top_tables = vec![
            table_size("public", "events", 900_000_000),
            table_size("public", "orders", 450_000_000),
            table_size("archive", "orders_2023", 120_000_000),
            table_size("public", "users", 8_000_000),
            table_size("public", "tiny", 10_000),
        ];
        let a = with_stats(160, 24, stats);
        let out = render(&a, 160, 24);
        for expected in [
            "Trends: largest tables",
            "events",
            "archive.orders_2023",
            "858.3 MB",
            "9.8 kB",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        // The bar of the largest table is full, and a small table keeps one block.
        let events = out.lines().find(|l| l.contains("events")).expect("row");
        assert!(events.contains(&"█".repeat(8)), "{events}");
        let tiny = out.lines().find(|l| l.contains("tiny")).expect("row");
        assert!(tiny.contains('█'), "{tiny}");
        assert_snapshot("trends_160x24", &out);
    }

    #[test]
    fn a_narrower_terminal_does_not_draw_the_trends() {
        let a = with_stats(130, 24, sample_stats());
        assert!(!render(&a, 130, 24).contains("┌ Trends"));
    }

    #[test]
    fn f7_hides_and_shows_the_trends() {
        let mut a = with_stats(160, 24, sample_stats());
        key(&mut a, KeyCode::F(7), KeyModifiers::NONE);
        assert!(!render(&a, 160, 24).contains("┌ Trends"));
        key(&mut a, KeyCode::F(7), KeyModifiers::NONE);
        assert!(render(&a, 160, 24).contains("┌ Trends"));
    }
}
