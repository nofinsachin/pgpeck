//! Keys map to named actions, per scope. Keymaps are data (R5).
//!
//! The UI never hardcodes key names. It asks the keymap.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::diag::{line_containing, line_of_offset, Diagnostic};
use crate::editor::{EditCmd, Motion};

const DEFAULT_KEYMAP: &str = include_str!("../config/keymap.default.toml");

// ---------------------------------------------------------------------------
// Actions and scopes
// ---------------------------------------------------------------------------

/// A named operation. Names are stable public identifiers (`area.verb_object`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    AppQuit,
    FocusContext,
    FocusEditor,
    FocusResults,
    FocusNext,
    FocusPrev,
    /// Leaves the editor for the commands pane, where single keys work.
    FocusCommands,
    ToggleContext,
    ToggleResults,
    HelpToggle,
    HelpScrollUp,
    HelpScrollDown,
    HelpPageUp,
    HelpPageDown,
    BufferNew,
    BufferClose,
    BufferNext,
    BufferPrev,
    BufferReopen,
    ConnectionOpen,
    ConnectionReconnect,
    QueryRun,
    QueryCancel,
    ResultsUp,
    ResultsDown,
    ResultsPageUp,
    ResultsPageDown,
    ResultsFirstRow,
    ResultsLastRow,
    ResultsLeft,
    ResultsRight,
    ResultsPrevTab,
    ResultsNextTab,
    DialogNext,
    DialogPrev,
    DialogSubmit,
    DialogCancel,
    DialogBackspace,
    DialogClear,
    PickerSaveProfile,
    CompletionTrigger,
    CompletionAccept,
    CompletionDismiss,
    CompletionNext,
    CompletionPrev,
    Edit(EditCmd),
}

