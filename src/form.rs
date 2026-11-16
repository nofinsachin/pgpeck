//! The "new connection" form: connection details typed by hand. There is no
//! password field, because oxpecker never stores one and asks when it
//! connects. Pure logic.

use crate::profile::{Profile, SslMode};

pub const LABELS: [&str; 7] = [
    "Host",
    "Port",
    "Database",
    "User",
    "Save as",
    "SSL mode",
    "Production",
];
const SSL_MODE: usize = 5;
const PRODUCTION: usize = 6;
const SSL_MODES: [SslMode; 5] = [
    SslMode::Disable,
    SslMode::Prefer,
    SslMode::Require,
    SslMode::VerifyCa,
    SslMode::VerifyFull,
];

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectForm {
    pub host: String,
    pub port: String,
    pub database: String,
    pub user: String,
    /// Saves the connection as a profile under this name. Empty means do not save.
    pub name: String,
    pub ssl_mode: SslMode,
    pub production: bool,
    /// Index into `LABELS`.
    pub focus: usize,
    pub error: Option<String>,
    /// The saved profile being edited. `None` for a new connection.
    pub editing: Option<Box<Profile>>,
}

impl ConnectForm {
    /// `user` is the suggestion for the login name, usually the OS user.
    pub fn new(user: &str) -> Self {
        Self {
            host: "localhost".to_string(),
            port: "5432".to_string(),
            database: String::new(),
            user: user.to_string(),
            name: String::new(),
            ssl_mode: SslMode::default(),
            production: false,
            focus: 0,
            error: None,
            editing: None,
        }
    }

    /// A form filled in from a saved profile. Options the form has no field
    /// for (TLS mode, color, ...) are kept when the profile is built again.
    pub fn edit(profile: &Profile) -> Self {
        Self {
            host: profile.host.clone(),
            port: profile.port.to_string(),
            database: profile.database.clone().unwrap_or_default(),
            user: profile.user.clone().unwrap_or_default(),
            name: profile.name.clone(),
            ssl_mode: profile.ssl_mode,
            production: profile.production,
            focus: 0,
            error: None,
            editing: Some(Box::new(profile.clone())),
        }
    }

    /// True when the focused field is a choice, not text.
    pub fn on_toggle(&self) -> bool {
        self.focus >= SSL_MODE
    }

    /// Moves the SSL mode to the next (or previous) choice.
    fn cycle_ssl_mode(&mut self, step: usize) {
        let at = SSL_MODES
            .iter()
            .position(|m| *m == self.ssl_mode)
            .unwrap_or(0);
        self.ssl_mode = SSL_MODES[(at + step) % SSL_MODES.len()];
    }

    /// The text of field `index`, or `None` for the toggle.
    pub fn text(&self, index: usize) -> Option<&str> {
        match index {
            0 => Some(&self.host),
            1 => Some(&self.port),
            2 => Some(&self.database),
            3 => Some(&self.user),
            4 => Some(&self.name),
            _ => None,
        }
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            0 => Some(&mut self.host),
            1 => Some(&mut self.port),
            2 => Some(&mut self.database),
            3 => Some(&mut self.user),
            4 => Some(&mut self.name),
            _ => None,
        }
    }

    /// Types a character. On the toggle, Space flips it and other keys do nothing.
    pub fn type_char(&mut self, c: char) {
        self.error = None;
        if self.on_toggle() {
            if c == ' ' {
                if self.focus == SSL_MODE {
                    self.cycle_ssl_mode(1);
                } else if self.focus == PRODUCTION {
                    self.production = !self.production;
                }
            }
        } else if let Some(text) = self.text_mut() {
            text.push(c);
        }
    }

    pub fn backspace(&mut self) {
        self.error = None;
        if let Some(text) = self.text_mut() {
            text.pop();
        }
    }

    pub fn clear(&mut self) {
        self.error = None;
        if let Some(text) = self.text_mut() {
            text.clear();
        }
    }

    pub fn next_field(&mut self) {
        self.focus = (self.focus + 1) % LABELS.len();
    }

    pub fn previous_field(&mut self) {
        self.focus = (self.focus + LABELS.len() - 1) % LABELS.len();
    }

    /// Checks the details and makes a profile from them.
    pub fn build(&self) -> Result<Profile, String> {
        let name = self.name.trim();
        if self.editing.is_some() && name.is_empty() {
            return Err("A saved connection needs a name".to_string());
        }
        let mut profile = match &self.editing {
            Some(original) => {
                let mut p = (**original).clone();
                p.name = name.to_string();
                p
            }
            None => Profile::new(if name.is_empty() { "(manual)" } else { name }),
        };
        let host = self.host.trim();
        if host.is_empty() {
            return Err("Enter a host, for example localhost".to_string());
        }
        profile.host = host.to_string();
        profile.port = self
            .port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or("The port must be a number from 1 to 65535")?;
        let optional = |text: &str| Some(text.trim().to_string()).filter(|t| !t.is_empty());
        profile.database = optional(&self.database);
        profile.user = optional(&self.user);
        profile.ssl_mode = self.ssl_mode;
        profile.production = self.production;
        profile.validate()?;
        Ok(profile)
    }
}

