//! Themes are data: a TOML file that maps color roles to colors.

use std::collections::BTreeMap;
use std::str::FromStr;

use ratatui::style::Color;

use crate::diag::{line_containing, Diagnostic};

const DARK: &str = include_str!("../config/themes/dark.toml");
const LIGHT: &str = include_str!("../config/themes/light.toml");

/// Every role a theme file may define.
const ROLES: &[&str] = &[
    "focus",
    "error",
    "null",
    "dimmed",
    "keyword",
    "string",
    "number",
    "status_prod",
    "status_fg",
    "status_bg",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub focus: Color,
    pub error: Color,
    pub null: Color,
    pub dimmed: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub status_prod: Color,
    pub status_fg: Color,
    pub status_bg: Color,
}

impl Theme {
    /// A built-in theme by name.
    pub fn builtin(name: &str) -> Option<Theme> {
        let src = match name {
            "dark" => DARK,
            "light" => LIGHT,
            _ => return None,
        };
        // Built-in files are covered by tests, so parsing cannot fail in practice.
        Some(Self::parse(src, name, &Self::fallback()).0)
    }

    /// Parses a theme file. Roles that are missing or invalid take their value
    /// from `base`, and each problem is reported.
    pub fn parse(src: &str, file: &str, base: &Theme) -> (Theme, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let mut roles: BTreeMap<&str, Color> = BTreeMap::new();

        let table = match toml::from_str::<toml::Table>(src) {
            Ok(t) => t,
            Err(e) => {
                let line = e.span().map(|s| crate::diag::line_of_offset(src, s.start));
                diags.push(Diagnostic::new(file, line, e.message().to_string()));
                return (base.clone(), diags);
            }
        };

        for (key, value) in &table {
            let line = line_containing(src, key);
            let Some(role) = ROLES.iter().find(|r| **r == key) else {
                diags.push(Diagnostic::new(
                    file,
                    line,
                    format!("unknown color role `{key}`"),
                ));
                continue;
            };
            match value.as_str().map(Color::from_str) {
                Some(Ok(color)) => {
                    roles.insert(role, color);
                }
                _ => diags.push(Diagnostic::new(
                    file,
                    line,
                    format!("role `{key}` needs a color name or `#rrggbb` value"),
                )),
            }
        }

        let get = |role: &str, default: Color| roles.get(role).copied().unwrap_or(default);
        let theme = Theme {
            focus: get("focus", base.focus),
            error: get("error", base.error),
            null: get("null", base.null),
            dimmed: get("dimmed", base.dimmed),
            keyword: get("keyword", base.keyword),
            string: get("string", base.string),
            number: get("number", base.number),
            status_prod: get("status_prod", base.status_prod),
            status_fg: get("status_fg", base.status_fg),
            status_bg: get("status_bg", base.status_bg),
        };
        (theme, diags)
    }

    /// Used only as the base when parsing the built-in themes. Every role is
    /// defined in those files, so these values never show.
    fn fallback() -> Theme {
        Theme {
            focus: Color::Reset,
            error: Color::Reset,
            null: Color::Reset,
            dimmed: Color::Reset,
            keyword: Color::Reset,
            string: Color::Reset,
            number: Color::Reset,
            status_prod: Color::Reset,
            status_fg: Color::Reset,
            status_bg: Color::Reset,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::builtin("dark").unwrap_or_else(Self::fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_themes_define_every_role_without_problems() {
        for (name, src) in [("dark", DARK), ("light", LIGHT)] {
            let (_, diags) = Theme::parse(src, name, &Theme::fallback());
            assert!(diags.is_empty(), "{name}: {diags:?}");
            let table: toml::Table = toml::from_str(src).expect("valid toml");
            for role in ROLES {
                assert!(table.contains_key(*role), "{name} lacks role {role}");
            }
        }
    }

    #[test]
    fn invalid_color_and_unknown_role_are_reported_with_fallback() {
        let base = Theme::default();
        let src = "focus = \"notacolor\"\nbogus = \"red\"\nerror = \"#ff0000\"\n";
        let (theme, diags) = Theme::parse(src, "t.toml", &base);
        assert_eq!(theme.focus, base.focus);
        assert_eq!(theme.error, Color::Rgb(255, 0, 0));
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].line, Some(2)); // `bogus` (BTreeMap order)
        assert_eq!(diags[1].line, Some(1));
    }

    #[test]
    fn syntax_error_is_reported_with_line() {
        let (theme, diags) = Theme::parse("a = \nb = 1", "t.toml", &Theme::default());
        assert_eq!(theme, Theme::default());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].line.is_some());
    }

    #[test]
    fn unknown_builtin_is_none() {
        assert!(Theme::builtin("nope").is_none());
    }
}