const ACTIONS: &[(&str, Action)] = &[
    ("app.quit", Action::AppQuit),
    ("focus.context", Action::FocusContext),
    ("focus.editor", Action::FocusEditor),
    ("focus.results", Action::FocusResults),
    ("focus.next", Action::FocusNext),
    ("focus.prev", Action::FocusPrev),
    ("focus.commands", Action::FocusCommands),
    ("view.toggle_context", Action::ToggleContext),
    ("view.toggle_results", Action::ToggleResults),
    ("help.toggle", Action::HelpToggle),
    ("help.scroll_up", Action::HelpScrollUp),
    ("help.scroll_down", Action::HelpScrollDown),
    ("help.page_up", Action::HelpPageUp),
    ("help.page_down", Action::HelpPageDown),
    ("buffer.new", Action::BufferNew),
    ("buffer.close", Action::BufferClose),
    ("buffer.next", Action::BufferNext),
    ("buffer.prev", Action::BufferPrev),
    ("buffer.reopen", Action::BufferReopen),
    ("connection.open", Action::ConnectionOpen),
    ("connection.reconnect", Action::ConnectionReconnect),
    ("query.run_statement", Action::QueryRun),
    ("query.cancel", Action::QueryCancel),
    ("results.scroll_up", Action::ResultsUp),
    ("results.scroll_down", Action::ResultsDown),
    ("results.page_up", Action::ResultsPageUp),
    ("results.page_down", Action::ResultsPageDown),
    ("results.first_row", Action::ResultsFirstRow),
    ("results.last_row", Action::ResultsLastRow),
    ("results.scroll_left", Action::ResultsLeft),
    ("results.scroll_right", Action::ResultsRight),
    ("results.prev_tab", Action::ResultsPrevTab),
    ("results.next_tab", Action::ResultsNextTab),
    ("dialog.next", Action::DialogNext),
    ("dialog.prev", Action::DialogPrev),
    ("dialog.submit", Action::DialogSubmit),
    ("dialog.cancel", Action::DialogCancel),
    ("dialog.backspace", Action::DialogBackspace),
    ("dialog.clear", Action::DialogClear),
    ("picker.save_profile", Action::PickerSaveProfile),
    ("completion.trigger", Action::CompletionTrigger),
    ("completion.accept", Action::CompletionAccept),
    ("completion.dismiss", Action::CompletionDismiss),
    ("completion.next", Action::CompletionNext),
    ("completion.prev", Action::CompletionPrev),
    (
        "editor.move_left",
        Action::Edit(EditCmd::Move(Motion::Left)),
    ),
    (
        "editor.move_right",
        Action::Edit(EditCmd::Move(Motion::Right)),
    ),
    ("editor.move_up", Action::Edit(EditCmd::Move(Motion::Up))),
    (
        "editor.move_down",
        Action::Edit(EditCmd::Move(Motion::Down)),
    ),
    (
        "editor.move_line_start",
        Action::Edit(EditCmd::Move(Motion::LineStart)),
    ),
    (
        "editor.move_line_end",
        Action::Edit(EditCmd::Move(Motion::LineEnd)),
    ),
    (
        "editor.move_word_left",
        Action::Edit(EditCmd::Move(Motion::WordLeft)),
    ),
    (
        "editor.move_word_right",
        Action::Edit(EditCmd::Move(Motion::WordRight)),
    ),
    (
        "editor.move_page_up",
        Action::Edit(EditCmd::Move(Motion::PageUp)),
    ),
    (
        "editor.move_page_down",
        Action::Edit(EditCmd::Move(Motion::PageDown)),
    ),
    (
        "editor.move_buffer_start",
        Action::Edit(EditCmd::Move(Motion::BufferStart)),
    ),
    (
        "editor.move_buffer_end",
        Action::Edit(EditCmd::Move(Motion::BufferEnd)),
    ),
    (
        "editor.select_left",
        Action::Edit(EditCmd::Select(Motion::Left)),
    ),
    (
        "editor.select_right",
        Action::Edit(EditCmd::Select(Motion::Right)),
    ),
    (
        "editor.select_up",
        Action::Edit(EditCmd::Select(Motion::Up)),
    ),
    (
        "editor.select_down",
        Action::Edit(EditCmd::Select(Motion::Down)),
    ),
    (
        "editor.select_line_start",
        Action::Edit(EditCmd::Select(Motion::LineStart)),
    ),
    (
        "editor.select_line_end",
        Action::Edit(EditCmd::Select(Motion::LineEnd)),
    ),
    (
        "editor.select_word_left",
        Action::Edit(EditCmd::Select(Motion::WordLeft)),
    ),
    (
        "editor.select_word_right",
        Action::Edit(EditCmd::Select(Motion::WordRight)),
    ),
    (
        "editor.select_page_up",
        Action::Edit(EditCmd::Select(Motion::PageUp)),
    ),
    (
        "editor.select_page_down",
        Action::Edit(EditCmd::Select(Motion::PageDown)),
    ),
    (
        "editor.select_buffer_start",
        Action::Edit(EditCmd::Select(Motion::BufferStart)),
    ),
    (
        "editor.select_buffer_end",
        Action::Edit(EditCmd::Select(Motion::BufferEnd)),
    ),
    ("editor.select_all", Action::Edit(EditCmd::SelectAll)),
    ("editor.backspace", Action::Edit(EditCmd::Backspace)),
    ("editor.delete", Action::Edit(EditCmd::Delete)),
    ("editor.newline", Action::Edit(EditCmd::Newline)),
    ("editor.indent", Action::Edit(EditCmd::Indent)),
    ("editor.outdent", Action::Edit(EditCmd::Outdent)),
    ("editor.undo", Action::Edit(EditCmd::Undo)),
    ("editor.redo", Action::Edit(EditCmd::Redo)),
    ("editor.cut", Action::Edit(EditCmd::Cut)),
    ("editor.copy", Action::Edit(EditCmd::Copy)),
    ("editor.paste", Action::Edit(EditCmd::Paste)),
];

