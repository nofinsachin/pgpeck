//! Application state and the update function. Nothing here draws or does I/O.

use crossterm::event::KeyCode;

use crate::config::Config;
use crate::events::Event;
use crate::keymap::{Action, KeyChord, Keymap, Scope};

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
    pub quit: bool,
    /// One-line message for the status bar.
    pub notice: Option<String>,
    pub size: (u16, u16),
}

impl State {
    pub fn new(config: &Config) -> Self {
        Self {
            focus: Pane::Editor,
            show_context: config.ui.show_context_strip,
            show_results: true,
            help_open: false,
            quit: false,
            notice: None,
            size: (0, 0),
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
        }
    }

    fn on_key(&mut self, chord: KeyChord) {
        let st = &self.state;
        if !st.help_open && self.tab_is_text(&chord) {
            return;
        }

        // A dialog takes every key. Otherwise the focused pane's scope wins
        // over global. A pane-scoped key never fires in another pane (R4.8).
        let action = if st.help_open {
            self.keymap.lookup(Scope::Dialog, chord)
        } else {
            self.keymap
                .lookup(st.focus.scope(), chord)
                .or_else(|| self.keymap.lookup(Scope::Global, chord))
        };
        if let Some(action) = action {
            self.perform(action);
        }
    }

    /// R4.6: Tab belongs to the editor when tab-indent is on. Other keys
    /// (Alt+1/2/3, F6) always leave the editor (R4.7).
    fn tab_is_text(&self, chord: &KeyChord) -> bool {
        self.state.focus == Pane::Editor
            && self.config.editor.tab_indent
            && matches!(chord.code, KeyCode::Tab | KeyCode::BackTab)
    }

    /// The key to show for `action` in the focused pane, for hints.
    pub fn hint_key(&self, action: Action) -> Option<KeyChord> {
        let scopes = [self.state.focus.scope(), Scope::Global];
        self.keymap
            .key_for(action, &scopes, |k| !self.tab_is_text(k))
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
            Action::HelpToggle => self.state.help_open = !self.state.help_open,
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
    fn tab_is_text_in_editor_when_tab_indent_is_on() {
        let mut a = app();
        press(&mut a, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(a.state.focus, Pane::Editor);
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

    #[test]
    fn printable_keys_do_nothing_here() {
        let mut a = app();
        let before = a.state.clone();
        for c in ['q', 'j', 'x', ' '] {
            press(&mut a, KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert_eq!(a.state, before);
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
