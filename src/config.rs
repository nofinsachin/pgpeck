//! User configuration (`config.toml`). Every option has a default. Invalid
//! options are reported and replaced by the default (R21.2).

use crate::diag::{line_containing, line_of_offset, Diagnostic};
use crate::secrets::Source;

const FILE: &str = "config.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub editor: EditorConfig,
    pub ui: UiConfig,
    pub connections: ConnectionsConfig,
    pub results: ResultsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultsConfig {
    /// Rows kept per result. More are counted but not stored (R12.9, PRF-005).
    pub row_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionsConfig {
    /// Where to look for a password, in order (R2.1).
    pub password_sources: Vec<Source>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorConfig {
    pub tab_indent: bool,
    pub indent_width: usize,
    pub auto_pair: bool,
    pub auto_indent: bool,
    pub completion: bool,
    pub completion_min_chars: usize,
}

impl EditorConfig {
    pub fn options(&self) -> crate::editor::Options {
        crate::editor::Options {
            indent_width: self.indent_width,
            auto_pair: self.auto_pair,
            auto_indent: self.auto_indent,
        }
    }
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
            editor: EditorConfig {
                tab_indent: true,
                indent_width: 4,
                auto_pair: true,
                auto_indent: true,
                completion: true,
                completion_min_chars: 2,
            },
            results: ResultsConfig { row_limit: 10_000 },
            connections: ConnectionsConfig {
                password_sources: Source::default_order(),
            },
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
    (
        "editor",
        &[
            "tab_indent",
            "indent_width",
            "auto_pair",
            "auto_indent",
            "completion",
            "completion_min_chars",
        ],
    ),
    ("ui", &["theme", "show_hint_bar", "show_context_strip"]),
    ("connections", &["password_sources"]),
    ("results", &["row_limit"]),
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
            ("editor", "auto_pair") => self.editor.auto_pair = want_bool(value)?,
            ("editor", "auto_indent") => self.editor.auto_indent = want_bool(value)?,
            ("editor", "completion") => self.editor.completion = want_bool(value)?,
            ("editor", "completion_min_chars") => {
                self.editor.completion_min_chars = value
                    .as_integer()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| (1..=10).contains(n))
                    .ok_or("expected a whole number from 1 to 10")?;
            }
            ("editor", "indent_width") => {
                self.editor.indent_width = value
                    .as_integer()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| (1..=16).contains(n))
                    .ok_or("expected a whole number from 1 to 16")?;
            }
            ("ui", "show_hint_bar") => self.ui.show_hint_bar = want_bool(value)?,
            ("ui", "show_context_strip") => self.ui.show_context_strip = want_bool(value)?,
            ("results", "row_limit") => {
                self.results.row_limit = value
                    .as_integer()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| (1..=1_000_000).contains(n))
                    .ok_or("expected a whole number from 1 to 1000000")?;
            }
            ("connections", "password_sources") => {
                self.connections.password_sources = parse_sources(value)?;
            }
            ("ui", "theme") => {
                self.ui.theme = value.as_str().ok_or("expected a string")?.to_string();
            }
            _ => {}
        }
        Ok(())
    }
}

/// A list of source names, each used once.
fn parse_sources(value: &toml::Value) -> Result<Vec<Source>, &'static str> {
    let names = value
        .as_array()
        .ok_or("expected a list such as [\"env\", \"prompt\"]")?;
    let mut sources = Vec::new();
    for name in names {
        let source = name
            .as_str()
            .and_then(Source::from_name)
            .ok_or("each entry must be one of pgpass, env, prompt")?;
        if sources.contains(&source) {
            return Err("each source can be listed only once");
        }
        sources.push(source);
    }
    if sources.is_empty() {
        return Err("the list cannot be empty");
    }
    Ok(sources)
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
    fn indent_width_is_range_checked() {
        for bad in ["0", "17", "-1", "2.5", "\"4\""] {
            let (config, diags) = Config::parse(&format!("[editor]\nindent_width = {bad}\n"));
            assert_eq!(config.editor.indent_width, 4, "{bad}");
            assert_eq!(diags.len(), 1, "{bad}");
        }
        let (config, diags) = Config::parse("[editor]\nindent_width = 2\n");
        assert!(diags.is_empty());
        assert_eq!(config.editor.indent_width, 2);
    }

    #[test]
    fn row_limit_is_range_checked() {
        for bad in ["0", "1000001", "-5", "\"10\""] {
            let (config, diags) = Config::parse(&format!("[results]\nrow_limit = {bad}\n"));
            assert_eq!(config.results.row_limit, 10_000, "{bad}");
            assert_eq!(diags.len(), 1, "{bad}");
        }
        let (config, diags) = Config::parse("[results]\nrow_limit = 500\n");
        assert!(diags.is_empty());
        assert_eq!(config.results.row_limit, 500);
    }

    #[test]
    fn password_sources_are_read_in_order() {
        let (config, diags) =
            Config::parse("[connections]\npassword_sources = [\"env\", \"prompt\"]\n");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(
            config.connections.password_sources,
            [Source::Env, Source::Prompt]
        );
    }

    #[test]
    fn bad_password_sources_keep_the_default() {
        for bad in [
            "[\"keychain\"]",
            "[]",
            "[\"env\", \"env\"]",
            "\"env\"",
            "[1]",
        ] {
            let (config, diags) =
                Config::parse(&format!("[connections]\npassword_sources = {bad}\n"));
            assert_eq!(
                config.connections.password_sources,
                Source::default_order(),
                "{bad}"
            );
            assert_eq!(diags.len(), 1, "{bad}");
        }
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
