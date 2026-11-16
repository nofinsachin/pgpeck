//! User configuration (`config.toml`). Every option has a default. Invalid
//! options are reported and replaced by the default (R21.2).

use crate::diag::{line_containing, line_of_offset, Diagnostic};

const FILE: &str = "config.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub editor: EditorConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorConfig {
    pub tab_indent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiConfig {
    pub theme: String,
    pub show_hint_bar: bool,
    pub show_context_strip: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            editor: EditorConfig { tab_indent: true },
            ui: UiConfig {
                theme: "dark".to_string(),
                show_hint_bar: true,
                show_context_strip: true,
            },
        }
    }
}

/// Known sections and their options. Anything else is reported.
const SCHEMA: &[(&str, &[&str])] = &[
    ("editor", &["tab_indent"]),
    ("ui", &["theme", "show_hint_bar", "show_context_strip"]),
];

impl Config {
    pub fn parse(src: &str) -> (Config, Vec<Diagnostic>) {
        let mut config = Config::default();
        let mut diags = Vec::new();

        let table = match toml::from_str::<toml::Table>(src) {
            Ok(t) => t,
            Err(e) => {
                let line = e.span().map(|s| line_of_offset(src, s.start));
                diags.push(Diagnostic::new(FILE, line, e.message().to_string()));
                return (config, diags);
            }
        };

        for (section, value) in &table {
            let line = line_containing(src, section);
            let Some((_, options)) = SCHEMA.iter().find(|(name, _)| name == section) else {
                diags.push(Diagnostic::new(
                    FILE,
                    line,
                    format!("unknown section `[{section}]`"),
                ));
                continue;
            };
            let Some(body) = value.as_table() else {
                diags.push(Diagnostic::new(
                    FILE,
                    line,
                    format!("`{section}` must be a table"),
                ));
                continue;
            };
            for (key, value) in body {
                let line = line_containing(src, key);
                if !options.contains(&key.as_str()) {
                    diags.push(Diagnostic::new(
                        FILE,
                        line,
                        format!("unknown option `{key}` in `[{section}]`"),
                    ));
                    continue;
                }
                if let Err(msg) = config.set(section, key, value) {
                    diags.push(Diagnostic::new(
                        FILE,
                        line,
                        format!("`{section}.{key}`: {msg}"),
                    ));
                }
            }
        }
        (config, diags)
    }

    /// Applies one already-validated (section, key). Returns a message when the
    /// value has the wrong type.
    fn set(&mut self, section: &str, key: &str, value: &toml::Value) -> Result<(), &'static str> {
        let want_bool = |v: &toml::Value| v.as_bool().ok_or("expected true or false");
        match (section, key) {
            ("editor", "tab_indent") => self.editor.tab_indent = want_bool(value)?,
            ("ui", "show_hint_bar") => self.ui.show_hint_bar = want_bool(value)?,
            ("ui", "show_context_strip") => self.ui.show_context_strip = want_bool(value)?,
            ("ui", "theme") => {
                self.ui.theme = value.as_str().ok_or("expected a string")?.to_string();
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_default_file_matches_built_in_defaults() {
        let src = include_str!("../config/default.toml");
        let (config, diags) = Config::parse(src);
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn every_schema_option_is_documented_in_default_file() {
        let src = include_str!("../config/default.toml");
        for (section, options) in SCHEMA {
            assert!(src.contains(&format!("[{section}]")), "missing [{section}]");
            for option in *options {
                assert!(src.contains(&format!("{option} =")), "missing {option}");
            }
        }
    }

    #[test]
    fn valid_values_are_applied() {
        let (config, diags) =
            Config::parse("[editor]\ntab_indent = false\n[ui]\ntheme = \"light\"\n");
        assert!(diags.is_empty());
        assert!(!config.editor.tab_indent);
        assert_eq!(config.ui.theme, "light");
    }

    #[test]
    fn wrong_type_keeps_default_and_reports_line() {
        let (config, diags) = Config::parse("[editor]\ntab_indent = \"yes\"\n");
        assert!(config.editor.tab_indent);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, Some(2));
        assert!(diags[0].message.contains("editor.tab_indent"));
    }

    #[test]
    fn unknown_section_and_option_are_reported() {
        let (_, diags) = Config::parse("[nope]\nx = 1\n[ui]\nbogus = 1\n");
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().any(|d| d.message.contains("unknown section")));
        assert!(diags
            .iter()
            .any(|d| d.message.contains("unknown option `bogus`")));
    }

    #[test]
    fn syntax_error_falls_back_to_defaults() {
        let (config, diags) = Config::parse("[editor\n");
        assert_eq!(config, Config::default());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, Some(1));
    }
}
