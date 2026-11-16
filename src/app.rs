//! Application state and the update function. Nothing here draws or does I/O.

use crossterm::event::KeyCode;
use ratatui::layout::Rect;

use crate::config::Config;
use crate::editor::{display_col, Buffer};
use crate::events::Event;
use crate::keymap::{Action, KeyChord, Keymap, Scope};
use crate::layout;

/// A top-level pane that can hold focus. The status bar never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Editor,
    Context,
    Results,
}

impl Pane {
    pub fn label(self) -> &'static str {
        match self {
            Pane::Editor => "Editor",
            Pane::Context => "Context",
            Pane::Results => "Results",
        }
    }

    pub fn scope(self) -> Scope {
        match self {
            Pane::Editor => Scope::Editor,
            Pane::Context => Scope::Context,
            Pane::Results => Scope::Results,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Text from the last cut or copy. The system clipboard comes later (R14).
    pub register: String,
}

/// One editor buffer with its view position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub name: String,
    pub buffer: Buffer,
    /// First visible line and first visible terminal column.
    pub scroll_top: usize,
    pub scroll_left: usize,
}

impl Document {
    fn new(name: String) -> Self {
        Self {
            name,
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

    pub fn editor(&self) -> &Buffer {
        &self.docs[self.active].buffer
    }

    pub fn editor_mut(&mut self) -> &mut Buffer {
        &mut self.docs[self.active].buffer
    }

    pub fn new(config: &Config) -> Self {
        Self {
            focus: Pane::Editor,
            show_context: config.ui.show_context_strip,
            show_results: true,
            help_open: false,
            help_scroll: 0,
            quit: false,
            notice: None,
            size: (0, 0),
            docs: vec![Document::new("Buffer 1".to_string())],
            active: 0,
            closed: Vec::new(),
            buffers_created: 1,
            register: String::new(),
        }
    }

    /// Panes in the order focus cycles through them (top to bottom).
    pub fn visible_panes(&self) -> Vec<Pane> {
        let mut panes = vec![Pane::Editor];
        if self.show_context {
            panes.push(Pane::Context);
        }
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
}

impl App {
    pub fn new(config: Config, keymap: Keymap) -> Self {
        Self {
            state: State::new(&config),
            keymap,
            config,
        }
    }

    pub fn update(&mut self, event: Event) {
        match event {
            Event::Resize(w, h) => self.state.size = (w, h),
            Event::Key(key) => self.on_key(KeyChord::from_event(&key)),
            Event::Paste(text) => self.on_paste(&text),
        }
        self.scroll_to_cursor();
    }

    fn on_key(&mut self, chord: KeyChord) {
        let st = &self.state;
        // A dialog takes every key.
        if st.help_open {
            if let Some(action) = self.keymap.lookup(Scope::Dialog, chord) {
                self.perform(action);
            }
            return;
        }
        // The focused pane's scope wins over global. A pane-scoped key never
        // fires in another pane (R4.8). With tab-indent off, Tab skips the
        // editor scope and reaches the global focus binding (R4.6).
        let editor_ignores_tab = st.focus == Pane::Editor
            && !self.config.editor.tab_indent
            && matches!(chord.code, KeyCode::Tab | KeyCode::BackTab);
        let pane_action = if editor_ignores_tab {
            None
        } else {
            self.keymap.lookup(st.focus.scope(), chord)
        };
        let action = pane_action.or_else(|| self.keymap.lookup(Scope::Global, chord));

        match action {
            Some(action) => self.perform(action),
            // R4.3: in the editor, any printable key without a binding types.
            None if st.focus == Pane::Editor && chord.is_printable() => {
                if let KeyCode::Char(c) = chord.code {
                    let opts = self.config.editor.options();
                    self.state.notice = None;
                    self.state.editor_mut().insert_char(c, &opts);
                }
            }
            None => {}
        }
    }

    /// Bracketed paste. The text is inserted and never executed (R20.10).
    fn on_paste(&mut self, text: &str) {
        if self.state.help_open || self.state.focus != Pane::Editor {
            return;
        }
        let opts = self.config.editor.options();
        self.state.notice = None;
        self.state.editor_mut().insert_str(text, &opts);
    }

    /// True while the editor uses Tab and Shift+Tab for indenting.
    fn editor_owns_tab(&self) -> bool {
        self.state.focus == Pane::Editor && self.config.editor.tab_indent
    }

    /// The key to show for `action` in the focused pane, for hints.
    pub fn hint_key(&self, action: Action) -> Option<KeyChord> {
        let scopes = [self.state.focus.scope(), Scope::Global];
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
        st.docs
            .push(Document::new(format!("Buffer {}", st.buffers_created)));
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
            st.docs
                .push(Document::new(format!("Buffer {}", st.buffers_created)));
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
            Action::FocusContext => {
                self.state.show_context = true;
                self.state.focus = Pane::Context;
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

    fn app_with(config: Config) -> App {
        App::new(config, Keymap::default_keymap())
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
        assert_eq!(a.state.focus, Pane::Context, "F6 always leaves the editor");
    }

    #[test]
    fn tab_cycles_focus_when_tab_indent_is_off() {
        let mut config = Config::default();
        config.editor.tab_indent = false;
        let mut a = app_with(config);
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Context);
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Results);
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Editor);
        press(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(a.state.focus, Pane::Results);
    }

    #[test]
    fn tab_cycles_from_other_panes_even_with_tab_indent() {
        let mut a = app();
        alt(&mut a, '1');
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Results);
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
        for c in ['q', 'j', 'x', ' '] {
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
}