impl Action {
    pub fn name(self) -> &'static str {
        ACTIONS
            .iter()
            .find(|(_, a)| *a == self)
            .map(|(n, _)| *n)
            .unwrap_or("unknown")
    }

    pub fn from_name(name: &str) -> Option<Action> {
        ACTIONS.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
    }

    /// Completion actions leave the popup state to their own handling.
    pub fn is_completion(self) -> bool {
        matches!(
            self,
            Action::CompletionTrigger
                | Action::CompletionAccept
                | Action::CompletionDismiss
                | Action::CompletionNext
                | Action::CompletionPrev
        )
    }

    /// Short description for the hint bar and help overlay.
    pub fn description(self) -> &'static str {
        match self {
            Action::AppQuit => "quit",
            Action::FocusContext => "focus context",
            Action::FocusCommands => "commands",
            Action::FocusEditor => "edit",
            Action::FocusResults => "focus results",
            Action::FocusNext => "next pane",
            Action::FocusPrev => "previous pane",
            Action::ToggleContext => "toggle context strip",
            Action::ToggleResults => "toggle results",
            Action::HelpToggle => "help",
            Action::HelpScrollUp => "scroll up",
            Action::HelpScrollDown => "scroll down",
            Action::HelpPageUp => "page up",
            Action::HelpPageDown => "page down",
            Action::BufferNew => "new buffer",
            Action::BufferClose => "close buffer",
            Action::BufferNext => "next buffer",
            Action::BufferPrev => "previous buffer",
            Action::BufferReopen => "reopen closed buffer",
            Action::ConnectionOpen => "connect",
            Action::ConnectionReconnect => "reconnect",
            Action::QueryRun => "run",
            Action::QueryCancel => "cancel query",
            Action::ResultsUp => "row up",
            Action::ResultsDown => "row down",
            Action::ResultsPageUp => "page up",
            Action::ResultsPageDown => "page down",
            Action::ResultsFirstRow => "first row",
            Action::ResultsLastRow => "last row",
            Action::ResultsLeft => "columns left",
            Action::ResultsRight => "columns right",
            Action::ResultsPrevTab => "previous result",
            Action::ResultsNextTab => "next result",
            Action::DialogNext => "next item",
            Action::DialogPrev => "previous item",
            Action::DialogSubmit => "confirm",
            Action::DialogCancel => "cancel",
            Action::DialogBackspace => "delete character",
            Action::DialogClear => "clear",
            Action::PickerSaveProfile => "save as profile",
            Action::CompletionTrigger => "show suggestions",
            Action::CompletionAccept => "accept suggestion",
            Action::CompletionDismiss => "dismiss suggestions",
            Action::CompletionNext => "next suggestion",
            Action::CompletionPrev => "previous suggestion",
            Action::Edit(cmd) => cmd.description(),
        }
    }
}

/// Where a binding applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    Global,
    Editor,
    Context,
    Results,
    /// Active only while the suggestion popup is open.
    Completion,
    /// The connection picker.
    Picker,
    /// The form for entering connection details.
    Form,
    /// The password prompt.
    Prompt,
    Dialog,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::Editor => "editor",
            Scope::Context => "context",
            Scope::Results => "results",
            Scope::Completion => "completion",
            Scope::Picker => "picker",
            Scope::Form => "form",
            Scope::Prompt => "prompt",
            Scope::Dialog => "dialog",
        }
    }

    fn from_name(name: &str) -> Option<Scope> {
        [
            Scope::Global,
            Scope::Editor,
            Scope::Context,
            Scope::Results,
            Scope::Completion,
            Scope::Picker,
            Scope::Form,
            Scope::Prompt,
            Scope::Dialog,
        ]
        .into_iter()
        .find(|s| s.name() == name)
    }

    /// R4.4: the editor must always accept printable characters, and global
    /// bindings are active while it has focus.
    fn forbids_printable(self) -> bool {
        matches!(
            self,
            Scope::Global
                | Scope::Editor
                | Scope::Completion
                | Scope::Picker
                | Scope::Form
                | Scope::Prompt
        )
    }
}

// ---------------------------------------------------------------------------
// Key chords
// ---------------------------------------------------------------------------

