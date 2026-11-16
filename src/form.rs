//! The "new connection" form: connection details typed by hand. There is no
//! password field, because oxpecker never stores one and asks when it
//! connects. Pure logic.

use crate::profile::Profile;

pub const LABELS: [&str; 6] = ["Host", "Port", "Database", "User", "Save as", "Production"];
const PRODUCTION: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectForm {
    pub host: String,
    pub port: String,
    pub database: String,
    pub user: String,
    /// Saves the connection as a profile under this name. Empty means do not save.
    pub name: String,
    pub production: bool,
    /// Index into `LABELS`.
    pub focus: usize,
    pub error: Option<String>,
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
            production: false,
            focus: 0,
            error: None,
        }
    }

    pub fn on_toggle(&self) -> bool {
        self.focus == PRODUCTION
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
                self.production = !self.production;
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
        let mut profile = Profile::new(if name.is_empty() { "(manual)" } else { name });
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
        profile.production = self.production;
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
    fn labels_match_the_fields() {
        let form = ConnectForm::new("u");
        assert_eq!(LABELS.len(), 6);
        assert!((0..5).all(|i| form.text(i).is_some()));
        assert_eq!(form.text(5), None);
    }
}