/// Changes the database or the user of the current connection. The
/// connection is made again with the new values. Nothing is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionForm {
    pub database: String,
    pub user: String,
    /// 0 is the database, 1 is the user.
    pub focus: usize,
    pub error: Option<String>,
}

pub const SESSION_LABELS: [&str; 2] = ["Database", "User"];

impl SessionForm {
    pub fn new(database: &str, user: &str, focus: usize) -> Self {
        Self {
            database: database.to_string(),
            user: user.to_string(),
            focus: focus.min(1),
            error: None,
        }
    }

    pub fn text(&self, index: usize) -> &str {
        if index == 0 {
            &self.database
        } else {
            &self.user
        }
    }

    fn text_mut(&mut self) -> &mut String {
        if self.focus == 0 {
            &mut self.database
        } else {
            &mut self.user
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.error = None;
        self.text_mut().push(c);
    }

    pub fn backspace(&mut self) {
        self.error = None;
        self.text_mut().pop();
    }

    pub fn clear(&mut self) {
        self.error = None;
        self.text_mut().clear();
    }

    pub fn next_field(&mut self) {
        self.focus = (self.focus + 1) % 2;
    }

    /// The connection details with the new database and user.
    pub fn apply(&self, current: &Profile) -> Result<Profile, String> {
        let user = self.user.trim();
        if user.is_empty() {
            return Err("Enter a user".to_string());
        }
        let mut profile = current.clone();
        profile.user = Some(user.to_string());
        profile.database = Some(self.database.trim().to_string()).filter(|d| !d.is_empty());
        profile.validate()?;
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(form: &mut ConnectForm, text: &str) {
        text.chars().for_each(|c| form.type_char(c));
    }

    #[test]
    fn starts_with_sensible_defaults() {
        let form = ConnectForm::new("sachin");
        let profile = form.build().expect("valid");
        assert_eq!((profile.host.as_str(), profile.port), ("localhost", 5432));
        assert_eq!(profile.user.as_deref(), Some("sachin"));
        assert_eq!(
            profile.database, None,
            "an empty database means the same as the user"
        );
        assert!(!profile.production);
        assert_eq!(profile.name, "(manual)");
    }

    #[test]
    fn typing_goes_to_the_focused_field() {
        let mut form = ConnectForm::new("u");
        form.clear();
        typed(&mut form, "db.example.com");
        form.next_field();
        form.clear();
        typed(&mut form, "6432");
        form.next_field();
        typed(&mut form, "sales");
        form.next_field();
        form.clear();
        typed(&mut form, "alice");
        form.next_field();
        typed(&mut form, "Sales DB");
        let profile = form.build().expect("valid");
        assert_eq!(
            (
                profile.host.as_str(),
                profile.port,
                profile.database.as_deref(),
                profile.user.as_deref()
            ),
            ("db.example.com", 6432, Some("sales"), Some("alice"))
        );
        assert_eq!(profile.name, "Sales DB");
    }

    #[test]
    fn backspace_and_clear_edit_only_the_focused_field() {
        let mut form = ConnectForm::new("u");
        form.backspace();
        assert_eq!(form.host, "localhos");
        form.clear();
        assert_eq!(form.host, "");
        assert_eq!(form.port, "5432");
    }

    #[test]
    fn focus_wraps_both_ways() {
        let mut form = ConnectForm::new("u");
        form.previous_field();
        assert!(form.on_toggle());
        form.next_field();
        assert_eq!(form.focus, 0);
    }

    #[test]
    fn space_flips_the_production_toggle_and_other_keys_do_nothing_there() {
        let mut form = ConnectForm::new("u");
        form.previous_field();
        assert_eq!(form.focus, PRODUCTION);
        form.type_char('x');
        assert!(!form.production);
        form.type_char(' ');
        assert!(form.production);
        assert!(form.build().expect("valid").production);
        form.type_char(' ');
        assert!(!form.production);
        form.backspace();
        form.clear();
    }

    #[test]
    fn spaces_in_text_fields_are_kept() {
        let mut form = ConnectForm::new("u");
        form.next_field();
        form.next_field();
        form.next_field();
        form.next_field();
        typed(&mut form, "my connection");
        assert_eq!(form.name, "my connection");
    }

    #[test]
    fn bad_details_are_explained() {
        let mut form = ConnectForm::new("u");
        form.clear();
        assert!(form.build().expect_err("host").contains("host"));
        form.host = "h".to_string();
        for bad in ["", "0", "65536", "abc", "-1"] {
            form.port = bad.to_string();
            assert!(form.build().expect_err(bad).contains("port"), "{bad}");
        }
        form.port = " 5433 ".to_string();
        assert_eq!(form.build().expect("trimmed").port, 5433);
    }

    #[test]
    fn typing_clears_the_previous_error() {
        let mut form = ConnectForm::new("u");
        form.error = Some("old".to_string());
        form.type_char('a');
        assert_eq!(form.error, None);
    }

    #[test]
    fn editing_fills_the_fields_and_keeps_other_options() {
        let mut saved = Profile::new("prod");
        saved.host = "db.example.com".to_string();
        saved.port = 6432;
        saved.database = Some("app".to_string());
        saved.user = Some("alice".to_string());
        saved.label = Some("Production DB".to_string());
        saved.last_used = Some(7);
        let mut form = ConnectForm::edit(&saved);
        assert_eq!(form.build().expect("valid"), saved);
        form.host = "other".to_string();
        form.name = "renamed".to_string();
        let built = form.build().expect("valid");
        assert_eq!(
            (built.host.as_str(), built.name.as_str()),
            ("other", "renamed")
        );
        assert_eq!(built.label.as_deref(), Some("Production DB"));
        form.name.clear();
        assert!(form.build().expect_err("name").contains("name"));
    }

    #[test]
    fn space_cycles_the_ssl_mode_and_it_is_built_into_the_profile() {
        let mut form = ConnectForm::new("u");
        assert_eq!(form.build().expect("valid").ssl_mode, SslMode::Disable);
        form.focus = SSL_MODE;
        form.type_char(' ');
        assert_eq!(form.build().expect("valid").ssl_mode, SslMode::Prefer);
        for _ in 0..4 {
            form.type_char(' ');
        }
        assert_eq!(form.ssl_mode, SslMode::Disable, "wraps around");
        let mut saved = Profile::new("x");
        saved.ssl_mode = SslMode::Prefer;
        let mut edit = ConnectForm::edit(&saved);
        edit.focus = SSL_MODE;
        edit.type_char(' ');
        assert_eq!(edit.build().expect("valid").ssl_mode, SslMode::Require);
    }

    #[test]
    fn the_session_form_changes_only_database_and_user() {
        let mut current = Profile::new("dev");
        current.host = "db".to_string();
        current.ssl_mode = SslMode::Prefer;
        let mut form = SessionForm::new("old", "alice", 0);
        form.clear();
        typed_session(&mut form, "sales");
        form.next_field();
        form.clear();
        typed_session(&mut form, "bob");
        let p = form.apply(&current).expect("valid");
        assert_eq!(
            (p.database.as_deref(), p.user.as_deref()),
            (Some("sales"), Some("bob"))
        );
        assert_eq!((p.host.as_str(), p.ssl_mode), ("db", SslMode::Prefer));
        form.clear();
        assert!(form.apply(&current).expect_err("user").contains("user"));
        form.focus = 0;
        form.clear();
        form.focus = 1;
        typed_session(&mut form, "bob");
        assert_eq!(form.apply(&current).expect("valid").database, None);
    }

    fn typed_session(form: &mut SessionForm, text: &str) {
        text.chars().for_each(|c| form.type_char(c));
    }

    #[test]
    fn labels_match_the_fields() {
        let form = ConnectForm::new("u");
        assert_eq!(LABELS.len(), 7);
        assert!((0..5).all(|i| form.text(i).is_some()));
        assert_eq!(form.text(5), None);
        assert_eq!(form.text(6), None);
    }
}