/// A key plus modifiers, in normalized form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl KeyChord {
    /// Normalization: `Shift` on a character is folded into the character
    /// (`shift+a` and `A` are the same chord).
    fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        match code {
            KeyCode::Char(c) if mods.contains(KeyModifiers::SHIFT) => KeyChord {
                code: KeyCode::Char(c.to_ascii_uppercase()),
                mods: mods - KeyModifiers::SHIFT,
            },
            // Letters with Ctrl, Alt or Super ignore case ("Ctrl+X" is "ctrl+x").
            KeyCode::Char(c)
                if mods.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                KeyChord {
                    code: KeyCode::Char(c.to_ascii_lowercase()),
                    mods,
                }
            }
            // Terminals report Shift+Tab as BackTab, sometimes with SHIFT set.
            KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => KeyChord {
                code: KeyCode::BackTab,
                mods: KeyModifiers::SHIFT,
            },
            KeyCode::BackTab => KeyChord {
                code,
                mods: KeyModifiers::SHIFT | (mods - KeyModifiers::SHIFT),
            },
            _ => KeyChord { code, mods },
        }
    }

    pub fn from_event(ev: &KeyEvent) -> Self {
        Self::new(ev.code, ev.modifiers)
    }

    /// True when pressing the chord types a character into a text field.
    pub fn is_printable(&self) -> bool {
        matches!(self.code, KeyCode::Char(_))
            && !self
                .mods
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    }

    pub fn parse(text: &str) -> Result<KeyChord, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("empty key".to_string());
        }
        // "+" and "ctrl++" name the plus key. Otherwise the last "+" splits
        // modifiers from the key.
        let (mod_part, key_part) = if text == "+" {
            ("", "+")
        } else if let Some(mods) = text.strip_suffix("++") {
            (mods, "+")
        } else {
            match text.rfind('+') {
                Some(i) => (&text[..i], &text[i + 1..]),
                None => ("", text),
            }
        };

        let mut mods = KeyModifiers::NONE;
        if !mod_part.is_empty() {
            for m in mod_part.split('+') {
                mods |= match m.to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => KeyModifiers::CONTROL,
                    "alt" | "option" | "meta" => KeyModifiers::ALT,
                    "shift" => KeyModifiers::SHIFT,
                    "super" | "cmd" => KeyModifiers::SUPER,
                    other => return Err(format!("unknown modifier `{other}`")),
                };
            }
        }

        let lower = key_part.to_ascii_lowercase();
        let code = match lower.as_str() {
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "space" => KeyCode::Char(' '),
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "insert" => KeyCode::Insert,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" => KeyCode::PageUp,
            "pagedown" => KeyCode::PageDown,
            f if f.starts_with('f')
                && f.len() > 1
                && f[1..].chars().all(|c| c.is_ascii_digit()) =>
            {
                match f[1..].parse::<u8>() {
                    Ok(n) if (1..=12).contains(&n) => KeyCode::F(n),
                    _ => return Err(format!("unknown key `{key_part}`")),
                }
            }
            _ => {
                let mut chars = key_part.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return Err(format!("unknown key `{key_part}`")),
                }
            }
        };
        Ok(KeyChord::new(code, mods))
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.contains(KeyModifiers::CONTROL) {
            write!(f, "Ctrl+")?;
        }
        if self.mods.contains(KeyModifiers::ALT) {
            write!(f, "Alt+")?;
        }
        if self.mods.contains(KeyModifiers::SUPER) {
            write!(f, "Super+")?;
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            write!(f, "Shift+")?;
        }
        match self.code {
            KeyCode::Char(' ') => write!(f, "Space"),
            KeyCode::Char(c) if self.mods.is_empty() => write!(f, "{c}"),
            KeyCode::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            KeyCode::BackTab => write!(f, "Tab"),
            KeyCode::Enter => write!(f, "Enter"),
            KeyCode::Esc => write!(f, "Esc"),
            KeyCode::Tab => write!(f, "Tab"),
            KeyCode::Backspace => write!(f, "Backspace"),
            KeyCode::Delete => write!(f, "Delete"),
            KeyCode::Insert => write!(f, "Insert"),
            KeyCode::Up => write!(f, "Up"),
            KeyCode::Down => write!(f, "Down"),
            KeyCode::Left => write!(f, "Left"),
            KeyCode::Right => write!(f, "Right"),
            KeyCode::Home => write!(f, "Home"),
            KeyCode::End => write!(f, "End"),
            KeyCode::PageUp => write!(f, "PageUp"),
            KeyCode::PageDown => write!(f, "PageDown"),
            KeyCode::F(n) => write!(f, "F{n}"),
            other => write!(f, "{other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Keymap
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Keymap {
    scopes: HashMap<Scope, HashMap<KeyChord, Action>>,
}

/// One parsed `"key" = "action"` line. `action` is `None` for `"none"`.
struct Entry {
    scope: Scope,
    chord: KeyChord,
    action: Option<Action>,
}

impl Keymap {
    /// The built-in keymap.
    pub fn default_keymap() -> Keymap {
        let (entries, diags) = parse_entries(DEFAULT_KEYMAP, "keymap.default.toml");
        debug_assert!(diags.is_empty(), "built-in keymap is invalid: {diags:?}");
        let mut keymap = Keymap::default();
        keymap.apply(entries);
        keymap
    }

    /// The built-in keymap with the user's file laid over it. Bad entries are
    /// refused and reported. The rest still apply.
    pub fn with_user(user_src: &str, file: &str) -> (Keymap, Vec<Diagnostic>) {
        let mut keymap = Self::default_keymap();
        let (entries, diags) = parse_entries(user_src, file);
        keymap.apply(entries);
        (keymap, diags)
    }

    fn apply(&mut self, entries: Vec<Entry>) {
        for e in entries {
            let scope = self.scopes.entry(e.scope).or_default();
            match e.action {
                Some(action) => {
                    scope.insert(e.chord, action);
                }
                None => {
                    scope.remove(&e.chord);
                }
            }
        }
    }

    /// The action bound to `chord` in `scope`, if any.
    pub fn lookup(&self, scope: Scope, chord: KeyChord) -> Option<Action> {
        self.scopes.get(&scope)?.get(&chord).copied()
    }

    /// Bindings of one scope, sorted by key for stable display.
    pub fn bindings(&self, scope: Scope) -> Vec<(KeyChord, Action)> {
        let mut list: Vec<_> = self
            .scopes
            .get(&scope)
            .map(|m| m.iter().map(|(k, a)| (*k, *a)).collect())
            .unwrap_or_default();
        list.sort_by_key(|(k, _)| k.to_string());
        list
    }

    /// The most convenient key for `action` in the given scopes. Fewest
    /// modifiers first, then not a function key, then the shortest name
    /// (`e` before `Enter`). `usable` can exclude keys that do not work in the
    /// current context.
    pub fn key_for(
        &self,
        action: Action,
        scopes: &[Scope],
        usable: impl Fn(&KeyChord) -> bool,
    ) -> Option<KeyChord> {
        scopes
            .iter()
            .flat_map(|s| self.bindings(*s))
            .filter(|(_, a)| *a == action)
            .map(|(k, _)| k)
            .filter(|k| usable(k))
            .min_by_key(|k| {
                let name = k.to_string();
                (
                    k.mods.bits().count_ones(),
                    matches!(k.code, KeyCode::F(_)),
                    name.chars().count(),
                    name,
                )
            })
    }
}

/// Parses a keymap file into entries, collecting every problem found.
fn parse_entries(src: &str, file: &str) -> (Vec<Entry>, Vec<Diagnostic>) {
    let mut entries = Vec::new();
    let mut diags = Vec::new();

    let table = match toml::from_str::<toml::Table>(src) {
        Ok(t) => t,
        Err(e) => {
            let line = e.span().map(|s| line_of_offset(src, s.start));
            diags.push(Diagnostic::new(file, line, e.message().to_string()));
            return (entries, diags);
        }
    };

    for (scope_name, body) in &table {
        let scope_line = line_containing(src, &format!("[{scope_name}"));
        let Some(scope) = Scope::from_name(scope_name) else {
            diags.push(Diagnostic::new(
                file,
                scope_line,
                format!("unknown scope `[{scope_name}]`"),
            ));
            continue;
        };
        let Some(body) = body.as_table() else {
            diags.push(Diagnostic::new(
                file,
                scope_line,
                format!("`{scope_name}` must be a table"),
            ));
            continue;
        };

        // Sorted by key text, so conflict reports are deterministic.
        let sorted: BTreeMap<_, _> = body.iter().collect();
        let mut seen: HashMap<KeyChord, &str> = HashMap::new();
        for (key_text, value) in sorted {
            let line = line_containing(src, &format!("\"{key_text}\""));
            let chord = match KeyChord::parse(key_text) {
                Ok(c) => c,
                Err(msg) => {
                    diags.push(Diagnostic::new(file, line, format!("[{scope_name}] {msg}")));
                    continue;
                }
            };
            let Some(action_name) = value.as_str() else {
                diags.push(Diagnostic::new(
                    file,
                    line,
                    format!("[{scope_name}] `{key_text}` needs an action name"),
                ));
                continue;
            };
            let action = if action_name == "none" {
                None
            } else {
                match Action::from_name(action_name) {
                    Some(a) => Some(a),
                    None => {
                        diags.push(Diagnostic::new(
                            file,
                            line,
                            format!("[{scope_name}] unknown action `{action_name}`"),
                        ));
                        continue;
                    }
                }
            };
            if action.is_some() && scope.forbids_printable() && chord.is_printable() {
                diags.push(Diagnostic::new(
                    file,
                    line,
                    format!(
                        "[{scope_name}] `{key_text}` types a character, so it cannot be bound here. \
                         Use a modifier or a function key"
                    ),
                ));
                continue;
            }
            if let Some(first) = seen.get(&chord) {
                diags.push(Diagnostic::new(
                    file,
                    line,
                    format!("[{scope_name}] `{key_text}` conflicts with `{first}`, which is the same key. Binding refused"),
                ));
                continue;
            }
            seen.insert(chord, key_text);
            entries.push(Entry {
                scope,
                chord,
                action,
            });
        }
    }
    (entries, diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> KeyChord {
        KeyChord::parse(s).expect("valid chord")
    }

    #[test]
    fn default_keymap_has_no_problems() {
        let (entries, diags) = parse_entries(DEFAULT_KEYMAP, "default");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(!entries.is_empty());
    }

    #[test]
    fn every_action_has_a_default_binding_and_a_unique_name() {
        let keymap = Keymap::default_keymap();
        let all = [
            Scope::Global,
            Scope::Editor,
            Scope::Context,
            Scope::Results,
            Scope::Completion,
            Scope::Picker,
            Scope::Form,
            Scope::Prompt,
            Scope::Dialog,
        ];
        for (name, action) in ACTIONS {
            assert_eq!(action.name(), *name);
            assert_eq!(Action::from_name(name), Some(*action));
            assert!(
                keymap.key_for(*action, &all, |_| true).is_some(),
                "{name} is unbound"
            );
        }
        let mut names: Vec<_> = ACTIONS.iter().map(|(n, _)| *n).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), ACTIONS.len());
    }

    #[test]
    fn chords_parse_and_display() {
        assert_eq!(chord("Ctrl+Q").to_string(), "Ctrl+Q");
        assert_eq!(chord("alt+1").to_string(), "Alt+1");
        assert_eq!(chord("shift+tab").to_string(), "Shift+Tab");
        assert_eq!(chord("f6").to_string(), "F6");
        assert_eq!(chord("?").to_string(), "?");
        assert_eq!(chord("ctrl+space").to_string(), "Ctrl+Space");
        assert_eq!(chord("ctrl++").code, KeyCode::Char('+'));
    }

    #[test]
    fn spelling_and_case_do_not_matter() {
        assert_eq!(chord("Ctrl+Enter"), chord("control+return"));
        assert_eq!(chord("shift+a"), chord("A"));
    }

    #[test]
    fn terminal_events_match_parsed_chords() {
        let ev = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(KeyChord::from_event(&ev), chord("shift+tab"));
        let ev = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT);
        assert_eq!(KeyChord::from_event(&ev), chord("A"));
    }

    #[test]
    fn bad_keys_are_rejected() {
        for bad in ["", "ctrl+", "hyper+x", "f13", "ab", "ctrl+foo"] {
            assert!(KeyChord::parse(bad).is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn printable_keys_cannot_be_bound_in_editor_or_global() {
        let src = "[editor]\n\"j\" = \"app.quit\"\n[global]\n\"?\" = \"help.toggle\"\n\"ctrl+j\" = \"app.quit\"\n";
        let (entries, diags) = parse_entries(src, "k.toml");
        assert_eq!(diags.len(), 2, "{diags:?}");
        assert_eq!(entries.len(), 1);
        assert!(diags
            .iter()
            .all(|d| d.message.contains("cannot be bound here")));
    }

    #[test]
    fn printable_keys_are_fine_in_other_panes() {
        let src = "[results]\n\"j\" = \"focus.next\"\n";
        let (entries, diags) = parse_entries(src, "k.toml");
        assert!(diags.is_empty());
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn conflicting_spellings_in_one_scope_are_refused() {
        let src = "[global]\n\"ctrl+x\" = \"app.quit\"\n\"Ctrl+X\" = \"focus.next\"\n";
        let (entries, diags) = parse_entries(src, "k.toml");
        assert_eq!(entries.len(), 1);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("conflicts"));
        assert_eq!(diags[0].line, Some(2), "reports the key that sorts second");
    }

    #[test]
    fn same_key_in_different_scopes_is_not_a_conflict() {
        let src = "[results]\n\"x\" = \"app.quit\"\n[context]\n\"x\" = \"focus.next\"\n";
        let (_, diags) = parse_entries(src, "k.toml");
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn unknown_scope_action_and_key_are_reported_with_lines() {
        let src = "[nope]\n\"a\" = \"app.quit\"\n[results]\n\"x\" = \"does.not_exist\"\n\"bad+\" = \"app.quit\"\n";
        let (entries, diags) = parse_entries(src, "k.toml");
        assert!(entries.is_empty());
        assert_eq!(diags.len(), 3, "{diags:?}");
        assert!(diags.iter().all(|d| d.line.is_some()));
    }

    #[test]
    fn user_binding_replaces_default_and_none_unbinds() {
        let src =
            "[global]\n\"ctrl+q\" = \"focus.next\"\n\"f3\" = \"none\"\n\"ctrl+e\" = \"app.quit\"\n";
        let (keymap, diags) = Keymap::with_user(src, "keymap.toml");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(
            keymap.lookup(Scope::Global, chord("ctrl+q")),
            Some(Action::FocusNext)
        );
        assert_eq!(keymap.lookup(Scope::Global, chord("f3")), None);
        assert_eq!(
            keymap.lookup(Scope::Global, chord("ctrl+e")),
            Some(Action::AppQuit)
        );
        // Untouched defaults remain.
        assert_eq!(
            keymap.lookup(Scope::Global, chord("f6")),
            Some(Action::FocusNext)
        );
    }

    #[test]
    fn bad_user_entries_are_refused_but_good_ones_apply() {
        let src = "[global]\n\"j\" = \"app.quit\"\n\"ctrl+e\" = \"app.quit\"\n";
        let (keymap, diags) = Keymap::with_user(src, "keymap.toml");
        assert_eq!(diags.len(), 1);
        assert_eq!(
            keymap.lookup(Scope::Global, chord("ctrl+e")),
            Some(Action::AppQuit)
        );
        assert_eq!(keymap.lookup(Scope::Global, chord("j")), None);
    }

    #[test]
    fn syntax_error_keeps_defaults() {
        let (keymap, diags) = Keymap::with_user("[global\n", "keymap.toml");
        assert_eq!(diags.len(), 1);
        assert_eq!(
            keymap.lookup(Scope::Global, chord("ctrl+q")),
            Some(Action::AppQuit)
        );
    }

    #[test]
    fn key_for_prefers_few_modifiers_then_plain_keys_then_short_names() {
        let keymap = Keymap::default_keymap();
        assert_eq!(
            keymap.key_for(Action::FocusNext, &[Scope::Global], |_| true),
            Some(chord("tab"))
        );
        assert_eq!(
            keymap.key_for(Action::FocusEditor, &[Scope::Global], |_| true),
            Some(chord("alt+2"))
        );
    }
}
