//! Application state and the update function. Nothing here draws or does I/O.

use crossterm::event::KeyCode;
use ratatui::layout::Rect;

use std::time::Instant;

use crate::catalog::{Catalog, Object as CatalogObject};
use crate::command::{Command, DocId};
use crate::complete::{self, Candidate, KeywordSource};
use crate::config::Config;
use crate::db::{ConnId, ConnectionInfo, SessionStats, TxState};
use crate::editor::{display_col, Buffer, EditCmd, Motion, Pos};
use crate::events::{Background, Event};
use crate::form::{ConnectForm, SessionForm};
use crate::keymap::{Action, KeyChord, Keymap, Scope};
use crate::layout;
use crate::picker::{self, Picker};
use crate::profile::{self, Profile, Profiles};
use crate::results::Results;
use crate::secrets::{Secret, Source};
use crate::sql;

/// A top-level pane that can hold focus. The status bar never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Editor,
    Context,
    Session,
    Results,
}

impl Pane {
    pub fn label(self) -> &'static str {
        match self {
            Pane::Editor => "Editor",
            Pane::Context => "Context",
            Pane::Session => "Session",
            Pane::Results => "Results",
        }
    }

    pub fn scope(self) -> Scope {
        match self {
            Pane::Editor => Scope::Editor,
            Pane::Context => Scope::Context,
            Pane::Session => Scope::Session,
            Pane::Results => Scope::Results,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// Exactly one pane has focus at a time (R4.2).
    pub focus: Pane,
    pub show_context: bool,
    pub show_results: bool,
    pub help_open: bool,
    /// First visible line of the help overlay.
    pub help_scroll: usize,
    pub quit: bool,
    /// One-line message for the status bar.
    pub notice: Option<String>,
    pub size: (u16, u16),
    /// Open buffers, in tab order. Never empty.
    pub docs: Vec<Document>,
    /// Index of the buffer shown in the editor.
    pub active: usize,
    /// Recently closed buffers, newest last, so a close can be undone.
    pub closed: Vec<Document>,
    /// Used to name new buffers.
    pub buffers_created: usize,
    pub completion: Option<Completion>,
    pub connections: Vec<Connection>,
    pub next_conn: ConnId,
    /// Saved connection profiles.
    pub profiles: Profiles,
    /// A modal box that takes all keys, such as the connection picker.
    pub dialog: Option<Dialog>,
    /// The statement being executed, if any.
    pub running: Option<Running>,
    /// Text from the last cut or copy. The system clipboard comes later (R14).
    pub register: String,
}

/// Rows a sample `SELECT` asks for (R9.7).
const SAMPLE_ROWS: usize = 100;

/// Whether a command tag means the schema may have changed.
fn changes_schema(tag: &str) -> bool {
    ["CREATE", "ALTER", "DROP", "SELECT INTO"]
        .iter()
        .any(|prefix| tag.starts_with(prefix))
}

/// Where a connection stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnStatus {
    Connecting,
    /// Waiting for the user to type a password.
    AwaitingPassword,
    Connected,
    Failed(String),
    /// The connection was lost after it had been established.
    Closed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub id: ConnId,
    pub profile: Profile,
    pub status: ConnStatus,
    pub info: Option<ConnectionInfo>,
    /// Updated after every run (R16.1).
    pub tx: TxState,
    /// Per-database numbers for the session pane. `None` until first read.
    pub stats: Option<SessionStats>,
    pub stats_error: Option<String>,
    /// A read is in flight.
    pub stats_busy: bool,
    pub stats_at: Option<Instant>,
    /// The user asked to cancel while a statistics read was in flight. A
    /// cancel request hits whatever the server runs at that moment, so it is
    /// sent once the read is over.
    pub cancel_after_stats: bool,
    /// Schema metadata for the find overlay. `None` until first read.
    pub catalog: Option<Catalog>,
    pub catalog_error: Option<String>,
    /// A read is in flight.
    pub catalog_busy: bool,
}

impl Connection {
    pub fn label(&self) -> &str {
        self.profile.display_name()
    }
}

/// The password prompt. The typed text is a `Secret`, so it never prints.
#[derive(Debug, Clone, PartialEq)]
pub struct PasswordPrompt {
    pub conn: ConnId,
    pub input: Secret,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Picker(Picker),
    /// The find overlay. It reuses the picker's input and selection.
    Find(Picker),
    /// Connection details typed by hand.
    Form(ConnectForm),
    /// Database and user of the current connection.
    Session(SessionForm),
    Password(PasswordPrompt),
    /// Asks before a risky statement runs (R15).
    Confirm(Confirm),
}

/// A statement waiting for the user's yes (R15.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub conn: ConnId,
    pub doc: DocId,
    pub sql: String,
    /// The connection's name, shown with the statement.
    pub label: String,
    pub production: bool,
    pub risk: sql::Assessment,
    pub input: String,
    pub error: Option<String>,
}

impl Confirm {
    /// Why the statement needs a yes, one sentence each.
    pub fn reasons(&self) -> Vec<&'static str> {
        let mut reasons = Vec::new();
        if self.production && self.risk.writes {
            reasons.push("This is a production connection and the statement changes data.");
        }
        if self.risk.no_where {
            reasons.push("UPDATE or DELETE without WHERE changes every row.");
        }
        if self.risk.destructive {
            reasons.push("DROP or TRUNCATE removes data or objects for good.");
        }
        reasons
    }
}

/// A statement in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub conn: ConnId,
    pub doc: DocId,
    pub started: Instant,
}

/// The open suggestion popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Where the word being completed starts.
    pub start: Pos,
    pub items: Vec<Candidate>,
    pub selected: usize,
}

/// One editor buffer with its view position.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub id: DocId,
    pub name: String,
    /// The connection this buffer runs against (R1.4).
    pub connection: Option<ConnId>,
    pub results: Option<Results>,
    pub buffer: Buffer,
    /// First visible line and first visible terminal column.
    pub scroll_top: usize,
    pub scroll_left: usize,
}

impl Document {
    fn new(id: DocId, name: String) -> Self {
        Self {
            id,
            name,
            connection: None,
            results: None,
            buffer: Buffer::new(),
            scroll_top: 0,
            scroll_left: 0,
        }
    }
}

/// How many closed buffers are remembered.
const CLOSED_LIMIT: usize = 10;

impl State {
    pub fn doc(&self) -> &Document {
        &self.docs[self.active]
    }

    pub fn connection(&self, id: ConnId) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// The connection of the buffer in the editor.
    pub fn active_connection(&self) -> Option<&Connection> {
        self.connection(self.doc().connection?)
    }

    #[cfg(test)]
    pub fn connection_mut_for_tests(&mut self, id: ConnId) -> &mut Connection {
        self.connection_mut(id).expect("connection")
    }

    fn connection_mut(&mut self, id: ConnId) -> Option<&mut Connection> {
        self.connections.iter_mut().find(|c| c.id == id)
    }

    pub fn editor(&self) -> &Buffer {
        &self.docs[self.active].buffer
    }

    pub fn editor_mut(&mut self) -> &mut Buffer {
        &mut self.docs[self.active].buffer
    }

    pub fn new(config: &Config) -> Self {
        Self {
            // Start on the commands pane: a single key connects (C).
            focus: if config.ui.show_context_strip {
                Pane::Context
            } else {
                Pane::Editor
            },
            show_context: config.ui.show_context_strip,
            show_results: true,
            help_open: false,
            help_scroll: 0,
            quit: false,
            notice: None,
            size: (0, 0),
            docs: vec![Document::new(1, "Buffer 1".to_string())],
            active: 0,
            closed: Vec::new(),
            buffers_created: 1,
            completion: None,
            connections: Vec::new(),
            next_conn: 1,
            profiles: Profiles::default(),
            dialog: None,
            running: None,
            register: String::new(),
        }
    }

    /// Panes in the order focus cycles through them (top to bottom).
    pub fn visible_panes(&self) -> Vec<Pane> {
        let mut panes = Vec::new();
        if self.show_context {
            panes.push(Pane::Context);
            panes.push(Pane::Session);
        }
        panes.push(Pane::Editor);
        if self.show_results {
            panes.push(Pane::Results);
        }
        panes
    }
}

/// One row of the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpLine {
    Header(String),
    Binding {
        key: String,
        description: &'static str,
        name: &'static str,
    },
    Blank,
}

pub struct App {
    pub state: State,
    pub keymap: Keymap,
    pub config: Config,
    /// Commands produced while handling the current event.
    pending: Vec<Command>,
}

impl App {
    pub fn new(config: Config, keymap: Keymap) -> Self {
        Self {
            state: State::new(&config),
            keymap,
            config,
            pending: Vec::new(),
        }
    }

    /// Handles an event and returns the work it asks for.
    pub fn update(&mut self, event: Event) -> Vec<Command> {
        match event {
            Event::Resize(w, h) => self.state.size = (w, h),
            Event::Key(key) => self.on_key(KeyChord::from_event(&key)),
            Event::Paste(text) => self.on_paste(&text),
            Event::Tick => self.on_tick(),
            Event::Background(background) => self.on_background(background),
        }
        self.scroll_to_cursor();
        std::mem::take(&mut self.pending)
    }

    /// Refreshes the session numbers now and then, when nothing else is using
    /// the connection.
    fn on_tick(&mut self) {
        if let Some(conn) = self.state.doc().connection {
            self.request_stats(conn, false);
        }
    }

