//! The peck command line: `peck orders` draws the foreign keys around a table.
//! More commands will follow, so the grammar lives here. Pure logic.

use crate::catalog::{Catalog, Object};
use crate::fuzzy;

/// How many matching tables the command line offers.
pub const SUGGESTIONS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command<'a> {
    /// Nothing typed yet.
    Empty,
    /// Draw this table or view, as `name` or `schema.name`.
    Table(&'a str),
    /// A command that is planned but not built, such as `schema`.
    NotBuilt(&'static str),
}

/// Commands that are planned and answer "not built yet" for now.
const PLANNED: [&str; 2] = ["schema", "function"];

/// Reads what was typed. The word `peck` in front is optional.
pub fn parse(input: &str) -> Command<'_> {
    let text = input.trim();
    let text = match text.split_once(char::is_whitespace) {
        Some((first, rest)) if first.eq_ignore_ascii_case("peck") => rest.trim(),
        None if text.eq_ignore_ascii_case("peck") => "",
        _ => text,
    };
    if text.is_empty() {
        return Command::Empty;
    }
    let (first, rest) = text
        .split_once(char::is_whitespace)
        .map_or((text, ""), |(a, b)| (a, b.trim()));
    if let Some(word) = PLANNED.iter().find(|w| first.eq_ignore_ascii_case(w)) {
        if rest.is_empty() || !rest.contains(char::is_whitespace) {
            return Command::NotBuilt(word);
        }
    }
    Command::Table(text)
}

/// Tables and views that match `query`, best first.
pub fn suggestions<'a>(catalog: &'a Catalog, query: &str) -> Vec<&'a Object> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(i64, &Object)> = catalog
        .objects
        .iter()
        .filter(|o| o.kind.is_relation())
        .filter_map(|o| {
            let score = [o.path(), o.name.clone()]
                .iter()
                .filter_map(|text| fuzzy::score(query, text))
                .max()?;
            Some((score, o))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path().cmp(&b.1.path())));
    found
        .into_iter()
        .take(SUGGESTIONS)
        .map(|(_, o)| o)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Kind;

    fn object(kind: Kind, schema: &str, name: &str) -> Object {
        Object {
            kind,
            schema: schema.to_string(),
            name: name.to_string(),
            parent: None,
            detail: String::new(),
        }
    }

    #[test]
    fn the_word_peck_is_optional_and_empty_input_is_empty() {
        assert_eq!(parse(""), Command::Empty);
        assert_eq!(parse("   "), Command::Empty);
        assert_eq!(parse("peck"), Command::Empty);
        assert_eq!(parse("PECK  "), Command::Empty);
        assert_eq!(parse("peck orders"), Command::Table("orders"));
        assert_eq!(
            parse("  peck   public.orders "),
            Command::Table("public.orders")
        );
        assert_eq!(parse("orders"), Command::Table("orders"));
        // A table whose name starts with the letters is still a table.
        assert_eq!(parse("pecker"), Command::Table("pecker"));
    }

    #[test]
    fn planned_commands_say_so_and_a_table_of_that_name_needs_its_schema() {
        assert_eq!(parse("peck schema"), Command::NotBuilt("schema"));
        assert_eq!(parse("peck schema sales"), Command::NotBuilt("schema"));
        assert_eq!(parse("FUNCTION refresh"), Command::NotBuilt("function"));
        assert_eq!(parse("peck public.schema"), Command::Table("public.schema"));
    }

    #[test]
    fn suggestions_are_tables_and_views_only_best_match_first_and_capped() {
        let catalog = Catalog {
            objects: vec![
                object(Kind::Table, "public", "orders"),
                object(Kind::Table, "archive", "orders"),
                object(Kind::View, "public", "order_totals"),
                object(Kind::Column, "public", "orders"),
                object(Kind::Function, "public", "orders_refresh"),
                object(Kind::Table, "public", "customers"),
            ],
            foreign_keys: Vec::new(),
        };
        let found = suggestions(&catalog, "orders");
        let paths: Vec<String> = found.iter().map(|o| o.path()).collect();
        assert_eq!(paths.len(), 3, "{paths:?}");
        assert!(paths.contains(&"public.orders".to_string()));
        assert!(found.iter().all(|o| o.kind.is_relation()));
        assert!(suggestions(&catalog, "").is_empty());
        assert!(suggestions(&catalog, "zzz").is_empty());
        let many = Catalog {
            objects: (0..20)
                .map(|n| object(Kind::Table, "s", &format!("t{n}")))
                .collect(),
            foreign_keys: Vec::new(),
        };
        assert_eq!(suggestions(&many, "t").len(), SUGGESTIONS);
    }
}