    /// Asks for the session statistics. They run on the user's own connection,
    /// so never while a statement runs and never inside a transaction: a read
    /// there could start the transaction's snapshot or fail in a failed one.
    fn request_stats(&mut self, conn: ConnId, force: bool) -> bool {
        if self.state.running.is_some() {
            return false;
        }
        let interval = self.config.session.refresh_seconds;
        let Some(c) = self.state.connection_mut(conn) else {
            return false;
        };
        if c.status != ConnStatus::Connected || c.tx != TxState::Idle || c.stats_busy {
            return false;
        }
        if !force {
            let due = interval > 0
                && c.stats_at
                    .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(interval));
            if !due {
                return false;
            }
        }
        c.stats_busy = true;
        c.stats_at = Some(Instant::now());
        self.pending.push(Command::Stats { conn });
        true
    }

    fn refresh_session(&mut self) {
        let Some(conn) = self.state.doc().connection else {
            return self.notice("Not connected");
        };
        if !self.request_stats(conn, true) {
            self.notice("Cannot refresh now: a statement or transaction is using the connection");
        }
    }

    fn on_key(&mut self, chord: KeyChord) {
        if self.state.dialog.is_some() {
            self.on_dialog_key(chord);
            return;
        }
        let st = &self.state;
        // The help overlay takes every key.
        if st.help_open {
            if let Some(action) = self.keymap.lookup(Scope::Dialog, chord) {
                self.perform(action);
            }
            return;
        }
        // The suggestion popup sees keys first, then the focused pane's scope,
        // then global. A pane-scoped key never fires in another pane (R4.8).
        // With tab-indent off, Tab skips the editor scope and reaches the
        // global focus binding (R4.6).
        let popup_action = if st.completion.is_some() {
            self.keymap.lookup(Scope::Completion, chord)
        } else {
            None
        };
        let editor_ignores_tab = st.focus == Pane::Editor
            && !self.config.editor.tab_indent
            && matches!(chord.code, KeyCode::Tab | KeyCode::BackTab);
        let pane_action = if editor_ignores_tab {
            None
        } else {
            self.keymap.lookup(st.focus.scope(), chord)
        };
        let action = popup_action
            .or(pane_action)
            .or_else(|| self.keymap.lookup(Scope::Global, chord));

        match action {
            Some(action) => {
                self.perform(action);
                // Deleting text keeps suggestions in step with the word.
                let edits_text =
                    matches!(action, Action::Edit(EditCmd::Backspace | EditCmd::Delete));
                if edits_text {
                    self.refresh_completion(false);
                } else if !action.is_completion() {
                    self.state.completion = None;
                }
            }
            // R4.3: in the editor, any printable key without a binding types.
            None if st.focus == Pane::Editor && chord.is_printable() => {
                if let KeyCode::Char(c) = chord.code {
                    let opts = self.config.editor.options();
                    self.state.notice = None;
                    self.state.editor_mut().insert_char(c, &opts);
                    self.refresh_completion(false);
                }
            }
            None => {}
        }
    }

    /// Recomputes the suggestions for the word at the cursor. `force` is for
    /// the explicit trigger: it ignores the settings for automatic popups.
    fn refresh_completion(&mut self, force: bool) {
        self.state.completion = None;
        let editor = &self.config.editor;
        if !force && !editor.completion {
            return;
        }
        let Some((start, prefix)) = self.state.editor().completion_prefix() else {
            return;
        };
        if !force && prefix.chars().count() < editor.completion_min_chars {
            return;
        }
        let items = complete::complete(&[&KeywordSource], &prefix);
        if !items.is_empty() {
            self.state.completion = Some(Completion {
                start,
                items,
                selected: 0,
            });
        }
    }

    fn accept_completion(&mut self) {
        let Some(completion) = self.state.completion.take() else {
            return;
        };
        if let Some(item) = completion.items.get(completion.selected) {
            self.state
                .editor_mut()
                .complete(completion.start, &item.text);
        }
    }

    fn move_completion(&mut self, step: isize) {
        if let Some(c) = &mut self.state.completion {
            let len = c.items.len() as isize;
            c.selected = (c.selected as isize + step).rem_euclid(len) as usize;
        }
    }

    /// Bracketed paste. The text is inserted and never executed (R20.10).
    fn on_paste(&mut self, text: &str) {
        if self.state.dialog.is_some() {
            // One line only: a pasted newline must not submit anything.
            text.chars()
                .filter(|c| !c.is_control())
                .for_each(|c| self.dialog_type(c));
            return;
        }
        if self.state.help_open || self.state.focus != Pane::Editor {
            return;
        }
        let opts = self.config.editor.options();
        self.state.notice = None;
        self.state.completion = None;
        self.state.editor_mut().insert_str(text, &opts);
    }

    /// True while the editor uses Tab and Shift+Tab for indenting.
    fn editor_owns_tab(&self) -> bool {
        self.state.focus == Pane::Editor && self.config.editor.tab_indent
    }

    /// The actions the hint bar advertises, most useful first.
    pub fn hint_actions(&self) -> &'static [Action] {
        if self.state.completion.is_some() {
            return &[
                Action::CompletionAccept,
                Action::CompletionNext,
                Action::CompletionDismiss,
            ];
        }
        match self.state.focus {
            Pane::Editor => &[
                Action::QueryRun,
                Action::FocusCommands,
                Action::ConnectionOpen,
            ],
            Pane::Context => &[
                Action::ConnectionOpen,
                Action::FindOpen,
                Action::QueryRun,
                Action::FocusEditor,
                Action::BufferNew,
                Action::AppQuit,
                Action::HelpToggle,
            ],
            Pane::Session => &[
                Action::SessionEditDatabase,
                Action::SessionEditUser,
                Action::SessionRefresh,
                Action::TransactionRollback,
                Action::ConnectionOpen,
                Action::FocusCommands,
                Action::HelpToggle,
            ],
            Pane::Results => &[
                Action::ConnectionOpen,
                Action::QueryRun,
                Action::FocusEditor,
                Action::AppQuit,
                Action::HelpToggle,
            ],
        }
    }

    /// The key to show for `action` in the focused pane, for hints.
    pub fn hint_key(&self, action: Action) -> Option<KeyChord> {
        let scopes = [Scope::Completion, self.state.focus.scope(), Scope::Global];
        let owns_tab = self.editor_owns_tab();
        self.keymap.key_for(action, &scopes, |k| {
            !(owns_tab && matches!(k.code, KeyCode::Tab | KeyCode::BackTab))
        })
    }

    fn screen(&self) -> Rect {
        Rect::new(0, 0, self.state.size.0, self.state.size.1)
    }

    /// Lines in a page-up or page-down step.
    fn page_lines(&self) -> usize {
        layout::compute(self.screen(), &self.state, &self.config)
            .editor
            .map_or(10, |pane| {
                let area = layout::editor_area(pane, self.state.editor().line_count());
                usize::from(area.text.height).max(1)
            })
    }

    /// Keeps the cursor inside the editor viewport.
    fn scroll_to_cursor(&mut self) {
        let Some(pane) = layout::compute(self.screen(), &self.state, &self.config).editor else {
            return;
        };
        let doc = &mut self.state.docs[self.state.active];
        let area = layout::editor_area(pane, doc.buffer.line_count());
        let (height, width) = (usize::from(area.text.height), usize::from(area.text.width));
        if height == 0 || width == 0 {
            return;
        }
        let cursor = doc.buffer.cursor();
        if cursor.line < doc.scroll_top {
            doc.scroll_top = cursor.line;
        } else if cursor.line >= doc.scroll_top + height {
            doc.scroll_top = cursor.line + 1 - height;
        }
        doc.scroll_top = doc.scroll_top.min(doc.buffer.line_count() - 1);

        let line = doc
            .buffer
            .lines()
            .get(cursor.line)
            .map_or("", String::as_str);
        let x = display_col(line, cursor.col);
        if x < doc.scroll_left {
            doc.scroll_left = x;
        } else if x >= doc.scroll_left + width {
            doc.scroll_left = x + 1 - width;
        }
    }

    fn new_buffer(&mut self) {
        let st = &mut self.state;
        st.buffers_created += 1;
        st.docs.push(Document::new(
            st.buffers_created as DocId,
            format!("Buffer {}", st.buffers_created),
        ));
        st.active = st.docs.len() - 1;
        st.focus = Pane::Editor;
    }

    fn switch_buffer(&mut self, step: isize) {
        let st = &mut self.state;
        let len = st.docs.len() as isize;
        st.active = (st.active as isize + step).rem_euclid(len) as usize;
    }

    /// Closing keeps the buffer, so `buffer.reopen` can bring it back.
    fn close_buffer(&mut self) {
        let st = &mut self.state;
        let closed = st.docs.remove(st.active);
        let name = closed.name.clone();
        if !closed.buffer.text().is_empty() {
            st.closed.push(closed);
            if st.closed.len() > CLOSED_LIMIT {
                st.closed.remove(0);
            }
        }
        if st.docs.is_empty() {
            st.buffers_created += 1;
            st.docs.push(Document::new(
                st.buffers_created as DocId,
                format!("Buffer {}", st.buffers_created),
            ));
        }
        st.active = st.active.min(st.docs.len() - 1);
        self.state.notice = Some(match self.hint_key(Action::BufferReopen) {
            Some(key) => format!("Closed {name}. {key} reopens it"),
            None => format!("Closed {name}"),
        });
    }

    fn reopen_buffer(&mut self) {
        let st = &mut self.state;
        match st.closed.pop() {
            Some(doc) => {
                st.notice = Some(format!("Reopened {}", doc.name));
                st.docs.push(doc);
                st.active = st.docs.len() - 1;
                st.focus = Pane::Editor;
            }
            None => st.notice = Some("No closed buffer to reopen".to_string()),
        }
    }

    /// Everything the help overlay lists: the focused pane's keys, then the global keys (R5.7).
    pub fn help_lines(&self) -> Vec<HelpLine> {
        let mut lines = Vec::new();
        for scope in [self.state.focus.scope(), Scope::Global] {
            let bindings = self.keymap.bindings(scope);
            if bindings.is_empty() {
                continue;
            }
            lines.push(HelpLine::Header(format!("{} keys", scope.name())));
            for (key, action) in bindings {
                lines.push(HelpLine::Binding {
                    key: key.to_string(),
                    description: action.description(),
                    name: action.name(),
                });
            }
            lines.push(HelpLine::Blank);
        }
        lines
    }

    /// Rows of help text visible at once, and the furthest the help can scroll.
    fn help_limits(&self) -> (usize, usize) {
        let total = self.help_lines().len();
        let popup = layout::help_popup(self.screen(), total);
        let visible = usize::from(popup.height.saturating_sub(2)).max(1);
        (visible, total.saturating_sub(visible))
    }

    fn scroll_help(&mut self, up: bool, page: bool) {
        let (visible, max) = self.help_limits();
        let step = if page { visible } else { 1 };
        let scroll = &mut self.state.help_scroll;
        *scroll = if up {
            scroll.saturating_sub(step)
        } else {
            (*scroll + step).min(max)
        };
    }

    fn perform(&mut self, action: Action) {
        self.state.notice = None;
        match action {
            Action::AppQuit => self.state.quit = true,
            Action::FocusEditor => self.state.focus = Pane::Editor,
            Action::FocusContext | Action::FocusCommands => {
                self.state.show_context = true;
                self.state.focus = Pane::Context;
            }
            Action::FocusSession => {
                self.state.show_context = true;
                self.state.focus = Pane::Session;
            }
            Action::FocusResults => {
                self.state.show_results = true;
                self.state.focus = Pane::Results;
            }
            Action::FocusNext => self.cycle_focus(1),
            Action::FocusPrev => self.cycle_focus(-1),
            Action::ToggleContext => {
                self.state.show_context = !self.state.show_context;
                self.refocus_if_hidden();
            }
            Action::ToggleResults => {
                self.state.show_results = !self.state.show_results;
                self.refocus_if_hidden();
            }
            Action::HelpToggle => {
                self.state.help_open = !self.state.help_open;
                self.state.help_scroll = 0;
            }
            Action::HelpScrollUp => self.scroll_help(true, false),
            Action::HelpScrollDown => self.scroll_help(false, false),
            Action::HelpPageUp => self.scroll_help(true, true),
            Action::HelpPageDown => self.scroll_help(false, true),
            Action::TransactionRollback => self.rollback(),
            Action::SessionRefresh => self.refresh_session(),
            Action::SessionEditDatabase => self.edit_session(0),
            Action::SessionEditUser => self.edit_session(1),
            Action::ConnectionOpen => self.open_picker(),
            Action::ConnectionReconnect => self.reconnect(),
            Action::QueryRun => self.run_statement(),
            Action::QueryCancel => self.cancel_query(),
            Action::ResultsUp => self.scroll_results(-1, false),
            Action::ResultsDown => self.scroll_results(1, false),
            Action::ResultsPageUp => self.scroll_results(-1, true),
            Action::ResultsPageDown => self.scroll_results(1, true),
            Action::ResultsFirstRow => {
                if let Some(r) = self.results_mut() {
                    r.first_row();
                }
            }
            Action::ResultsLastRow => {
                let page = self.results_page_rows();
                if let Some(r) = self.results_mut() {
                    r.last_row(page);
                }
            }
            Action::ResultsLeft => {
                if let Some(r) = self.results_mut() {
                    r.scroll_columns(-1);
                }
            }
            Action::ResultsRight => {
                if let Some(r) = self.results_mut() {
                    r.scroll_columns(1);
                }
            }
            Action::ResultsPrevTab => {
                if let Some(r) = self.results_mut() {
                    r.select_tab(-1);
                }
            }
            Action::ResultsNextTab => {
                if let Some(r) = self.results_mut() {
                    r.select_tab(1);
                }
            }
            // These act only inside dialogs, which handle them themselves.
            Action::DialogNext
            | Action::DialogPrev
            | Action::DialogSubmit
            | Action::DialogCancel
            | Action::DialogBackspace
            | Action::DialogClear
            | Action::PickerSaveProfile
            | Action::PickerEditProfile
            | Action::FindSample => {}
            Action::FindOpen => self.open_find(),
            Action::CompletionTrigger => self.refresh_completion(true),
            Action::CompletionAccept => self.accept_completion(),
            Action::CompletionDismiss => self.state.completion = None,
            Action::CompletionNext => self.move_completion(1),
            Action::CompletionPrev => self.move_completion(-1),
            Action::BufferNew => self.new_buffer(),
            Action::BufferClose => self.close_buffer(),
            Action::BufferNext => self.switch_buffer(1),
            Action::BufferPrev => self.switch_buffer(-1),
            Action::BufferReopen => self.reopen_buffer(),
            Action::Edit(cmd) => {
                let opts = self.config.editor.options();
                let page = self.page_lines();
                let st = &mut self.state;
                st.docs[st.active]
                    .buffer
                    .apply(cmd, &opts, page, &mut st.register);
            }
        }
    }

    // -- connections ----------------------------------------------------------

    /// The label of this buffer's connection when it can be reconnected.
    fn reconnect_label(&self) -> Option<String> {
        let conn = self.state.connection(self.state.doc().connection?)?;
        matches!(conn.status, ConnStatus::Failed(_) | ConnStatus::Closed(_))
            .then(|| conn.label().to_string())
    }

    fn open_picker(&mut self) {
        self.state.help_open = false;
        self.state.completion = None;
        self.state.dialog = Some(Dialog::Picker(Picker::default()));
    }

    /// The catalog of the connection the active buffer uses.
    pub fn active_catalog(&self) -> Option<&Catalog> {
        self.state
            .connection(self.state.doc().connection?)?
            .catalog
            .as_ref()
    }

    /// What the find overlay lists for `input`.
    fn find_results<'a>(
        connections: &'a [Connection],
        conn: Option<ConnId>,
        input: &str,
    ) -> Vec<&'a CatalogObject> {
        conn.and_then(|id| connections.iter().find(|c| c.id == id))
            .and_then(|c| c.catalog.as_ref())
            .map_or_else(Vec::new, |catalog| catalog.search(input))
    }

    /// The objects the overlay lists now, for drawing.
    pub fn find_items(&self, input: &str) -> Vec<&CatalogObject> {
        Self::find_results(&self.state.connections, self.state.doc().connection, input)
    }

    /// Asks for the schema metadata. Same conditions as the statistics read:
    /// it must not start or break a transaction.
    fn request_catalog(&mut self, conn: ConnId) -> bool {
        if self.state.running.is_some() {
            return false;
        }
        let Some(c) = self.state.connection_mut(conn) else {
            return false;
        };
        if c.status != ConnStatus::Connected || c.tx != TxState::Idle || c.catalog_busy {
            return false;
        }
        c.catalog_busy = true;
        self.pending.push(Command::Catalog { conn });
        true
    }

    fn open_find(&mut self) {
        let connected = self
            .state
            .doc()
            .connection
            .and_then(|id| self.state.connection(id))
            .filter(|c| c.status == ConnStatus::Connected)
            .map(|c| (c.id, c.catalog.is_none()));
        let Some((conn, missing)) = connected else {
            let key = self.hint_key(Action::ConnectionOpen).map(|k| k.to_string());
            return self.notice(match key {
                Some(key) => format!("Not connected. Press {key} to connect"),
                None => "Not connected".to_string(),
            });
        };
        if missing {
            self.request_catalog(conn);
        }
        self.state.help_open = false;
        self.state.completion = None;
        self.state.dialog = Some(Dialog::Find(Picker::default()));
    }

    /// Enter puts the object's name in the editor. With `sample`, a table or
    /// view gets a `SELECT` that runs at once (R9.7).
    fn submit_find(&mut self, sample: bool) {
        let Some(Dialog::Find(find)) = &self.state.dialog else {
            return;
        };
        let items = self.find_items(&find.input);
        let Some(object) = items.get(find.selected).copied() else {
            return;
        };
        let name = object.sql_name();
        let relation = object.kind.is_relation();
        if sample && !relation {
            return self.notice("Sample rows are available for tables and views");
        }
        self.state.dialog = None;
        self.state.focus = Pane::Editor;
        let opts = self.config.editor.options();
        if sample {
            let statement = format!("SELECT * FROM {name} LIMIT {SAMPLE_ROWS};");
            let editor = self.state.editor_mut();
            editor.move_cursor(Motion::BufferEnd, false, 1);
            if !editor.text().trim().is_empty() {
                editor.insert_str("\n\n", &opts);
            }
            editor.insert_str(&statement, &opts);
            self.run_statement();
        } else {
            self.state.editor_mut().insert_str(&name, &opts);
        }
    }

    fn notice(&mut self, text: impl Into<String>) {
        self.state.notice = Some(text.into());
    }

    /// Starts connecting the buffer in the editor to `profile`.
    fn connect(&mut self, profile: Profile, password: Option<Secret>) {
        let st = &mut self.state;
        let conn = st.next_conn;
        st.next_conn += 1;
        let doc = st.active;
        if let Some(old) = st.docs[doc].connection.replace(conn) {
            let in_use = st.docs.iter().any(|d| d.connection == Some(old));
            if !in_use {
                st.connections.retain(|c| c.id != old);
                self.pending.push(Command::Disconnect { conn: old });
            }
        }
        let label = profile.display_name().to_string();
        let lookup = password.is_none();
        self.state.connections.push(Connection {
            id: conn,
            profile: profile.clone(),
            status: ConnStatus::Connecting,
            info: None,
            tx: TxState::Idle,
            stats: None,
            stats_error: None,
            stats_busy: false,
            stats_at: None,
            cancel_after_stats: false,
            catalog: None,
            catalog_error: None,
            catalog_busy: false,
        });
        self.state.dialog = None;
        self.notice(format!("Connecting to {label}…"));
        self.pending.push(Command::Connect {
            conn,
            profile: Box::new(profile),
            password,
            lookup,
        });
    }

    fn connect_to_saved(&mut self, name: &str) {
        let Some(profile) = self.state.profiles.get(name).cloned() else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        self.state.profiles.touch(name, now);
        self.pending.push(Command::SaveProfiles {
            contents: self.state.profiles.to_toml(),
        });
        self.connect(profile, None);
    }

    fn reconnect(&mut self) {
        let Some(id) = self.state.doc().connection else {
            return self.notice("This buffer has no connection. Open one first");
        };
        let Some(conn) = self.state.connection_mut(id) else {
            return;
        };
        if matches!(conn.status, ConnStatus::Connected | ConnStatus::Connecting) {
            return;
        }
        conn.status = ConnStatus::Connecting;
        let profile = conn.profile.clone();
        self.notice(format!("Reconnecting to {}…", profile.display_name()));
        self.pending.push(Command::Connect {
            conn: id,
            profile: Box::new(profile),
            password: None,
            lookup: true,
        });
    }

    fn on_dialog_key(&mut self, chord: KeyChord) {
        let scope = match self.state.dialog {
            Some(Dialog::Picker(_)) => Scope::Picker,
            Some(Dialog::Find(_)) => Scope::Find,
            Some(Dialog::Form(_) | Dialog::Session(_) | Dialog::Confirm(_)) => Scope::Form,
            Some(Dialog::Password(_)) => Scope::Prompt,
            None => return,
        };
        match self.keymap.lookup(scope, chord) {
            Some(action) => self.dialog_action(action),
            None if chord.is_printable() => {
                if let KeyCode::Char(c) = chord.code {
                    self.dialog_type(c);
                }
            }
            None => {}
        }
    }

    fn dialog_type(&mut self, c: char) {
        match &mut self.state.dialog {
            Some(Dialog::Picker(p) | Dialog::Find(p)) => p.push(c),
            Some(Dialog::Form(f)) => f.type_char(c),
            Some(Dialog::Session(f)) => f.type_char(c),
            Some(Dialog::Confirm(c0)) => {
                c0.input.push(c);
                c0.error = None;
            }
            Some(Dialog::Password(p)) => {
                p.input = Secret::new(format!("{}{c}", p.input.expose()));
                p.error = None;
            }
            None => {}
        }
    }

    fn dialog_action(&mut self, action: Action) {
        let reconnect = self.reconnect_label();
        let profiles = &self.state.profiles;
        match (action, &mut self.state.dialog) {
            (Action::DialogNext, Some(Dialog::Picker(p))) => {
                let count = p.items(profiles, reconnect.as_deref()).len();
                p.select_next(count);
            }
            (Action::DialogPrev, Some(Dialog::Picker(p))) => {
                let count = p.items(profiles, reconnect.as_deref()).len();
                p.select_previous(count);
            }
            (Action::DialogNext, Some(Dialog::Find(p))) => {
                let count = Self::find_results(
                    &self.state.connections,
                    self.state.docs[self.state.active].connection,
                    &p.input,
                )
                .len();
                p.select_next(count);
            }
            (Action::DialogPrev, Some(Dialog::Find(p))) => {
                let count = Self::find_results(
                    &self.state.connections,
                    self.state.docs[self.state.active].connection,
                    &p.input,
                )
                .len();
                p.select_previous(count);
            }
            (Action::DialogNext, Some(Dialog::Form(f))) => f.next_field(),
            (Action::DialogPrev, Some(Dialog::Form(f))) => f.previous_field(),
            (Action::DialogNext | Action::DialogPrev, Some(Dialog::Session(f))) => f.next_field(),
            (Action::DialogBackspace, Some(Dialog::Picker(p) | Dialog::Find(p))) => p.backspace(),
            (Action::DialogBackspace, Some(Dialog::Form(f))) => f.backspace(),
            (Action::DialogBackspace, Some(Dialog::Session(f))) => f.backspace(),
            (Action::DialogBackspace, Some(Dialog::Confirm(c))) => {
                c.input.pop();
                c.error = None;
            }
            (Action::DialogBackspace, Some(Dialog::Password(p))) => {
                let mut text = p.input.expose().to_string();
                text.pop();
                p.input = Secret::new(text);
            }
            (Action::DialogClear, Some(Dialog::Picker(p) | Dialog::Find(p))) => p.clear(),
            (Action::DialogClear, Some(Dialog::Form(f))) => f.clear(),
            (Action::DialogClear, Some(Dialog::Session(f))) => f.clear(),
            (Action::DialogClear, Some(Dialog::Confirm(c))) => c.input.clear(),
            (Action::DialogClear, Some(Dialog::Password(p))) => p.input = Secret::new(""),
            (Action::DialogCancel, Some(Dialog::Picker(_) | Dialog::Find(_))) => {
                self.state.dialog = None;
            }
            // Esc in the form goes back to the list.
            (Action::DialogCancel, Some(Dialog::Form(_))) => {
                self.state.dialog = Some(Dialog::Picker(Picker::default()));
            }
            (Action::DialogCancel, Some(Dialog::Session(_))) => self.state.dialog = None,
            (Action::DialogCancel, Some(Dialog::Confirm(_))) => {
                self.state.dialog = None;
                self.notice("Cancelled. Nothing was run");
            }
            (Action::DialogCancel, Some(Dialog::Password(p))) => {
                let conn = p.conn;
                self.state.dialog = None;
                self.abandon_connection(conn);
                self.notice("Connection cancelled");
            }
            (Action::DialogSubmit, Some(Dialog::Picker(_))) => self.submit_picker(),
            (Action::DialogSubmit, Some(Dialog::Find(_))) => self.submit_find(false),
            (Action::FindSample, Some(Dialog::Find(_))) => self.submit_find(true),
            (Action::DialogSubmit, Some(Dialog::Form(_))) => self.submit_form(),
            (Action::DialogSubmit, Some(Dialog::Session(_))) => self.submit_session(),
            (Action::DialogSubmit, Some(Dialog::Confirm(_))) => self.submit_confirm(),
            (Action::DialogSubmit, Some(Dialog::Password(_))) => self.submit_password(),
            (Action::PickerSaveProfile, Some(Dialog::Picker(_))) => self.save_uri_as_profile(),
            (Action::PickerEditProfile, Some(Dialog::Picker(_))) => self.edit_selected_profile(),
            _ => {}
        }
    }

    fn submit_form(&mut self) {
        let Some(Dialog::Form(form)) = &self.state.dialog else {
            return;
        };
        let form = form.clone();
        let profile = match form.build() {
            Ok(profile) => profile,
            Err(message) => return self.set_form_error(message),
        };
        if let Some(original) = &form.editing {
            if let Err(message) = self.state.profiles.update(&original.name, profile) {
                return self.set_form_error(message);
            }
            self.pending.push(Command::SaveProfiles {
                contents: self.state.profiles.to_toml(),
            });
            let name = form.name.trim().to_string();
            self.state.dialog = Some(Dialog::Picker(Picker::default()));
            return self.notice(format!("Saved changes to `{name}`"));
        }
        if !form.name.trim().is_empty() {
            if let Err(message) = self.state.profiles.add(profile.clone()) {
                return self.set_form_error(message);
            }
            self.pending.push(Command::SaveProfiles {
                contents: self.state.profiles.to_toml(),
            });
        }
        self.connect(profile, None);
    }

    /// Opens the form for the saved profile selected in the picker.
    fn edit_selected_profile(&mut self) {
        let reconnect = self.reconnect_label();
        let Some(Dialog::Picker(picker)) = &self.state.dialog else {
            return;
        };
        match picker.current(&self.state.profiles, reconnect.as_deref()) {
            Some(picker::Item::Profile(name)) => {
                if let Some(profile) = self.state.profiles.get(&name) {
                    self.state.dialog = Some(Dialog::Form(ConnectForm::edit(profile)));
                }
            }
            _ => self.notice("Select a saved connection to edit it"),
        }
    }

    /// Opens the session form on the database (0) or the user (1).
    fn edit_session(&mut self, focus: usize) {
        let Some(conn) = self.state.active_connection() else {
            return self.notice("Not connected. Connect first, then change the session");
        };
        if matches!(
            conn.status,
            ConnStatus::Connecting | ConnStatus::AwaitingPassword
        ) {
            return self.notice("Wait for the connection to finish first");
        }
        // What the server reports is more exact than what the profile says.
        let (database, user) = match &conn.info {
            Some(info) => (info.database.clone(), info.user.clone()),
            None => (
                conn.profile.database.clone().unwrap_or_default(),
                conn.profile
                    .user
                    .clone()
                    .unwrap_or_else(crate::runtime::default_user),
            ),
        };
        self.state.dialog = Some(Dialog::Session(SessionForm::new(&database, &user, focus)));
    }

    fn submit_session(&mut self) {
        let Some(Dialog::Session(form)) = &self.state.dialog else {
            return;
        };
        let Some(current) = self.state.active_connection() else {
            return;
        };
        match form.apply(&current.profile) {
            // The password is looked up or asked for again: it can differ per user.
            Ok(profile) => self.connect(profile, None),
            Err(message) => {
                if let Some(Dialog::Session(form)) = &mut self.state.dialog {
                    form.error = Some(message);
                }
            }
        }
    }

    fn set_form_error(&mut self, message: String) {
        if let Some(Dialog::Form(form)) = &mut self.state.dialog {
            form.error = Some(message);
        }
    }

    fn submit_picker(&mut self) {
        let reconnect = self.reconnect_label();
        let Some(Dialog::Picker(picker)) = &self.state.dialog else {
            return;
        };
        match picker.current(&self.state.profiles, reconnect.as_deref()) {
            Some(picker::Item::Uri(text)) => match profile::parse_uri(&text) {
                Ok(uri) => self.connect(uri.profile, uri.password),
                Err(message) => self.notice(format!("Invalid connection URI: {message}")),
            },
            Some(picker::Item::Profile(name)) => self.connect_to_saved(&name),
            Some(picker::Item::Reconnect(_)) => {
                self.state.dialog = None;
                self.reconnect();
            }
            Some(picker::Item::New) => {
                let user = crate::runtime::default_user();
                self.state.dialog = Some(Dialog::Form(ConnectForm::new(&user)));
            }
            None => {}
        }
    }

    /// Saves the typed URI as a profile. The password is not saved (R2.3).
    fn save_uri_as_profile(&mut self) {
        let Some(Dialog::Picker(picker)) = &self.state.dialog else {
            return;
        };
        if !picker::looks_like_uri(&picker.input) {
            return self.notice("Type a postgres:// URI first, then press the save key");
        }
        let uri = match profile::parse_uri(&picker.input) {
            Ok(uri) => uri,
            Err(message) => return self.notice(format!("Invalid connection URI: {message}")),
        };
        let mut saved = uri.profile;
        let base = format!(
            "{}{}",
            saved.host,
            saved
                .database
                .as_deref()
                .map(|d| format!("/{d}"))
                .unwrap_or_default()
        );
        saved.name = base.clone();
        let mut n = 2;
        while self.state.profiles.get(&saved.name).is_some() {
            saved.name = format!("{base} ({n})");
            n += 1;
        }
        let name = saved.name.clone();
        match self.state.profiles.add(saved) {
            Ok(()) => {
                self.pending.push(Command::SaveProfiles {
                    contents: self.state.profiles.to_toml(),
                });
                let note = if uri.password.is_some() {
                    " The password was not saved"
                } else {
                    ""
                };
                self.notice(format!("Saved profile `{name}`.{note}"));
            }
            Err(message) => self.notice(format!("Could not save the profile: {message}")),
        }
    }

    fn submit_password(&mut self) {
        let Some(Dialog::Password(prompt)) = self.state.dialog.take() else {
            return;
        };
        let Some(conn) = self.state.connection_mut(prompt.conn) else {
            return;
        };
        conn.status = ConnStatus::Connecting;
        let profile = conn.profile.clone();
        // An empty password means: try without one (for example `trust` logins).
        let password = (!prompt.input.is_empty()).then_some(prompt.input);
        self.pending.push(Command::Connect {
            conn: prompt.conn,
            profile: Box::new(profile),
            password,
            lookup: false,
        });
    }

    /// Forgets a connection that never came up.
    fn abandon_connection(&mut self, conn: ConnId) {
        self.state.connections.retain(|c| c.id != conn);
        for doc in &mut self.state.docs {
            if doc.connection == Some(conn) {
                doc.connection = None;
            }
        }
        self.pending.push(Command::Disconnect { conn });
    }

    fn prompting_allowed(&self) -> bool {
        self.config
            .connections
            .password_sources
            .contains(&Source::Prompt)
    }

    fn on_background(&mut self, background: Background) {
        match background {
            Background::PasswordNeeded { conn, notes } => {
                let Some(c) = self.state.connection_mut(conn) else {
                    return;
                };
                c.status = ConnStatus::AwaitingPassword;
                if let Some(first) = notes.first() {
                    self.notice(first.clone());
                }
                self.state.dialog = Some(Dialog::Password(PasswordPrompt {
                    conn,
                    input: Secret::new(""),
                    error: None,
                }));
            }
            Background::Connected { conn, info } => {
                let Some(c) = self.state.connection_mut(conn) else {
                    return;
                };
                c.status = ConnStatus::Connected;
                let text = format!(
                    "Connected to {} (PostgreSQL {})",
                    c.label(),
                    info.server_version
                );
                c.info = Some(info);
                self.notice(text);
                self.request_stats(conn, true);
                self.request_catalog(conn);
                // Ready to write SQL: move from the commands pane to the editor.
                if self.state.doc().connection == Some(conn) && self.state.focus == Pane::Context {
                    self.state.focus = Pane::Editor;
                }
            }
            Background::ConnectFailed { conn, error } => {
                if self.state.connection(conn).is_none() {
                    return;
                }
                if error.is_auth_failure()
                    && self.prompting_allowed()
                    && self.state.dialog.is_none()
                {
                    if let Some(c) = self.state.connection_mut(conn) {
                        c.status = ConnStatus::AwaitingPassword;
                    }
                    self.state.dialog = Some(Dialog::Password(PasswordPrompt {
                        conn,
                        input: Secret::new(""),
                        error: Some(error.message),
                    }));
                    return;
                }
                if let Some(c) = self.state.connection_mut(conn) {
                    c.status = ConnStatus::Failed(error.message.clone());
                }
                self.notice(format!("Could not connect: {}", error.message));
            }
            Background::ConnectionClosed { conn, reason } => {
                let Some(c) = self.state.connection_mut(conn) else {
                    return;
                };
                if c.status != ConnStatus::Connected {
                    return;
                }
                c.status = ConnStatus::Closed(reason.clone());
                if self.state.running.as_ref().is_some_and(|r| r.conn == conn) {
                    self.state.running = None;
                }
                let hint = self
                    .hint_key(Action::ConnectionReconnect)
                    .map(|k| format!("Press {k} to reconnect. "))
                    .unwrap_or_default();
                self.notice(format!("{hint}Connection lost: {reason}"));
            }
            Background::QueryFinished { conn, doc, output } => {
                if self.state.running.as_ref().is_some_and(|r| r.conn == conn) {
                    self.state.running = None;
                }
                let cancelled = output.error.as_ref().is_some_and(|e| e.is_cancellation());
                let tx = output.tx;
                if let Some(c) = self.state.connection_mut(conn) {
                    c.tx = tx;
                }
                // A statement may have changed the numbers.
                self.request_stats(conn, true);
                // Schema changes (R-SCH-007): read the catalog again.
                if output.sets.iter().any(|s| changes_schema(&s.tag)) {
                    self.request_catalog(conn);
                }
                if tx == TxState::Failed {
                    let key = self
                        .keymap
                        .key_for(Action::TransactionRollback, &[Scope::Global], |_| true)
                        .map(|k| format!(" Press {k} to roll back"))
                        .unwrap_or_default();
                    self.notice(format!("The transaction failed.{key}"));
                }
                if let Some(d) = self.state.docs.iter_mut().find(|d| d.id == doc) {
                    d.results = Some(Results::new(output));
                    self.state.show_results = true;
                }
                if cancelled {
                    self.notice("Query cancelled");
                }
            }
            Background::Catalog { conn, result } => {
                if let Some(c) = self.state.connection_mut(conn) {
                    c.catalog_busy = false;
                    match result {
                        Ok(catalog) => {
                            c.catalog = Some(catalog);
                            c.catalog_error = None;
                        }
                        Err(e) if e.is_cancellation() => {}
                        Err(e) => c.catalog_error = Some(e.message),
                    }
                    // A cancel hits whatever the server runs, so wait until
                    // both reads are over.
                    if !c.stats_busy && !c.catalog_busy && std::mem::take(&mut c.cancel_after_stats)
                    {
                        self.pending.push(Command::Cancel { conn });
                    }
                }
            }
            Background::Stats { conn, result } => {
                if let Some(c) = self.state.connection_mut(conn) {
                    c.stats_busy = false;
                    match result {
                        Ok(stats) => {
                            c.stats = Some(stats);
                            c.stats_error = None;
                        }
                        // Someone's cancel hit the read. Not worth reporting.
                        Err(e) if e.is_cancellation() => {}
                        Err(e) => c.stats_error = Some(e.message),
                    }
                    // A cancel hits whatever the server runs, so wait until
                    // both reads are over.
                    if !c.stats_busy && !c.catalog_busy && std::mem::take(&mut c.cancel_after_stats)
                    {
                        self.pending.push(Command::Cancel { conn });
                    }
                }
            }
            Background::ProfilesSaved(Err(message)) => {
                self.notice(format!("Could not save profiles: {message}"));
            }
            Background::ProfilesSaved(Ok(())) => {}
        }
    }

    // -- running queries ------------------------------------------------------

    fn run_statement(&mut self) {
        let doc = self.state.doc();
        let (doc_id, conn_id) = (doc.id, doc.connection);
        let connected = conn_id
            .and_then(|id| self.state.connection(id))
            .filter(|c| c.status == ConnStatus::Connected)
            .map(|c| c.id);
        let Some(conn) = connected else {
            let key = self.hint_key(Action::ConnectionOpen).map(|k| k.to_string());
            return self.notice(match (conn_id, key) {
                (None, Some(key)) => format!("Not connected. Press {key} to connect"),
                (None, None) => "Not connected".to_string(),
                (Some(_), _) => "The connection is not ready".to_string(),
            });
        };
        if self.state.running.is_some() {
            let key = self
                .hint_key(Action::QueryCancel)
                .map(|k| format!(" {k} cancels it."))
                .unwrap_or_default();
            return self.notice(format!("A statement is already running.{key}"));
        }

        let editor = self.state.editor();
        let text = match editor.selected_text() {
            Some(selected) => selected,
            None => {
                let all = editor.text();
                match sql::statement_at(&all, editor.cursor_offset()) {
                    Some(statement) => statement.text(&all).to_string(),
                    None => String::new(),
                }
            }
        };
        if text.trim().is_empty() {
            return self.notice("Nothing to run. Put the cursor in a statement");
        }
        let risk = sql::assess(&text);
        let profile = &self.state.connection(conn).map(|c| c.profile.clone());
        let production = profile.as_ref().is_some_and(|p| p.production);
        if (production && risk.writes) || risk.no_where || risk.destructive {
            let label = profile
                .as_ref()
                .map_or_else(String::new, |p| p.shown_name().to_string());
            self.state.completion = None;
            self.state.dialog = Some(Dialog::Confirm(Confirm {
                conn,
                doc: doc_id,
                sql: text,
                label,
                production,
                risk,
                input: String::new(),
                error: None,
            }));
            return;
        }
        self.start_run(conn, doc_id, text);
    }

    fn start_run(&mut self, conn: ConnId, doc: DocId, sql: String) {
        self.state.running = Some(Running {
            conn,
            doc,
            started: Instant::now(),
        });
        self.state.completion = None;
        self.pending.push(Command::Run { conn, doc, sql });
    }

    /// Only `y` or `yes` confirms. Enter alone never does (R15.5).
    fn submit_confirm(&mut self) {
        let Some(Dialog::Confirm(confirm)) = &mut self.state.dialog else {
            return;
        };
        let answer = confirm.input.trim().to_ascii_lowercase();
        if answer != "y" && answer != "yes" {
            confirm.error = Some("Type y or yes to run it, or press Esc to cancel".to_string());
            return;
        }
        let (conn, doc, text) = (confirm.conn, confirm.doc, confirm.sql.clone());
        self.state.dialog = None;
        if self.state.running.is_some() {
            return self.notice("A statement is already running");
        }
        self.start_run(conn, doc, text);
    }

    /// Ends the open or failed transaction of this buffer's connection (R16.3).
    fn rollback(&mut self) {
        let doc = self.state.doc();
        let doc_id = doc.id;
        let Some(conn) = doc
            .connection
            .and_then(|id| self.state.connection(id))
            .filter(|c| c.status == ConnStatus::Connected)
        else {
            return self.notice("Not connected");
        };
        if conn.tx == TxState::Idle {
            return self.notice("No transaction is open");
        }
        let id = conn.id;
        if self.state.running.is_some() {
            return self.notice("A statement is already running");
        }
        self.start_run(id, doc_id, "ROLLBACK".to_string());
    }

    fn cancel_query(&mut self) {
        if let Some(running) = &self.state.running {
            let conn = running.conn;
            match self.state.connection_mut(conn) {
                // The server would cancel the statistics read, not the statement.
                Some(c) if c.stats_busy || c.catalog_busy => c.cancel_after_stats = true,
                _ => self.pending.push(Command::Cancel { conn }),
            }
            self.notice("Cancelling…");
        }
    }

    // -- results --------------------------------------------------------------

    fn results_mut(&mut self) -> Option<&mut Results> {
        let active = self.state.active;
        self.state.docs[active].results.as_mut()
    }

    /// Rows of the grid visible at once.
    pub fn results_page_rows(&self) -> usize {
        let areas = layout::compute(self.screen(), &self.state, &self.config);
        let tabs = self
            .state
            .doc()
            .results
            .as_ref()
            .map_or(0, Results::tab_count);
        areas
            .results
            .map_or(10, |rect| {
                usize::from(layout::results_body(rect, tabs > 1).rows.height)
            })
            .max(1)
    }

    fn scroll_results(&mut self, direction: isize, page: bool) {
        let rows = self.results_page_rows();
        let step = if page { rows as isize } else { 1 };
        if let Some(results) = self.results_mut() {
            results.scroll_rows(direction * step, rows);
        }
    }

    fn cycle_focus(&mut self, step: isize) {
        let panes = self.state.visible_panes();
        let current = panes
            .iter()
            .position(|p| *p == self.state.focus)
            .unwrap_or(0);
        let len = panes.len() as isize;
        let next = (current as isize + step).rem_euclid(len) as usize;
        self.state.focus = panes[next];
    }

    /// Hiding the focused pane sends focus back to the editor.
    fn refocus_if_hidden(&mut self) {
        if !self.state.visible_panes().contains(&self.state.focus) {
            self.state.focus = Pane::Editor;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// An app already focused on the editor, so most tests can type at once.
    /// The real starting state is covered by the landing tests.
    fn app_with(config: Config) -> App {
        let mut app = App::new(config, Keymap::default_keymap());
        app.state.focus = Pane::Editor;
        app
    }

    fn app() -> App {
        app_with(Config::default())
    }

    fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        app.update(Event::Key(KeyEvent::new(code, mods)));
    }

    fn alt(app: &mut App, c: char) {
        press(app, KeyCode::Char(c), KeyModifiers::ALT);
    }

    #[test]
    fn starts_with_editor_focused() {
        assert_eq!(app().state.focus, Pane::Editor);
    }

    #[test]
    fn focus_keys_move_focus() {
        let mut a = app();
        alt(&mut a, '1');
        assert_eq!(a.state.focus, Pane::Context);
        alt(&mut a, '3');
        assert_eq!(a.state.focus, Pane::Results);
        alt(&mut a, '2');
        assert_eq!(a.state.focus, Pane::Editor);
        press(&mut a, KeyCode::Char('1'), KeyModifiers::CONTROL);
        assert_eq!(a.state.focus, Pane::Context);
    }

    #[test]
    fn tab_indents_in_editor_when_tab_indent_is_on() {
        let mut a = app();
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Editor);
        assert_eq!(a.state.editor().text(), "    ");
        press(&mut a, KeyCode::F(6), KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Results, "F6 always leaves the editor");
    }

    #[test]
    fn tab_cycles_focus_when_tab_indent_is_off() {
        let mut config = Config::default();
        config.editor.tab_indent = false;
        let mut a = app_with(config);
        // Panes run left to right, then down: Context, Session, Editor, Results.
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Results);
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Context);
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Session);
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Editor);
        press(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(a.state.focus, Pane::Session);
    }

    #[test]
    fn tab_cycles_from_other_panes_even_with_tab_indent() {
        let mut a = app();
        alt(&mut a, '1');
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Session);
        alt(&mut a, '4');
        assert_eq!(a.state.focus, Pane::Session);
    }

    #[test]
    fn cycling_skips_hidden_panes() {
        let mut a = app();
        press(&mut a, KeyCode::F(3), KeyModifiers::NONE); // hide context
        press(&mut a, KeyCode::F(6), KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Results);
        press(&mut a, KeyCode::F(6), KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Editor);
    }

    #[test]
    fn hiding_the_focused_pane_returns_focus_to_editor() {
        let mut a = app();
        alt(&mut a, '3');
        press(&mut a, KeyCode::F(4), KeyModifiers::NONE);
        assert!(!a.state.show_results);
        assert_eq!(a.state.focus, Pane::Editor);
    }

    #[test]
    fn focusing_a_hidden_pane_shows_it() {
        let mut a = app();
        press(&mut a, KeyCode::F(4), KeyModifiers::NONE);
        alt(&mut a, '3');
        assert!(a.state.show_results);
        assert_eq!(a.state.focus, Pane::Results);
    }

    #[test]
    fn pane_scoped_key_only_works_in_its_pane() {
        let mut a = app();
        // `?` opens help in the results pane (R4.8) but not in the editor.
        press(&mut a, KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(!a.state.help_open);
        alt(&mut a, '3');
        press(&mut a, KeyCode::Char('?'), KeyModifiers::NONE);
        assert!(a.state.help_open);
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    #[test]
    fn printable_keys_type_in_the_editor_including_ones_bound_elsewhere() {
        let mut a = app();
        // `?` opens help in other panes. In the editor it is just a character (R4.3, R4.4).
        type_text(&mut a, "select ?, q j");
        assert_eq!(a.state.editor().text(), "select ?, q j");
        assert!(!a.state.help_open);
    }

    #[test]
    fn shifted_letters_type_as_capitals() {
        let mut a = app();
        press(&mut a, KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert_eq!(a.state.editor().text(), "A");
    }

    #[test]
    fn unbound_alt_and_ctrl_keys_do_not_type() {
        let mut a = app();
        press(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
        press(&mut a, KeyCode::Char('k'), KeyModifiers::ALT);
        assert_eq!(a.state.editor().text(), "");
    }

    #[test]
    fn printable_keys_do_nothing_outside_the_editor() {
        let mut a = app();
        alt(&mut a, '3');
        let before = a.state.clone();
        // Keys with no meaning in the results pane. (c, r, e, q and ? do have one.)
        for c in ['j', 'x', 'z', ' '] {
            press(&mut a, KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert_eq!(a.state, before);
        assert_eq!(a.state.editor().text(), "");
    }

    #[test]
    fn editor_keys_do_nothing_outside_the_editor() {
        let mut a = app();
        type_text(&mut a, "abc");
        alt(&mut a, '3');
        for code in [
            KeyCode::Left,
            KeyCode::Backspace,
            KeyCode::Enter,
            KeyCode::Delete,
        ] {
            press(&mut a, code, KeyModifiers::NONE);
        }
        assert_eq!(a.state.editor().text(), "abc");
        assert_eq!(a.state.editor().cursor().col, 3);
    }

    #[test]
    fn editing_keys_work_through_the_keymap() {
        let mut a = app();
        type_text(&mut a, "ab");
        press(&mut a, KeyCode::Left, KeyModifiers::NONE);
        press(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(a.state.editor().text(), "b");
        press(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(a.state.editor().text(), "\nb");
        press(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(a.state.editor().text(), "b");
        press(&mut a, KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(a.state.editor().text(), "\nb");
    }

    #[test]
    fn select_cut_and_paste_use_the_register() {
        let mut a = app();
        type_text(&mut a, "hello");
        press(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
        press(&mut a, KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(a.state.editor().text(), "");
        assert_eq!(a.state.register, "hello");
        press(&mut a, KeyCode::Char('v'), KeyModifiers::CONTROL);
        press(&mut a, KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert_eq!(a.state.editor().text(), "hellohello");
    }

    #[test]
    fn shift_arrows_select() {
        let mut a = app();
        type_text(&mut a, "abc");
        press(&mut a, KeyCode::Left, KeyModifiers::SHIFT);
        press(&mut a, KeyCode::Left, KeyModifiers::SHIFT);
        assert_eq!(a.state.editor().selected_text().as_deref(), Some("bc"));
    }

    #[test]
    fn bracketed_paste_inserts_text_in_the_editor_only() {
        let mut a = app();
        a.update(Event::Paste("select 1;\nselect 2;".to_string()));
        assert_eq!(a.state.editor().text(), "select 1;\nselect 2;");
        assert!(!a.state.quit);

        alt(&mut a, '3');
        a.update(Event::Paste("ignored".to_string()));
        assert_eq!(a.state.editor().text(), "select 1;\nselect 2;");

        alt(&mut a, '2');
        press(&mut a, KeyCode::F(1), KeyModifiers::NONE);
        a.update(Event::Paste("ignored".to_string()));
        assert_eq!(a.state.editor().text(), "select 1;\nselect 2;");
    }

    #[test]
    fn pasted_text_with_a_quit_key_sequence_is_just_text() {
        let mut a = app();
        a.update(Event::Paste("\u{11}".to_string())); // Ctrl+Q as a raw byte
        assert!(!a.state.quit);
        assert_eq!(a.state.editor().text(), "");
    }

    #[test]
    fn viewport_follows_the_cursor() {
        let mut a = app();
        a.update(Event::Resize(80, 24));
        for _ in 0..40 {
            press(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        }
        assert!(a.state.doc().scroll_top > 0);
        let cursor_line = a.state.editor().cursor().line;
        let height = a.page_lines();
        assert!(
            cursor_line >= a.state.doc().scroll_top
                && cursor_line < a.state.doc().scroll_top + height
        );
        press(&mut a, KeyCode::Home, KeyModifiers::CONTROL);
        assert_eq!(a.state.doc().scroll_top, 0);
    }

    #[test]
    fn viewport_scrolls_sideways_for_long_lines() {
        let mut a = app();
        a.update(Event::Resize(80, 24));
        type_text(&mut a, &"x".repeat(200));
        assert!(a.state.doc().scroll_left > 100);
        press(&mut a, KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(a.state.doc().scroll_left, 0);
    }

    #[test]
    fn page_keys_move_by_the_visible_height() {
        let mut a = app();
        a.update(Event::Resize(80, 24));
        for _ in 0..60 {
            press(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        }
        press(&mut a, KeyCode::Home, KeyModifiers::CONTROL);
        let page = a.page_lines();
        press(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(a.state.editor().cursor().line, page);
    }

    fn ctrl(app: &mut App, c: char) {
        press(app, KeyCode::Char(c), KeyModifiers::CONTROL);
    }

    #[test]
    fn new_buffer_is_empty_active_and_focuses_the_editor() {
        let mut a = app();
        type_text(&mut a, "first");
        alt(&mut a, '3');
        ctrl(&mut a, 'n');
        assert_eq!(a.state.docs.len(), 2);
        assert_eq!(a.state.active, 1);
        assert_eq!(a.state.focus, Pane::Editor);
        assert_eq!(a.state.editor().text(), "");
        assert_eq!(a.state.doc().name, "Buffer 2");
    }

    #[test]
    fn buffers_keep_their_own_text_cursor_and_scroll() {
        let mut a = app();
        a.update(Event::Resize(80, 24));
        for _ in 0..40 {
            press(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        }
        let first_top = a.state.doc().scroll_top;
        assert!(first_top > 0);
        ctrl(&mut a, 'n');
        type_text(&mut a, "second");
        assert_eq!(a.state.doc().scroll_top, 0);
        press(&mut a, KeyCode::Left, KeyModifiers::ALT);
        assert_eq!(a.state.active, 0);
        assert_eq!(a.state.doc().scroll_top, first_top);
        assert_eq!(a.state.editor().line_count(), 41);
        press(&mut a, KeyCode::Right, KeyModifiers::ALT);
        assert_eq!(a.state.editor().text(), "second");
    }

    #[test]
    fn buffers_have_separate_undo_history() {
        let mut a = app();
        type_text(&mut a, "one");
        ctrl(&mut a, 'n');
        ctrl(&mut a, 'z');
        assert_eq!(
            a.state.editor().text(),
            "",
            "nothing to undo in the new buffer"
        );
        press(&mut a, KeyCode::PageUp, KeyModifiers::CONTROL);
        assert_eq!(a.state.editor().text(), "one");
        ctrl(&mut a, 'z');
        assert_eq!(a.state.editor().text(), "");
    }

    #[test]
    fn switching_wraps_around() {
        let mut a = app();
        ctrl(&mut a, 'n');
        ctrl(&mut a, 'n');
        assert_eq!(a.state.active, 2);
        press(&mut a, KeyCode::PageDown, KeyModifiers::CONTROL);
        assert_eq!(a.state.active, 0);
        press(&mut a, KeyCode::PageUp, KeyModifiers::CONTROL);
        assert_eq!(a.state.active, 2);
    }

    #[test]
    fn closing_selects_a_neighbour_and_can_be_undone() {
        let mut a = app();
        type_text(&mut a, "keep");
        ctrl(&mut a, 'n');
        type_text(&mut a, "lose");
        ctrl(&mut a, 'w');
        assert_eq!(a.state.docs.len(), 1);
        assert_eq!(a.state.editor().text(), "keep");
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Closed Buffer 2") && n.contains("Alt+T")));
        alt(&mut a, 't');
        assert_eq!(a.state.docs.len(), 2);
        assert_eq!(a.state.editor().text(), "lose");
        assert_eq!(a.state.doc().name, "Buffer 2");
    }

    #[test]
    fn closing_the_last_buffer_leaves_a_fresh_empty_one() {
        let mut a = app();
        type_text(&mut a, "text");
        ctrl(&mut a, 'w');
        assert_eq!(a.state.docs.len(), 1);
        assert_eq!(a.state.editor().text(), "");
        assert_eq!(
            a.state.doc().name,
            "Buffer 2",
            "a closed name is not reused"
        );
        alt(&mut a, 't');
        assert_eq!(a.state.editor().text(), "text");
    }

    #[test]
    fn empty_buffers_are_not_kept_for_reopening() {
        let mut a = app();
        ctrl(&mut a, 'n');
        ctrl(&mut a, 'w');
        assert!(a.state.closed.is_empty());
        alt(&mut a, 't');
        assert_eq!(a.state.docs.len(), 1);
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("No closed buffer")));
    }

    #[test]
    fn only_the_most_recent_closed_buffers_are_kept() {
        let mut a = app();
        for i in 0..CLOSED_LIMIT + 3 {
            type_text(&mut a, &i.to_string());
            ctrl(&mut a, 'w');
        }
        assert_eq!(a.state.closed.len(), CLOSED_LIMIT);
    }

    #[test]
    fn buffer_keys_work_from_any_pane() {
        let mut a = app();
        alt(&mut a, '3');
        ctrl(&mut a, 'n');
        assert_eq!(a.state.docs.len(), 2);
    }

    #[test]
    fn the_register_is_shared_between_buffers() {
        let mut a = app();
        type_text(&mut a, "shared");
        ctrl(&mut a, 'a');
        ctrl(&mut a, 'x');
        ctrl(&mut a, 'n');
        ctrl(&mut a, 'v');
        assert_eq!(a.state.editor().text(), "shared");
    }

    fn items(app: &App) -> Vec<String> {
        app.state
            .completion
            .as_ref()
            .map(|c| c.items.iter().map(|i| i.text.clone()).collect())
            .unwrap_or_default()
    }

    fn enter(app: &mut App) {
        press(app, KeyCode::Enter, KeyModifiers::NONE);
    }

    fn esc(app: &mut App) {
        press(app, KeyCode::Esc, KeyModifiers::NONE);
    }

    #[test]
    fn suggestions_appear_after_the_minimum_number_of_characters() {
        let mut a = app();
        type_text(&mut a, "s");
        assert!(a.state.completion.is_none());
        type_text(&mut a, "e");
        assert_eq!(items(&a)[..2], ["set", "select"]);
    }

    #[test]
    fn enter_accepts_the_highlighted_suggestion_without_a_newline() {
        let mut a = app();
        type_text(&mut a, "sel");
        assert_eq!(items(&a), ["select"]);
        enter(&mut a);
        assert_eq!(a.state.editor().text(), "select");
        assert!(a.state.completion.is_none());
        assert_eq!(a.state.editor().cursor().col, 6);
    }

    #[test]
    fn enter_without_a_popup_inserts_a_newline() {
        let mut a = app();
        type_text(&mut a, "x");
        enter(&mut a);
        assert_eq!(a.state.editor().text(), "x\n");
    }

    #[test]
    fn escape_dismisses_without_changing_the_buffer() {
        let mut a = app();
        type_text(&mut a, "sel");
        esc(&mut a);
        assert!(a.state.completion.is_none());
        assert_eq!(a.state.editor().text(), "sel");
        enter(&mut a);
        assert_eq!(a.state.editor().text(), "sel\n", "Enter is a newline again");
    }

    #[test]
    fn tab_also_accepts() {
        let mut a = app();
        type_text(&mut a, "sel");
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.editor().text(), "select");
    }

    #[test]
    fn arrows_choose_a_suggestion_and_wrap() {
        let mut a = app();
        type_text(&mut a, "de");
        assert_eq!(items(&a), ["desc", "delete", "declare", "default"]);
        press(&mut a, KeyCode::Down, KeyModifiers::NONE);
        enter(&mut a);
        assert_eq!(a.state.editor().text(), "delete");

        let mut a = app();
        type_text(&mut a, "de");
        press(&mut a, KeyCode::Up, KeyModifiers::NONE);
        enter(&mut a);
        assert_eq!(
            a.state.editor().text(),
            "default",
            "Up from the first wraps to the last"
        );
    }

    #[test]
    fn capitals_give_capital_keywords() {
        let mut a = app();
        type_text(&mut a, "SEL");
        enter(&mut a);
        assert_eq!(a.state.editor().text(), "SELECT");
    }

    #[test]
    fn typing_on_filters_and_backspace_widens() {
        let mut a = app();
        type_text(&mut a, "de");
        assert_eq!(items(&a).len(), 4);
        type_text(&mut a, "l");
        assert_eq!(items(&a), ["delete"]);
        press(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(items(&a).len(), 4);
        press(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
        assert!(a.state.completion.is_none(), "below the minimum again");
    }

    #[test]
    fn a_fully_typed_keyword_closes_the_popup() {
        let mut a = app();
        type_text(&mut a, "select");
        assert!(a.state.completion.is_none());
    }

    #[test]
    fn moving_or_leaving_closes_the_popup_and_keys_do_not_act_twice() {
        let mut a = app();
        type_text(&mut a, "sel");
        press(&mut a, KeyCode::Left, KeyModifiers::NONE);
        assert!(a.state.completion.is_none());
        assert_eq!(
            a.state.editor().cursor().col,
            2,
            "the arrow also moved the cursor"
        );

        let mut a = app();
        type_text(&mut a, "sel");
        alt(&mut a, '3');
        assert!(a.state.completion.is_none());

        let mut a = app();
        type_text(&mut a, "sel");
        ctrl(&mut a, 'n');
        assert!(a.state.completion.is_none());
    }

    #[test]
    fn no_suggestions_in_strings_comments_or_after_a_dot() {
        for text in ["'sel", "-- sel", "t.sel", "12"] {
            let mut a = app();
            // Paste so that auto-pairing does not change the text.
            a.update(Event::Paste(text.to_string()));
            type_text(&mut a, "e");
            assert!(a.state.completion.is_none(), "{text}");
        }
    }

    #[test]
    fn paste_closes_the_popup() {
        let mut a = app();
        type_text(&mut a, "sel");
        a.update(Event::Paste("x".to_string()));
        assert!(a.state.completion.is_none());
    }

    #[test]
    fn the_popup_can_be_switched_off_and_triggered_by_hand() {
        let mut config = Config::default();
        config.editor.completion = false;
        let mut a = app_with(config);
        type_text(&mut a, "sel");
        assert!(a.state.completion.is_none());
        press(&mut a, KeyCode::Char(' '), KeyModifiers::CONTROL);
        assert_eq!(items(&a), ["select"]);
        enter(&mut a);
        assert_eq!(a.state.editor().text(), "select");
    }

    #[test]
    fn manual_trigger_works_with_a_single_character() {
        let mut a = app();
        type_text(&mut a, "w");
        assert!(a.state.completion.is_none());
        alt(&mut a, '/');
        assert!(items(&a).contains(&"where".to_string()));
    }

    #[test]
    fn minimum_characters_is_configurable() {
        let mut config = Config::default();
        config.editor.completion_min_chars = 1;
        let mut a = app_with(config);
        type_text(&mut a, "w");
        assert!(a.state.completion.is_some());
    }

    #[test]
    fn accepting_is_one_undo_step_back_to_the_prefix() {
        let mut a = app();
        type_text(&mut a, "sel");
        enter(&mut a);
        ctrl(&mut a, 'z');
        assert_eq!(a.state.editor().text(), "sel");
    }

    #[test]
    fn completion_keys_do_not_leak_into_normal_editing() {
        let mut a = app();
        type_text(&mut a, "x");
        // No popup is open, so Up and Down move the cursor as usual.
        press(&mut a, KeyCode::Up, KeyModifiers::NONE);
        press(&mut a, KeyCode::Down, KeyModifiers::NONE);
        assert!(a.state.completion.is_none());
        esc(&mut a);
        assert_eq!(a.state.editor().text(), "x");
    }

    #[test]
    fn the_hint_bar_offers_the_popup_keys_while_it_is_open() {
        let mut a = app();
        type_text(&mut a, "sel");
        assert_eq!(
            a.hint_key(Action::CompletionAccept)
                .map(|k| k.to_string())
                .as_deref(),
            Some("Enter")
        );
        assert!(a.hint_actions().contains(&Action::CompletionDismiss));
        esc(&mut a);
        assert!(a.hint_actions().contains(&Action::QueryRun));
        assert!(a.hint_actions().contains(&Action::FocusCommands));
    }

    #[test]
    fn help_scrolls_and_stops_at_the_end() {
        let mut a = app();
        a.update(Event::Resize(80, 24));
        press(&mut a, KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(a.state.help_scroll, 0);
        press(&mut a, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(a.state.help_scroll, 1);
        for _ in 0..20 {
            press(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
        }
        let (visible, max) = a.help_limits();
        assert_eq!(a.state.help_scroll, max);
        assert!(max > 0 && visible < a.help_lines().len());
        for _ in 0..20 {
            press(&mut a, KeyCode::PageUp, KeyModifiers::NONE);
        }
        assert_eq!(a.state.help_scroll, 0);
        // Closing and reopening starts at the top again.
        press(&mut a, KeyCode::Down, KeyModifiers::NONE);
        press(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut a, KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(a.state.help_scroll, 0);
    }

    #[test]
    fn f1_opens_help_in_any_pane_and_dialog_swallows_other_keys() {
        let mut a = app();
        press(&mut a, KeyCode::F(1), KeyModifiers::NONE);
        assert!(a.state.help_open);
        alt(&mut a, '3'); // ignored while help is open
        assert_eq!(a.state.focus, Pane::Editor);
        press(&mut a, KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(!a.state.quit, "global quit is also held back by the dialog");
        press(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!a.state.help_open);
    }

    #[test]
    fn ctrl_q_quits() {
        let mut a = app();
        press(&mut a, KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(a.state.quit);
    }

    #[test]
    fn ctrl_c_is_unbound() {
        let mut a = app();
        let before = a.state.clone();
        press(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(a.state, before);
    }

    #[test]
    fn resize_is_recorded() {
        let mut a = app();
        a.update(Event::Resize(120, 40));
        assert_eq!(a.state.size, (120, 40));
    }

    // -- connecting and running ------------------------------------------------

    use crate::command::Command;
    use crate::db::{ConnectionInfo, DbError, QueryOutput, ResultSet};
    use crate::events::Background;
    use crate::form::ConnectForm;
    use std::time::Duration;

    fn send(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Vec<Command> {
        app.update(Event::Key(KeyEvent::new(code, mods)))
    }

    fn type_into(app: &mut App, text: &str) -> Vec<Command> {
        let mut all = Vec::new();
        for c in text.chars() {
            all.extend(send(app, KeyCode::Char(c), KeyModifiers::NONE));
        }
        all
    }

    fn open_picker(app: &mut App) {
        send(app, KeyCode::Char('o'), KeyModifiers::CONTROL);
    }

    fn info() -> ConnectionInfo {
        ConnectionInfo {
            host: "127.0.0.1".to_string(),
            port: 5432,
            database: "app".to_string(),
            user: "alice".to_string(),
            server_version: "16.1".to_string(),
            tls: false,
        }
    }

    /// Connects the active buffer through the picker and a URI.
    fn connected(app: &mut App) -> ConnId {
        open_picker(app);
        type_into(app, "postgres://alice:pw@127.0.0.1/app");
        let commands = send(app, KeyCode::Enter, KeyModifiers::NONE);
        let Some(Command::Connect { conn, .. }) = commands.into_iter().next() else {
            panic!("expected a Connect command");
        };
        app.update(Event::Background(Background::Connected {
            conn,
            info: info(),
        }));
        conn
    }

    fn rows_set(rows: &[&[Option<&str>]]) -> ResultSet {
        ResultSet {
            columns: vec!["a".to_string(), "b".to_string()],
            rows: rows
                .iter()
                .map(|r| r.iter().map(|c| c.map(str::to_string)).collect())
                .collect(),
            total_rows: rows.len() as u64,
            tag: format!("SELECT {}", rows.len()),
        }
    }

    fn output(sets: Vec<ResultSet>, error: Option<DbError>) -> QueryOutput {
        QueryOutput {
            sets,
            error,
            elapsed: Duration::from_millis(12),
            tx: crate::db::TxState::Idle,
        }
    }

    #[test]
    fn ctrl_o_opens_the_picker_which_takes_every_key() {
        let mut a = app();
        open_picker(&mut a);
        assert!(matches!(a.state.dialog, Some(Dialog::Picker(_))));
        // Keys that would act elsewhere are text or ignored here.
        type_into(&mut a, "q");
        alt(&mut a, '3');
        assert_eq!(a.state.focus, Pane::Editor);
        assert_eq!(
            a.state.editor().text(),
            "",
            "typing goes to the picker, not the buffer"
        );
        esc(&mut a);
        assert!(a.state.dialog.is_none());
    }

    #[test]
    fn a_typed_uri_starts_a_connection_for_the_active_buffer() {
        let mut a = app();
        open_picker(&mut a);
        type_into(&mut a, "postgres://alice:s3cret@127.0.0.1:5433/app");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(commands.len(), 1);
        let Command::Connect {
            conn,
            profile,
            password,
            lookup,
        } = &commands[0]
        else {
            panic!("expected Connect, got {commands:?}");
        };
        assert_eq!((profile.host.as_str(), profile.port), ("127.0.0.1", 5433));
        assert_eq!(password.as_ref().map(Secret::expose), Some("s3cret"));
        assert!(!lookup, "a password in the URI is used as given");
        assert_eq!(a.state.doc().connection, Some(*conn));
        assert_eq!(
            a.state.connection(*conn).map(|c| c.status.clone()),
            Some(ConnStatus::Connecting)
        );
        assert!(a.state.dialog.is_none());
    }

    #[test]
    fn the_password_never_shows_up_in_state_debug_output_or_notices() {
        let mut a = app();
        open_picker(&mut a);
        type_into(&mut a, "postgres://alice:topsecretpw@127.0.0.1/app");
        send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!format!("{:?}", a.state).contains("topsecretpw"));
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| !n.contains("topsecretpw")));
    }

    #[test]
    fn a_bad_uri_keeps_the_picker_open_with_a_message_that_hides_the_input() {
        let mut a = app();
        open_picker(&mut a);
        type_into(&mut a, "postgres://alice:topsecretpw@127.0.0.1:99999/app");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(commands.is_empty());
        assert!(a.state.dialog.is_some());
        let notice = a.state.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("Invalid connection URI") && !notice.contains("topsecretpw"),
            "{notice}"
        );
    }

    #[test]
    fn saved_profiles_are_picked_with_fuzzy_search_and_remembered() {
        let mut a = app();
        let mut p = Profile::new("production");
        p.host = "db.example.com".to_string();
        a.state.profiles.add(p).expect("add");
        a.state.profiles.add(Profile::new("scratch")).expect("add");
        open_picker(&mut a);
        type_into(&mut a, "prd");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            matches!(&commands[..], [Command::SaveProfiles { .. }, Command::Connect { profile, lookup: true, .. }] if profile.name == "production")
        );
        assert!(a
            .state
            .profiles
            .get("production")
            .and_then(|p| p.last_used)
            .is_some());
        assert!(a
            .state
            .profiles
            .get("scratch")
            .and_then(|p| p.last_used)
            .is_none());
    }

    #[test]
    fn no_match_still_offers_a_new_connection() {
        let mut a = app();
        open_picker(&mut a);
        type_into(&mut a, "zzz");
        assert!(send(&mut a, KeyCode::Enter, KeyModifiers::NONE).is_empty());
        assert!(
            matches!(a.state.dialog, Some(Dialog::Form(_))),
            "Enter opens the details form"
        );
    }

    #[test]
    fn arrows_choose_a_profile() {
        let mut a = app();
        for name in ["alpha", "bravo"] {
            a.state.profiles.add(Profile::new(name)).expect("add");
        }
        open_picker(&mut a);
        send(&mut a, KeyCode::Down, KeyModifiers::NONE);
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            matches!(commands.last(), Some(Command::Connect { profile, .. }) if profile.name == "bravo")
        );
    }

    #[test]
    fn a_typed_uri_can_be_saved_without_its_password() {
        let mut a = app();
        open_picker(&mut a);
        type_into(&mut a, "postgres://alice:topsecretpw@db.example.com/sales");
        let commands = send(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
        let Some(Command::SaveProfiles { contents }) = commands.first() else {
            panic!("expected SaveProfiles, got {commands:?}");
        };
        assert!(!contents.contains("topsecretpw"));
        assert!(contents.contains("db.example.com") && contents.contains("sales"));
        assert!(a.state.profiles.get("db.example.com/sales").is_some());
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("password was not saved")));
        // Saving the same address again does not overwrite it.
        send(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(a.state.profiles.get("db.example.com/sales (2)").is_some());
    }

    #[test]
    fn saving_needs_a_uri() {
        let mut a = app();
        open_picker(&mut a);
        type_into(&mut a, "hello");
        assert!(send(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL).is_empty());
        assert!(a.state.profiles.is_empty());
    }

    #[test]
    fn pasting_into_the_picker_inserts_one_line() {
        let mut a = app();
        open_picker(&mut a);
        a.update(Event::Paste("postgres://u@h/d\nextra".to_string()));
        let Some(Dialog::Picker(p)) = &a.state.dialog else {
            panic!("picker")
        };
        assert_eq!(p.input, "postgres://u@h/dextra");
        assert_eq!(a.state.editor().text(), "");
    }

    #[test]
    fn connecting_updates_the_status_and_shows_the_connection() {
        let mut a = app();
        let conn = connected(&mut a);
        let c = a.state.connection(conn).expect("connection");
        assert_eq!(c.status, ConnStatus::Connected);
        assert_eq!(
            a.state.active_connection().and_then(|c| c.info.clone()),
            Some(info())
        );
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Connected to") && n.contains("16.1")));
    }

    #[test]
    fn a_new_connection_in_the_same_buffer_replaces_the_old_one() {
        let mut a = app();
        let first = connected(&mut a);
        open_picker(&mut a);
        type_into(&mut a, "postgres://bob@127.0.0.1/other");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            matches!(commands[0], Command::Disconnect { conn } if conn == first),
            "{commands:?}"
        );
        assert!(a.state.connection(first).is_none());
        assert_eq!(a.state.connections.len(), 1);
    }

    #[test]
    fn a_shared_connection_is_kept_while_another_buffer_uses_it() {
        let mut a = app();
        let conn = connected(&mut a);
        ctrl(&mut a, 'n');
        // Point the second buffer at the same connection.
        a.state.docs[1].connection = Some(conn);
        open_picker(&mut a);
        type_into(&mut a, "postgres://bob@127.0.0.1/other");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(commands
            .iter()
            .all(|c| !matches!(c, Command::Disconnect { .. })));
        assert!(a.state.connection(conn).is_some());
    }

    #[test]
    fn each_buffer_has_its_own_connection() {
        let mut a = app();
        let first = connected(&mut a);
        ctrl(&mut a, 'n');
        assert_eq!(a.state.doc().connection, None);
        let second = connected(&mut a);
        assert_ne!(first, second);
        press(&mut a, KeyCode::Left, KeyModifiers::ALT);
        assert_eq!(a.state.doc().connection, Some(first));
    }

    #[test]
    fn running_without_a_connection_says_how_to_connect() {
        let mut a = app();
        type_into(&mut a, "select 1");
        assert!(press_run(&mut a).is_empty());
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Not connected") && n.contains("Ctrl+O")));
    }

    fn press_run(app: &mut App) -> Vec<Command> {
        send(app, KeyCode::Char('r'), KeyModifiers::CONTROL)
    }

    #[test]
    fn ctrl_r_runs_the_statement_under_the_cursor() {
        let mut a = app();
        let conn = connected(&mut a);
        a.update(Event::Paste("select 1;\nselect 2;\nselect 3".to_string()));
        send(&mut a, KeyCode::Up, KeyModifiers::NONE);
        let commands = press_run(&mut a);
        assert!(
            matches!(&commands[..], [Command::Run { conn: c, sql, .. }] if *c == conn && sql == "select 2"),
            "{commands:?}"
        );
        assert!(a.state.running.is_some());
    }

    #[test]
    fn run_works_from_the_editor_and_by_a_single_key_elsewhere() {
        // Ctrl+R in the editor.
        let mut a = app();
        connected(&mut a);
        type_into(&mut a, "select 1");
        assert!(matches!(press_run(&mut a)[..], [Command::Run { .. }]));

        // `r` in the commands pane and in the results pane.
        for pane_key in ['1', '3'] {
            let mut a = app();
            connected(&mut a);
            type_into(&mut a, "select 1");
            alt(&mut a, pane_key);
            let commands = send(&mut a, KeyCode::Char('r'), KeyModifiers::NONE);
            assert!(
                matches!(commands[..], [Command::Run { .. }]),
                "r in pane {pane_key}"
            );
        }
    }

    #[test]
    fn old_run_keys_are_not_bound_by_default() {
        let mut a = app();
        connected(&mut a);
        type_into(&mut a, "select 1");
        for (code, mods) in [
            (KeyCode::F(5), KeyModifiers::NONE),
            (KeyCode::Enter, KeyModifiers::ALT),
        ] {
            let commands = send(&mut a, code, mods);
            assert!(commands.is_empty(), "{code:?} {mods:?}");
        }
    }

    #[test]
    fn a_selection_runs_instead_of_the_statement() {
        let mut a = app();
        connected(&mut a);
        a.update(Event::Paste("select 1; select 2".to_string()));
        ctrl(&mut a, 'a');
        let commands = press_run(&mut a);
        assert!(matches!(&commands[..], [Command::Run { sql, .. }] if sql == "select 1; select 2"));
    }

    #[test]
    fn an_empty_buffer_runs_nothing() {
        let mut a = app();
        connected(&mut a);
        assert!(press_run(&mut a).is_empty());
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Nothing to run")));
        a.update(Event::Paste("-- only a comment".to_string()));
        assert!(press_run(&mut a).is_empty());
    }

    #[test]
    fn the_editor_stays_editable_while_a_statement_runs() {
        let mut a = app();
        connected(&mut a);
        type_into(&mut a, "select pg_sleep(5)");
        press_run(&mut a);
        type_into(&mut a, " -- more");
        assert!(a.state.editor().text().ends_with("-- more"));
    }

    #[test]
    fn a_second_run_is_refused_while_one_is_running() {
        let mut a = app();
        connected(&mut a);
        type_into(&mut a, "select 1");
        press_run(&mut a);
        assert!(press_run(&mut a).is_empty());
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("already running") && n.contains("Ctrl+C")));
    }

    #[test]
    fn ctrl_c_cancels_only_a_running_statement() {
        let mut a = app();
        let conn = connected(&mut a);
        // The first statistics and schema reads are over, so nothing else is
        // on the connection.
        a.update(Event::Background(Background::Stats {
            conn,
            result: Ok(sample_stats()),
        }));
        a.update(Event::Background(Background::Catalog {
            conn,
            result: Ok(sample_catalog()),
        }));
        assert!(
            send(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL).is_empty(),
            "nothing to cancel"
        );
        type_into(&mut a, "select pg_sleep(5)");
        press_run(&mut a);
        let commands = send(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(commands[..], [Command::Cancel { conn: c }] if c == conn));
        assert_eq!(
            a.state.editor().text(),
            "select pg_sleep(5)",
            "Ctrl+C never edits"
        );
    }

    #[test]
    fn finished_results_are_stored_per_buffer_and_the_pane_is_shown() {
        let mut a = app();
        connected(&mut a);
        type_into(&mut a, "select 1");
        let commands = press_run(&mut a);
        let Command::Run { conn, doc, .. } = commands[0].clone() else {
            panic!("run")
        };
        a.state.show_results = false;
        a.update(Event::Background(Background::QueryFinished {
            conn,
            doc,
            output: output(vec![rows_set(&[&[Some("1"), None]])], None),
        }));
        assert!(a.state.running.is_none());
        assert!(a.state.show_results);
        assert!(a.state.doc().results.is_some());
        ctrl(&mut a, 'n');
        assert!(
            a.state.doc().results.is_none(),
            "another buffer has no results"
        );
    }

    #[test]
    fn results_for_a_closed_buffer_are_dropped() {
        let mut a = app();
        connected(&mut a);
        type_into(&mut a, "select 1");
        let Command::Run { conn, doc, .. } = press_run(&mut a)[0].clone() else {
            panic!("run")
        };
        ctrl(&mut a, 'n');
        press(&mut a, KeyCode::Left, KeyModifiers::ALT);
        ctrl(&mut a, 'w'); // closes the buffer the statement was started in
        assert_eq!(a.state.docs.len(), 1);
        a.update(Event::Background(Background::QueryFinished {
            conn,
            doc,
            output: output(vec![], None),
        }));
        assert!(a.state.running.is_none());
    }

    #[test]
    fn a_cancelled_query_says_so() {
        let mut a = app();
        connected(&mut a);
        type_into(&mut a, "select pg_sleep(9)");
        let Command::Run { conn, doc, .. } = press_run(&mut a)[0].clone() else {
            panic!("run")
        };
        let mut error = DbError::new("canceling statement due to user request");
        error.sqlstate = Some("57014".to_string());
        a.update(Event::Background(Background::QueryFinished {
            conn,
            doc,
            output: output(vec![], Some(error)),
        }));
        assert_eq!(a.state.notice.as_deref(), Some("Query cancelled"));
    }

    #[test]
    fn results_keys_scroll_the_grid_and_switch_tabs() {
        let mut a = app();
        a.update(Event::Resize(100, 30));
        connected(&mut a);
        type_into(&mut a, "select 1");
        let Command::Run { conn, doc, .. } = press_run(&mut a)[0].clone() else {
            panic!("run")
        };
        let many: Vec<Vec<Option<&str>>> = (0..100).map(|_| vec![Some("x"), Some("y")]).collect();
        let slices: Vec<&[Option<&str>]> = many.iter().map(Vec::as_slice).collect();
        let sets = vec![rows_set(&slices), rows_set(&slices[..3])];
        a.update(Event::Background(Background::QueryFinished {
            conn,
            doc,
            output: output(sets, None),
        }));
        alt(&mut a, '3');
        let tab = |a: &App| {
            a.state
                .doc()
                .results
                .as_ref()
                .map(|r| (r.tab, r.row, r.col))
        };
        assert_eq!(
            tab(&a),
            Some((1, 0, 0)),
            "opens on the last statement that returned rows"
        );
        send(&mut a, KeyCode::Char('['), KeyModifiers::NONE);
        assert_eq!(tab(&a), Some((0, 0, 0)));
        send(&mut a, KeyCode::Down, KeyModifiers::NONE);
        send(&mut a, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(tab(&a), Some((0, 2, 0)));
        send(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
        assert!(tab(&a).is_some_and(|t| t.1 > 2));
        send(&mut a, KeyCode::End, KeyModifiers::NONE);
        let last = tab(&a).map(|t| t.1).unwrap_or(0);
        assert!(last > 50 && last < 100, "the last page is full: {last}");
        send(&mut a, KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(tab(&a), Some((0, 0, 0)));
        send(&mut a, KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(tab(&a), Some((0, 0, 1)));
        send(&mut a, KeyCode::Char(']'), KeyModifiers::NONE);
        assert_eq!(tab(&a), Some((1, 0, 0)));
    }

    #[test]
    fn results_keys_do_nothing_in_the_editor() {
        let mut a = app();
        type_into(&mut a, "abc");
        send(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
        send(&mut a, KeyCode::Char('['), KeyModifiers::NONE);
        assert_eq!(
            a.state.editor().text(),
            "abc[]",
            "[ is typed in the editor (and auto-paired), not a results key"
        );
    }

    // -- passwords ------------------------------------------------------------

    fn needs_password(app: &mut App) -> ConnId {
        open_picker(app);
        type_into(app, "postgres://alice@127.0.0.1/app");
        let Command::Connect { conn, lookup, .. } =
            send(app, KeyCode::Enter, KeyModifiers::NONE)[0].clone()
        else {
            panic!("connect");
        };
        assert!(lookup);
        app.update(Event::Background(Background::PasswordNeeded {
            conn,
            notes: Vec::new(),
        }));
        conn
    }

    #[test]
    fn a_missing_password_opens_the_prompt() {
        let mut a = app();
        let conn = needs_password(&mut a);
        assert!(
            matches!(&a.state.dialog, Some(Dialog::Password(p)) if p.conn == conn && p.error.is_none())
        );
        assert_eq!(
            a.state.connection(conn).map(|c| c.status.clone()),
            Some(ConnStatus::AwaitingPassword)
        );
    }

    #[test]
    fn the_prompt_collects_a_password_and_connects_without_looking_again() {
        let mut a = app();
        let conn = needs_password(&mut a);
        type_into(&mut a, "hunter2x");
        send(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
        assert!(!format!("{:?}", a.state).contains("hunter2"));
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        let [Command::Connect {
            conn: c,
            password,
            lookup,
            ..
        }] = &commands[..]
        else {
            panic!("{commands:?}")
        };
        assert_eq!(*c, conn);
        assert_eq!(password.as_ref().map(Secret::expose), Some("hunter2"));
        assert!(!lookup);
        assert!(a.state.dialog.is_none());
    }

    #[test]
    fn an_empty_password_connects_without_one() {
        let mut a = app();
        needs_password(&mut a);
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(
            &commands[..],
            [Command::Connect {
                password: None,
                lookup: false,
                ..
            }]
        ));
    }

    #[test]
    fn typing_in_the_prompt_never_reaches_the_buffer_or_other_dialogs() {
        let mut a = app();
        needs_password(&mut a);
        type_into(&mut a, "select");
        assert_eq!(a.state.editor().text(), "");
    }

    #[test]
    fn escape_cancels_the_connection_attempt() {
        let mut a = app();
        let conn = needs_password(&mut a);
        let commands = send(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(matches!(commands[..], [Command::Disconnect { conn: c }] if c == conn));
        assert!(a.state.dialog.is_none());
        assert!(a.state.connection(conn).is_none());
        assert_eq!(a.state.doc().connection, None);
    }

    #[test]
    fn a_rejected_password_reopens_the_prompt_with_the_reason() {
        let mut a = app();
        let conn = needs_password(&mut a);
        type_into(&mut a, "wrong");
        send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        let mut error = DbError::new("password authentication failed for user \"alice\"");
        error.sqlstate = Some("28P01".to_string());
        a.update(Event::Background(Background::ConnectFailed { conn, error }));
        let Some(Dialog::Password(p)) = &a.state.dialog else {
            panic!("prompt")
        };
        assert!(p
            .error
            .as_deref()
            .is_some_and(|e| e.contains("authentication failed")));
        assert_eq!(p.input.expose(), "", "the wrong password is not kept");
    }

    #[test]
    fn a_rejected_password_does_not_loop_when_prompting_is_off() {
        let mut config = Config::default();
        config.connections.password_sources = vec![Source::Env];
        let mut a = app_with(config);
        open_picker(&mut a);
        type_into(&mut a, "postgres://alice@127.0.0.1/app");
        let Command::Connect { conn, .. } =
            send(&mut a, KeyCode::Enter, KeyModifiers::NONE)[0].clone()
        else {
            panic!()
        };
        let mut error = DbError::new("password authentication failed");
        error.sqlstate = Some("28P01".to_string());
        a.update(Event::Background(Background::ConnectFailed { conn, error }));
        assert!(a.state.dialog.is_none());
        assert!(matches!(
            a.state.connection(conn).map(|c| &c.status),
            Some(ConnStatus::Failed(_))
        ));
    }

    #[test]
    fn other_connection_errors_are_shown_not_prompted() {
        let mut a = app();
        open_picker(&mut a);
        type_into(&mut a, "postgres://alice@127.0.0.1/app");
        let Command::Connect { conn, .. } =
            send(&mut a, KeyCode::Enter, KeyModifiers::NONE)[0].clone()
        else {
            panic!()
        };
        a.update(Event::Background(Background::ConnectFailed {
            conn,
            error: DbError::new("connection refused"),
        }));
        assert!(a.state.dialog.is_none());
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("connection refused")));
    }

    #[test]
    fn events_for_forgotten_connections_are_ignored() {
        let mut a = app();
        a.update(Event::Background(Background::Connected {
            conn: 99,
            info: info(),
        }));
        a.update(Event::Background(Background::PasswordNeeded {
            conn: 99,
            notes: Vec::new(),
        }));
        a.update(Event::Background(Background::ConnectionClosed {
            conn: 99,
            reason: "x".to_string(),
        }));
        a.update(Event::Background(Background::ConnectFailed {
            conn: 99,
            error: DbError::new("x"),
        }));
        assert!(a.state.dialog.is_none() && a.state.connections.is_empty());
    }

    // -- losing a connection --------------------------------------------------

    #[test]
    fn a_lost_connection_keeps_the_text_and_offers_to_reconnect() {
        let mut a = app();
        let conn = connected(&mut a);
        type_into(&mut a, "select 1");
        press_run(&mut a);
        a.update(Event::Background(Background::ConnectionClosed {
            conn,
            reason: "server closed the connection".to_string(),
        }));
        assert!(a.state.running.is_none());
        assert!(matches!(
            a.state.connection(conn).map(|c| &c.status),
            Some(ConnStatus::Closed(_))
        ));
        assert_eq!(a.state.editor().text(), "select 1");
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Alt+O to reconnect")));

        let commands = send(&mut a, KeyCode::Char('o'), KeyModifiers::ALT);
        assert!(
            matches!(&commands[..], [Command::Connect { conn: c, lookup: true, .. }] if *c == conn)
        );
        assert_eq!(
            a.state.connection(conn).map(|c| c.status.clone()),
            Some(ConnStatus::Connecting)
        );
    }

    #[test]
    fn running_on_a_lost_connection_is_refused_with_a_reason() {
        let mut a = app();
        let conn = connected(&mut a);
        a.update(Event::Background(Background::ConnectionClosed {
            conn,
            reason: "gone".to_string(),
        }));
        type_into(&mut a, "select 1");
        assert!(press_run(&mut a).is_empty());
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("not ready")));
    }

    #[test]
    fn reconnect_does_nothing_while_connected_or_without_a_connection() {
        let mut a = app();
        assert!(send(&mut a, KeyCode::Char('o'), KeyModifiers::ALT).is_empty());
        connected(&mut a);
        assert!(send(&mut a, KeyCode::Char('o'), KeyModifiers::ALT).is_empty());
    }

    #[test]
    fn the_tick_event_changes_nothing() {
        let mut a = app();
        let before = a.state.clone();
        assert!(a.update(Event::Tick).is_empty());
        assert_eq!(a.state, before);
    }

    // -- the landing screen and single keys ------------------------------------

    /// The app exactly as a user finds it at startup.
    fn landing() -> App {
        let mut a = App::new(Config::default(), Keymap::default_keymap());
        a.update(Event::Resize(100, 30));
        a
    }

    fn key(app: &mut App, c: char) -> Vec<Command> {
        send(app, KeyCode::Char(c), KeyModifiers::NONE)
    }

    // -- the session pane -------------------------------------------------------

    #[test]
    fn the_session_pane_takes_focus_and_changes_the_database_and_user() {
        let mut a = landing();
        let conn = connected(&mut a);
        a.update(Event::Key(KeyEvent::new(
            KeyCode::Char('4'),
            KeyModifiers::ALT,
        )));
        assert_eq!(a.state.focus, Pane::Session);
        key(&mut a, 'd');
        let Some(Dialog::Session(form)) = &a.state.dialog else {
            panic!("expected the session form")
        };
        assert_eq!(
            (form.database.as_str(), form.user.as_str(), form.focus),
            ("app", "alice", 0)
        );
        send(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        type_into(&mut a, "sales");
        send(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        send(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        type_into(&mut a, "bob");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        let [Command::Disconnect { conn: old }, Command::Connect { profile, .. }] = &commands[..]
        else {
            panic!("{commands:?}")
        };
        assert_eq!(*old, conn);
        assert_eq!(
            (profile.database.as_deref(), profile.user.as_deref()),
            (Some("sales"), Some("bob"))
        );
        assert!(a.state.dialog.is_none());
        assert!(a.state.profiles.is_empty(), "nothing is saved");
    }

    #[test]
    fn u_in_the_session_pane_starts_on_the_user_and_esc_leaves_it() {
        let mut a = landing();
        connected(&mut a);
        a.update(Event::Key(KeyEvent::new(
            KeyCode::Char('4'),
            KeyModifiers::ALT,
        )));
        key(&mut a, 'u');
        let Some(Dialog::Session(form)) = &a.state.dialog else {
            panic!("expected the session form")
        };
        assert_eq!(form.focus, 1);
        send(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(a.state.dialog.is_none());
        send(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Context);
    }

    #[test]
    fn the_session_cannot_be_changed_without_a_connection() {
        let mut a = landing();
        a.update(Event::Key(KeyEvent::new(
            KeyCode::Char('4'),
            KeyModifiers::ALT,
        )));
        key(&mut a, 'd');
        assert!(a.state.dialog.is_none());
        assert!(a
            .state
            .notice
            .as_deref()
            .unwrap_or("")
            .contains("Not connected"));
    }

    #[test]
    fn an_empty_user_is_refused_and_keeps_the_form_open() {
        let mut a = landing();
        connected(&mut a);
        a.update(Event::Key(KeyEvent::new(
            KeyCode::Char('4'),
            KeyModifiers::ALT,
        )));
        key(&mut a, 'u');
        send(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(commands.is_empty());
        let Some(Dialog::Session(form)) = &a.state.dialog else {
            panic!("expected the session form")
        };
        assert!(form.error.is_some());
    }

    // -- confirmations and transactions ----------------------------------------

    /// Connected as in `connected`, with the profile optionally marked production.
    fn connected_production(app: &mut App, production: bool) -> ConnId {
        let conn = connected(app);
        if let Some(c) = app.state.connection_mut(conn) {
            c.profile.production = production;
        }
        conn
    }

    fn run_sql(app: &mut App, text: &str) -> Vec<Command> {
        type_into(app, text);
        press_run(app)
    }

    #[test]
    fn a_plain_select_runs_at_once_even_on_production() {
        let mut a = app();
        connected_production(&mut a, true);
        let commands = run_sql(&mut a, "select 1");
        assert!(matches!(&commands[..], [Command::Run { .. }]));
        assert!(a.state.dialog.is_none());
    }

    #[test]
    fn a_write_on_production_needs_a_yes() {
        let mut a = app();
        connected_production(&mut a, true);
        let commands = run_sql(&mut a, "update t set a = 1 where id = 2");
        assert!(commands.is_empty());
        let Some(Dialog::Confirm(c)) = &a.state.dialog else {
            panic!("expected a confirmation")
        };
        assert!(c.production && c.reasons().len() == 1);
        assert!(c.sql.contains("update t"), "the statement is shown");
        // Enter alone does nothing (R15.5).
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(commands.is_empty());
        let Some(Dialog::Confirm(c)) = &a.state.dialog else {
            panic!("still asking")
        };
        assert!(c.error.is_some());
        // Anything but y or yes does not confirm.
        type_into(&mut a, "no");
        assert!(send(&mut a, KeyCode::Enter, KeyModifiers::NONE).is_empty());
        send(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        type_into(&mut a, "Yes");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(&commands[..], [Command::Run { sql, .. }] if sql.contains("update t")));
        assert!(a.state.dialog.is_none() && a.state.running.is_some());
    }

    #[test]
    fn a_write_off_production_runs_without_asking() {
        let mut a = app();
        connected_production(&mut a, false);
        let commands = run_sql(&mut a, "update t set a = 1 where id = 2");
        assert!(matches!(&commands[..], [Command::Run { .. }]));
    }

    #[test]
    fn update_without_where_and_drop_ask_on_any_connection() {
        for text in [
            "delete from t",
            "update t set a = 1",
            "drop table t",
            "truncate t",
        ] {
            let mut a = app();
            connected_production(&mut a, false);
            assert!(run_sql(&mut a, text).is_empty(), "{text}");
            assert!(matches!(a.state.dialog, Some(Dialog::Confirm(_))), "{text}");
        }
    }

    #[test]
    fn escape_cancels_a_confirmation_and_runs_nothing() {
        let mut a = app();
        connected_production(&mut a, false);
        run_sql(&mut a, "drop table t");
        type_into(&mut a, "y");
        let commands = send(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(commands.is_empty() && a.state.dialog.is_none());
        assert!(a.state.running.is_none());
    }

    fn finish_with_tx(app: &mut App, conn: ConnId, tx: TxState) {
        let doc = app.state.doc().id;
        let mut out = output(vec![], None);
        out.tx = tx;
        app.update(Event::Background(Background::QueryFinished {
            conn,
            doc,
            output: out,
        }));
    }

    #[test]
    fn the_transaction_state_follows_each_run() {
        let mut a = app();
        let conn = connected(&mut a);
        assert_eq!(
            a.state.active_connection().map(|c| c.tx),
            Some(TxState::Idle)
        );
        finish_with_tx(&mut a, conn, TxState::Open);
        assert_eq!(
            a.state.active_connection().map(|c| c.tx),
            Some(TxState::Open)
        );
        finish_with_tx(&mut a, conn, TxState::Failed);
        assert_eq!(
            a.state.active_connection().map(|c| c.tx),
            Some(TxState::Failed)
        );
        assert!(a
            .state
            .notice
            .as_deref()
            .unwrap_or("")
            .contains("roll back"));
        finish_with_tx(&mut a, conn, TxState::Idle);
        assert_eq!(
            a.state.active_connection().map(|c| c.tx),
            Some(TxState::Idle)
        );
    }

    #[test]
    fn rollback_runs_only_while_a_transaction_is_open() {
        let mut a = app();
        let conn = connected(&mut a);
        assert!(send(&mut a, KeyCode::Char('z'), KeyModifiers::ALT).is_empty());
        assert!(a
            .state
            .notice
            .as_deref()
            .unwrap_or("")
            .contains("No transaction"));
        finish_with_tx(&mut a, conn, TxState::Failed);
        let commands = send(&mut a, KeyCode::Char('z'), KeyModifiers::ALT);
        assert!(matches!(&commands[..], [Command::Run { sql, .. }] if sql == "ROLLBACK"));
    }

    // -- session statistics -----------------------------------------------------

    fn sample_stats() -> SessionStats {
        SessionStats {
            connections: 3,
            server_connections: 12,
            max_connections: 100,
            active: Some(1),
            idle_in_transaction: Some(0),
            oldest_transaction_secs: None,
            size_bytes: 1_288_490_188,
            cache_hit_percent: Some(99.2),
            deadlocks: 0,
            uptime_secs: 3 * 86400 + 4 * 3600,
            sees_all_sessions: true,
        }
    }

    fn stats_commands(commands: &[Command]) -> usize {
        commands
            .iter()
            .filter(|c| matches!(c, Command::Stats { .. }))
            .count()
    }

    /// Like `connected`, but returns what the Connected event asked for.
    fn connected_commands(app: &mut App) -> (ConnId, Vec<Command>) {
        open_picker(app);
        type_into(app, "postgres://alice:pw@127.0.0.1/app");
        let commands = send(app, KeyCode::Enter, KeyModifiers::NONE);
        let Some(Command::Connect { conn, .. }) = commands.into_iter().next() else {
            panic!("expected a Connect command");
        };
        let commands = app.update(Event::Background(Background::Connected {
            conn,
            info: info(),
        }));
        (conn, commands)
    }

    #[test]
    fn connecting_reads_the_statistics_and_stores_them() {
        let mut a = app();
        let (conn, commands) = connected_commands(&mut a);
        assert_eq!(stats_commands(&commands), 1);
        assert!(a.state.connection(conn).is_some_and(|c| c.stats_busy));
        a.update(Event::Background(Background::Stats {
            conn,
            result: Ok(sample_stats()),
        }));
        let c = a.state.connection(conn).expect("connection");
        assert!(!c.stats_busy && c.stats == Some(sample_stats()));
        a.update(Event::Background(Background::Stats {
            conn,
            result: Err(crate::db::DbError::new("denied")),
        }));
        let c = a.state.connection(conn).expect("connection");
        assert_eq!(c.stats_error.as_deref(), Some("denied"));
        assert!(c.stats.is_some(), "the last good numbers stay");
    }

    #[test]
    fn the_numbers_refresh_on_the_configured_schedule_only() {
        let mut config = Config::default();
        config.session.refresh_seconds = 5;
        let mut a = app_with(config);
        let (conn, _) = connected_commands(&mut a);
        // A read is in flight, then done. Not yet due.
        a.update(Event::Background(Background::Stats {
            conn,
            result: Ok(sample_stats()),
        }));
        assert_eq!(stats_commands(&a.update(Event::Tick)), 0);
        // Pretend the last read was long ago.
        a.state.connection_mut(conn).expect("c").stats_at =
            Some(Instant::now() - std::time::Duration::from_secs(6));
        assert_eq!(stats_commands(&a.update(Event::Tick)), 1);
        assert_eq!(stats_commands(&a.update(Event::Tick)), 0, "one at a time");
    }

    #[test]
    fn a_refresh_interval_of_zero_turns_the_automatic_refresh_off() {
        let mut config = Config::default();
        config.session.refresh_seconds = 0;
        let mut a = app_with(config);
        let (conn, _) = connected_commands(&mut a);
        a.update(Event::Background(Background::Stats {
            conn,
            result: Ok(sample_stats()),
        }));
        a.state.connection_mut(conn).expect("c").stats_at =
            Some(Instant::now() - std::time::Duration::from_secs(3600));
        assert_eq!(stats_commands(&a.update(Event::Tick)), 0);
        let commands = send(&mut a, KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(stats_commands(&commands), 1, "the key still works");
    }

    #[test]
    fn nothing_is_read_while_a_statement_runs_or_a_transaction_is_open() {
        let mut a = app();
        let (conn, _) = connected_commands(&mut a);
        a.update(Event::Background(Background::Stats {
            conn,
            result: Ok(sample_stats()),
        }));
        a.state.connection_mut(conn).expect("c").stats_at = None;
        a.state.connection_mut(conn).expect("c").tx = TxState::Open;
        assert_eq!(
            stats_commands(&a.update(Event::Tick)),
            0,
            "transaction open"
        );
        let commands = send(&mut a, KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(stats_commands(&commands), 0);
        assert!(a
            .state
            .notice
            .as_deref()
            .unwrap_or("")
            .contains("Cannot refresh"));
        a.state.connection_mut(conn).expect("c").tx = TxState::Idle;
        a.state.running = Some(Running {
            conn,
            doc: a.state.doc().id,
            started: Instant::now(),
        });
        assert_eq!(
            stats_commands(&a.update(Event::Tick)),
            0,
            "statement running"
        );
    }

    #[test]
    fn a_finished_statement_refreshes_the_numbers() {
        let mut a = app();
        let (conn, _) = connected_commands(&mut a);
        a.update(Event::Background(Background::Stats {
            conn,
            result: Ok(sample_stats()),
        }));
        let doc = a.state.doc().id;
        let commands = a.update(Event::Background(Background::QueryFinished {
            conn,
            doc,
            output: output(vec![], None),
        }));
        assert_eq!(stats_commands(&commands), 1);
    }

    #[test]
    fn a_cancel_during_a_statistics_read_waits_for_the_read() {
        let mut a = app();
        let (conn, _) = connected_commands(&mut a); // the read is in flight
        type_into(&mut a, "select pg_sleep(30)");
        let commands = press_run(&mut a);
        assert!(matches!(&commands[..], [Command::Run { .. }]));
        let commands = send(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(commands.is_empty(), "it would hit the statistics read");
        let commands = a.update(Event::Background(Background::Stats {
            conn,
            result: Ok(sample_stats()),
        }));
        assert!(commands.is_empty(), "the schema read is still going");
        let commands = a.update(Event::Background(Background::Catalog {
            conn,
            result: Ok(sample_catalog()),
        }));
        assert!(
            matches!(&commands[..], [Command::Cancel { .. }]),
            "{commands:?}"
        );
    }

    #[test]
    fn a_cancelled_statistics_read_is_not_reported() {
        let mut a = app();
        let (conn, _) = connected_commands(&mut a);
        let mut cancelled = crate::db::DbError::new("canceling statement due to user request");
        cancelled.sqlstate = Some("57014".to_string());
        a.update(Event::Background(Background::Stats {
            conn,
            result: Err(cancelled),
        }));
        let c = a.state.connection(conn).expect("c");
        assert!(c.stats_error.is_none() && !c.stats_busy);
    }

    #[test]
    fn oxpecker_starts_on_the_commands_pane() {
        let a = landing();
        assert_eq!(a.state.focus, Pane::Context);
        assert!(a.state.dialog.is_none());
    }

    #[test]
    fn without_the_context_pane_it_starts_in_the_editor() {
        let mut config = Config::default();
        config.ui.show_context_strip = false;
        let a = App::new(config, Keymap::default_keymap());
        assert_eq!(a.state.focus, Pane::Editor);
    }

    #[test]
    fn c_opens_the_connect_window_and_nothing_else_is_needed() {
        let mut a = landing();
        key(&mut a, 'c');
        assert!(matches!(a.state.dialog, Some(Dialog::Picker(_))));
    }

    #[test]
    fn letters_at_the_landing_screen_never_end_up_in_the_buffer() {
        let mut a = landing();
        for c in ['j', 'k', 'x', 'z', 'w', 'a'] {
            key(&mut a, c);
        }
        assert_eq!(a.state.editor().text(), "");
        assert_eq!(a.state.focus, Pane::Context);
    }

    #[test]
    fn q_quits_from_the_commands_pane_but_is_just_a_letter_in_the_editor() {
        let mut a = landing();
        key(&mut a, 'q');
        assert!(a.state.quit);

        let mut a = landing();
        key(&mut a, 'e');
        key(&mut a, 'q');
        assert!(!a.state.quit);
        assert_eq!(a.state.editor().text(), "q");
    }

    #[test]
    fn e_and_enter_go_to_the_editor() {
        for code in [KeyCode::Char('e'), KeyCode::Enter] {
            let mut a = landing();
            send(&mut a, code, KeyModifiers::NONE);
            assert_eq!(a.state.focus, Pane::Editor, "{code:?}");
            assert_eq!(a.state.editor().text(), "", "the key itself is not typed");
        }
    }

    #[test]
    fn n_opens_a_new_buffer_and_moves_to_it() {
        let mut a = landing();
        key(&mut a, 'n');
        assert_eq!(a.state.docs.len(), 2);
        assert_eq!(a.state.focus, Pane::Editor);
    }

    #[test]
    fn question_mark_opens_help_at_the_landing_screen() {
        let mut a = landing();
        key(&mut a, '?');
        assert!(a.state.help_open);
    }

    #[test]
    fn escape_leaves_the_editor_and_e_comes_back_with_the_text_intact() {
        let mut a = landing();
        key(&mut a, 'e');
        type_into(&mut a, "select 1");
        esc(&mut a);
        assert_eq!(a.state.focus, Pane::Context);
        key(&mut a, 'e');
        assert_eq!(a.state.focus, Pane::Editor);
        assert_eq!(a.state.editor().text(), "select 1");
    }

    #[test]
    fn escape_first_closes_a_suggestion_popup_and_only_then_leaves() {
        let mut a = landing();
        key(&mut a, 'e');
        type_into(&mut a, "sel");
        assert!(a.state.completion.is_some());
        esc(&mut a);
        assert_eq!(
            (a.state.focus, a.state.completion.is_some()),
            (Pane::Editor, false)
        );
        esc(&mut a);
        assert_eq!(a.state.focus, Pane::Context);
    }

    #[test]
    fn escape_shows_the_commands_pane_even_when_it_was_hidden() {
        let mut a = app();
        press(&mut a, KeyCode::F(3), KeyModifiers::NONE);
        assert!(!a.state.show_context);
        esc(&mut a);
        assert!(a.state.show_context);
        assert_eq!(a.state.focus, Pane::Context);
    }

    #[test]
    fn the_results_pane_has_a_few_single_keys_too() {
        let mut a = landing();
        alt(&mut a, '3');
        key(&mut a, 'c');
        assert!(matches!(a.state.dialog, Some(Dialog::Picker(_))));
        esc(&mut a);
        key(&mut a, 'e');
        assert_eq!(a.state.focus, Pane::Editor);
        alt(&mut a, '3');
        key(&mut a, 'q');
        assert!(a.state.quit);
    }

    #[test]
    fn connecting_moves_you_to_the_editor() {
        let mut a = landing();
        key(&mut a, 'c');
        type_into(&mut a, "postgres://alice:pw@127.0.0.1/app");
        let Command::Connect { conn, .. } =
            send(&mut a, KeyCode::Enter, KeyModifiers::NONE)[0].clone()
        else {
            panic!("connect")
        };
        assert_eq!(
            a.state.focus,
            Pane::Context,
            "still on the commands pane while connecting"
        );
        a.update(Event::Background(Background::Connected {
            conn,
            info: info(),
        }));
        assert_eq!(a.state.focus, Pane::Editor);
    }

    #[test]
    fn a_failed_connection_keeps_you_on_the_commands_pane() {
        let mut a = landing();
        key(&mut a, 'c');
        type_into(&mut a, "postgres://alice:pw@127.0.0.1/app");
        let Command::Connect { conn, .. } =
            send(&mut a, KeyCode::Enter, KeyModifiers::NONE)[0].clone()
        else {
            panic!("connect")
        };
        a.update(Event::Background(Background::ConnectFailed {
            conn,
            error: DbError::new("refused"),
        }));
        assert_eq!(a.state.focus, Pane::Context);
    }

    #[test]
    fn hints_follow_the_focus() {
        let mut a = landing();
        assert!(a.hint_actions().contains(&Action::ConnectionOpen));
        assert_eq!(
            a.hint_key(Action::ConnectionOpen)
                .map(|k| k.to_string())
                .as_deref(),
            Some("c")
        );
        key(&mut a, 'e');
        assert_eq!(a.hint_actions()[0], Action::QueryRun);
        assert_eq!(
            a.hint_key(Action::QueryRun)
                .map(|k| k.to_string())
                .as_deref(),
            Some("Ctrl+R")
        );
        assert_eq!(
            a.hint_key(Action::FocusCommands)
                .map(|k| k.to_string())
                .as_deref(),
            Some("Esc")
        );
    }

    // -- the connect window ----------------------------------------------------

    fn open_form(app: &mut App) {
        key(app, 'c');
        // With no saved profiles the only item is "New connection".
        send(app, KeyCode::Enter, KeyModifiers::NONE);
    }

    fn form(app: &App) -> &ConnectForm {
        match &app.state.dialog {
            Some(Dialog::Form(f)) => f,
            other => panic!("expected the form, got {other:?}"),
        }
    }

    #[test]
    fn a_saved_connection_can_be_edited_from_the_picker() {
        let mut a = landing();
        let mut saved = Profile::new("dev");
        saved.host = "old-host".to_string();
        saved.ssl_mode = profile::SslMode::Require;
        a.state.profiles.add(saved).expect("add");
        key(&mut a, 'c');
        send(&mut a, KeyCode::Char('e'), KeyModifiers::CONTROL);
        assert_eq!(form(&a).host, "old-host");
        assert_eq!(form(&a).name, "dev");
        send(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        type_into(&mut a, "new-host");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            matches!(&commands[..], [Command::SaveProfiles { .. }]),
            "saves without connecting: {commands:?}"
        );
        let p = a.state.profiles.get("dev").expect("still there");
        assert_eq!(p.host, "new-host");
        assert_eq!(
            p.ssl_mode,
            profile::SslMode::Require,
            "other options are kept"
        );
        assert!(matches!(a.state.dialog, Some(Dialog::Picker(_))));
    }

    #[test]
    fn the_first_choice_with_no_profiles_is_a_new_connection() {
        let mut a = landing();
        open_form(&mut a);
        assert_eq!(form(&a).host, "localhost");
        assert_eq!(form(&a).port, "5432");
    }

    #[test]
    fn filling_the_form_connects_and_then_asks_for_the_password() {
        let mut a = landing();
        open_form(&mut a);
        send(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL); // clear host
        type_into(&mut a, "db.example.com");
        send(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        send(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        type_into(&mut a, "6432");
        send(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        type_into(&mut a, "sales");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        let [Command::Connect {
            profile,
            password,
            lookup,
            ..
        }] = &commands[..]
        else {
            panic!("{commands:?}")
        };
        assert_eq!(
            (profile.host.as_str(), profile.port),
            ("db.example.com", 6432)
        );
        assert_eq!(profile.database.as_deref(), Some("sales"));
        assert!(
            password.is_none() && *lookup,
            "the password is looked up, then asked"
        );
        assert!(
            a.state.profiles.is_empty(),
            "nothing is saved without a name"
        );
    }

    #[test]
    fn a_name_saves_the_profile_without_any_password() {
        let mut a = landing();
        open_form(&mut a);
        for _ in 0..4 {
            send(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        }
        type_into(&mut a, "Local dev");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(
            &commands[..],
            [Command::SaveProfiles { .. }, Command::Connect { .. }]
        ));
        let Some(Command::SaveProfiles { contents }) = commands.first() else {
            panic!()
        };
        assert!(contents.contains("Local dev") && !contents.to_lowercase().contains("password ="));
        assert!(a.state.profiles.get("Local dev").is_some());
    }

    #[test]
    fn the_production_switch_marks_the_profile() {
        let mut a = landing();
        open_form(&mut a);
        send(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT); // wraps to the last field
        send(&mut a, KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(form(&a).production);
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        let [Command::Connect { profile, .. }] = &commands[..] else {
            panic!("{commands:?}")
        };
        assert!(profile.production);
    }

    #[test]
    fn a_bad_port_stays_in_the_form_with_a_message() {
        let mut a = landing();
        open_form(&mut a);
        send(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        send(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
        type_into(&mut a, "abc");
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(commands.is_empty());
        assert!(form(&a)
            .error
            .as_deref()
            .is_some_and(|e| e.contains("port")));
        type_into(&mut a, "1");
        assert_eq!(form(&a).error, None, "typing clears the message");
    }

    #[test]
    fn a_name_that_is_taken_is_refused_in_the_form() {
        let mut a = landing();
        a.state.profiles.add(Profile::new("taken")).expect("add");
        key(&mut a, 'c');
        send(&mut a, KeyCode::Down, KeyModifiers::NONE); // past "taken" to "New connection"
        send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        for _ in 0..4 {
            send(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        }
        type_into(&mut a, "taken");
        assert!(send(&mut a, KeyCode::Enter, KeyModifiers::NONE).is_empty());
        assert!(form(&a)
            .error
            .as_deref()
            .is_some_and(|e| e.contains("already exists")));
    }

    #[test]
    fn escape_in_the_form_goes_back_to_the_list() {
        let mut a = landing();
        open_form(&mut a);
        esc(&mut a);
        assert!(matches!(a.state.dialog, Some(Dialog::Picker(_))));
        esc(&mut a);
        assert!(a.state.dialog.is_none());
    }

    #[test]
    fn the_form_takes_every_key() {
        let mut a = landing();
        open_form(&mut a);
        type_into(&mut a, "q");
        assert!(!a.state.quit);
        assert_eq!(a.state.editor().text(), "");
        assert!(form(&a).host.ends_with('q'));
    }

    #[test]
    fn saved_profiles_come_before_new_connection_and_enter_picks_the_first() {
        let mut a = landing();
        a.state.profiles.add(Profile::new("alpha")).expect("add");
        key(&mut a, 'c');
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            matches!(commands.last(), Some(Command::Connect { profile, .. }) if profile.name == "alpha")
        );
    }

    #[test]
    fn a_lost_connection_is_offered_for_reconnecting_from_the_connect_window() {
        let mut a = landing();
        let conn = connected(&mut a);
        a.update(Event::Background(Background::ConnectionClosed {
            conn,
            reason: "gone".to_string(),
        }));
        esc(&mut a);
        key(&mut a, 'c');
        let commands = send(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            matches!(&commands[..], [Command::Connect { conn: c, lookup: true, .. }] if *c == conn),
            "{commands:?}"
        );
        assert!(a.state.dialog.is_none());
    }

    fn sample_catalog() -> Catalog {
        use crate::catalog::{Kind, Object};
        let object = |kind, schema: &str, name: &str, parent: Option<&str>| Object {
            kind,
            schema: schema.to_string(),
            name: name.to_string(),
            parent: parent.map(str::to_string),
            detail: String::new(),
        };
        Catalog {
            objects: vec![
                object(Kind::Table, "public", "orders", None),
                object(Kind::Table, "archive", "orders", None),
                object(Kind::Column, "public", "id", Some("orders")),
                object(Kind::Function, "public", "refresh", None),
            ],
        }
    }

    #[test]
    fn connecting_reads_the_schema_and_the_overlay_waits_for_it() {
        let mut a = app();
        let conn = connected(&mut a);
        assert!(a.state.connection(conn).is_some_and(|c| c.catalog_busy));
        ctrl(&mut a, 'p');
        assert!(matches!(a.state.dialog, Some(Dialog::Find(_))));
        assert!(a.find_items("").is_empty(), "still reading");
        a.update(Event::Background(Background::Catalog {
            conn,
            result: Ok(sample_catalog()),
        }));
        assert!(a.state.connection(conn).is_some_and(|c| !c.catalog_busy));
        assert_eq!(a.find_items("").len(), 3, "columns wait for a query");
        assert_eq!(a.find_items("orders.id").len(), 1);
    }

    #[test]
    fn find_needs_a_connection() {
        let mut a = app();
        ctrl(&mut a, 'p');
        assert!(a.state.dialog.is_none());
        assert!(a
            .state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Not connected")));
    }

    #[test]
    fn find_takes_typing_and_escape_changes_nothing() {
        let mut a = app();
        let conn = connected(&mut a);
        a.update(Event::Background(Background::Catalog {
            conn,
            result: Ok(sample_catalog()),
        }));
        type_into(&mut a, "x");
        ctrl(&mut a, 'p');
        type_into(&mut a, "orders");
        let Some(Dialog::Find(f)) = &a.state.dialog else {
            panic!("find")
        };
        assert_eq!(f.input, "orders");
        press(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(a.state.dialog.is_none());
        assert_eq!(a.state.editor().text(), "x");
    }

    #[test]
    fn enter_inserts_the_qualified_name_at_the_cursor() {
        let mut a = app();
        let conn = connected(&mut a);
        a.update(Event::Background(Background::Catalog {
            conn,
            result: Ok(sample_catalog()),
        }));
        type_into(&mut a, "select * from ");
        ctrl(&mut a, 'p');
        type_into(&mut a, "archive");
        press(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(a.state.dialog.is_none());
        assert_eq!(a.state.editor().text(), "select * from archive.orders");
    }

    #[test]
    fn alt_s_adds_a_sample_select_and_runs_it() {
        let mut a = app();
        let conn = connected(&mut a);
        a.update(Event::Background(Background::Catalog {
            conn,
            result: Ok(sample_catalog()),
        }));
        type_into(&mut a, "select 1;");
        ctrl(&mut a, 'p');
        type_into(&mut a, "public.orders");
        let commands = send(&mut a, KeyCode::Char('s'), KeyModifiers::ALT);
        assert_eq!(
            a.state.editor().text(),
            "select 1;\n\nSELECT * FROM public.orders LIMIT 100;"
        );
        assert!(matches!(
            &commands[..],
            [Command::Run { sql, .. }] if sql == "SELECT * FROM public.orders LIMIT 100"
        ));
    }

    #[test]
    fn alt_s_on_a_column_does_nothing() {
        let mut a = app();
        let conn = connected(&mut a);
        a.update(Event::Background(Background::Catalog {
            conn,
            result: Ok(sample_catalog()),
        }));
        ctrl(&mut a, 'p');
        type_into(&mut a, "orders.id");
        let commands = send(&mut a, KeyCode::Char('s'), KeyModifiers::ALT);
        assert!(commands.is_empty());
        assert!(matches!(a.state.dialog, Some(Dialog::Find(_))));
    }

    #[test]
    fn a_schema_change_reads_the_catalog_again() {
        let mut a = app();
        let conn = connected(&mut a);
        a.update(Event::Background(Background::Catalog {
            conn,
            result: Ok(sample_catalog()),
        }));
        type_into(&mut a, "create table t(a int)");
        let Command::Run { doc, .. } = press_run(&mut a)[0].clone() else {
            panic!("run")
        };
        let mut set = rows_set(&[]);
        set.columns.clear();
        set.tag = "CREATE TABLE".to_string();
        a.update(Event::Background(Background::QueryFinished {
            conn,
            doc,
            output: output(vec![set], None),
        }));
        assert!(a.state.connection(conn).is_some_and(|c| c.catalog_busy));
    }
}
