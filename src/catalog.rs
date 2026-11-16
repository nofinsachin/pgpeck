//! Cached schema metadata for the find overlay (R9). Pure logic: reading it
//! from the server happens in `db`.

use crate::fuzzy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Table,
    View,
    MaterializedView,
    ForeignTable,
    Column,
    Function,
    Type,
    Schema,
}

impl Kind {
    /// Display order of the groups in the find overlay (R9.4).
    pub const ORDER: [Kind; 8] = [
        Kind::Table,
        Kind::View,
        Kind::MaterializedView,
        Kind::ForeignTable,
        Kind::Column,
        Kind::Function,
        Kind::Type,
        Kind::Schema,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Table => "table",
            Kind::View => "view",
            Kind::MaterializedView => "materialized view",
            Kind::ForeignTable => "foreign table",
            Kind::Column => "column",
            Kind::Function => "function",
            Kind::Type => "type",
            Kind::Schema => "schema",
        }
    }

    /// A word short enough for a key hint.
    pub fn short(self) -> &'static str {
        match self {
            Kind::MaterializedView => "matview",
            Kind::ForeignTable => "foreign",
            other => other.label(),
        }
    }

    pub fn plural(self) -> &'static str {
        match self {
            Kind::Table => "Tables",
            Kind::View => "Views",
            Kind::MaterializedView => "Materialized views",
            Kind::ForeignTable => "Foreign tables",
            Kind::Column => "Columns",
            Kind::Function => "Functions",
            Kind::Type => "Types",
            Kind::Schema => "Schemas",
        }
    }

    /// Tables, views and the like, which can be selected from.
    pub fn is_relation(self) -> bool {
        matches!(
            self,
            Kind::Table | Kind::View | Kind::MaterializedView | Kind::ForeignTable
        )
    }

    /// The word the catalog query uses for this kind.
    pub fn from_code(code: &str) -> Option<Kind> {
        Some(match code {
            "table" => Kind::Table,
            "view" => Kind::View,
            "matview" => Kind::MaterializedView,
            "foreign" => Kind::ForeignTable,
            "column" => Kind::Column,
            "function" => Kind::Function,
            "type" => Kind::Type,
            "schema" => Kind::Schema,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub kind: Kind,
    pub schema: String,
    pub name: String,
    /// For a column, the relation it belongs to.
    pub parent: Option<String>,
    /// A column's data type, or a function's arguments.
    pub detail: String,
}

impl Object {
    /// The path shown to the user, so equal names in different schemas stay
    /// apart (R9.3): `schema.table.column`, `schema.table`, or `schema`.
    pub fn path(&self) -> String {
        match (self.kind, &self.parent) {
            (Kind::Schema, _) => self.name.clone(),
            (_, Some(parent)) => format!("{}.{parent}.{}", self.schema, self.name),
            _ => format!("{}.{}", self.schema, self.name),
        }
    }

    /// What follows the path: a type or an argument list.
    pub fn summary(&self) -> String {
        match self.kind {
            Kind::Function => format!("({})", self.detail),
            _ => self.detail.clone(),
        }
    }

    /// Text to put in the editor, quoted where PostgreSQL needs it. A column
    /// is qualified by its relation only.
    pub fn sql_name(&self) -> String {
        match (self.kind, &self.parent) {
            (Kind::Schema, _) => quote(&self.name),
            (_, Some(parent)) => format!("{}.{}", quote(parent), quote(&self.name)),
            _ => format!("{}.{}", quote(&self.schema), quote(&self.name)),
        }
    }

    fn haystacks(&self) -> [String; 2] {
        [self.path(), self.name.clone()]
    }
}

/// Quotes an identifier unless it is plain lower case and not a keyword.
pub fn quote(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '$');
    let keyword = crate::sql::keywords()
        .binary_search(&name.to_ascii_uppercase().as_str())
        .is_ok();
    if plain && !keyword {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub objects: Vec<Object>,
}

/// One row of the catalog query: kind, schema, name, parent, detail.
pub fn object_from_row(row: [&str; 5]) -> Option<Object> {
    Some(Object {
        kind: Kind::from_code(row[0])?,
        schema: row[1].to_string(),
        name: row[2].to_string(),
        parent: (!row[3].is_empty()).then(|| row[3].to_string()),
        detail: row[4].to_string(),
    })
}

/// How many results one group shows.
pub const PER_KIND: usize = 25;

impl Catalog {
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Fuzzy matches, grouped by kind in `Kind::ORDER` and best first within a
    /// group. With nothing typed, columns are left out: there are too many to
    /// be a useful list.
    pub fn search(&self, query: &str) -> Vec<&Object> {
        let query = query.trim();
        let mut groups: Vec<Vec<(i64, &Object)>> = vec![Vec::new(); Kind::ORDER.len()];
        for object in &self.objects {
            if query.is_empty() && object.kind == Kind::Column {
                continue;
            }
            let score = object
                .haystacks()
                .iter()
                .filter_map(|text| fuzzy::score(query, text))
                .max();
            let Some(score) = score else { continue };
            if let Some(index) = Kind::ORDER.iter().position(|k| *k == object.kind) {
                groups[index].push((score, object));
            }
        }
        let mut found = Vec::new();
        for mut group in groups {
            group.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path().cmp(&b.1.path())));
            found.extend(group.into_iter().take(PER_KIND).map(|(_, o)| o));
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(kind: Kind, schema: &str, name: &str, parent: Option<&str>) -> Object {
        Object {
            kind,
            schema: schema.into(),
            name: name.into(),
            parent: parent.map(Into::into),
            detail: String::new(),
        }
    }

    fn catalog() -> Catalog {
        Catalog {
            objects: vec![
                object(Kind::Schema, "public", "public", None),
                object(Kind::Table, "public", "orders", None),
                object(Kind::Table, "archive", "orders", None),
                object(Kind::View, "public", "order_totals", None),
                object(Kind::Column, "public", "order_id", Some("orders")),
                object(Kind::Function, "public", "refresh_orders", None),
            ],
        }
    }

    #[test]
    fn results_are_grouped_by_kind_with_tables_first() {
        let c = catalog();
        let kinds: Vec<Kind> = c.search("order").iter().map(|o| o.kind).collect();
        let mut sorted = kinds.clone();
        sorted.sort_by_key(|k| Kind::ORDER.iter().position(|o| o == k));
        assert_eq!(kinds, sorted);
        assert_eq!(kinds[0], Kind::Table);
    }

    #[test]
    fn same_names_in_different_schemas_show_their_schema() {
        let c = catalog();
        let paths: Vec<String> = c
            .search("orders")
            .iter()
            .filter(|o| o.kind == Kind::Table)
            .map(|o| o.path())
            .collect();
        assert!(paths.contains(&"public.orders".to_string()));
        assert!(paths.contains(&"archive.orders".to_string()));
    }

    #[test]
    fn a_schema_in_the_query_narrows_the_results() {
        let c = catalog();
        let tables: Vec<String> = c
            .search("archive.ord")
            .iter()
            .filter(|o| o.kind == Kind::Table)
            .map(|o| o.path())
            .collect();
        assert_eq!(tables, ["archive.orders"]);
    }

    #[test]
    fn an_empty_query_lists_everything_but_columns() {
        let c = catalog();
        let found = c.search("");
        assert_eq!(found.len(), 5);
        assert!(found.iter().all(|o| o.kind != Kind::Column));
    }

    #[test]
    fn each_group_is_capped() {
        let objects = (0..100)
            .map(|i| object(Kind::Table, "public", &format!("t{i}"), None))
            .collect();
        assert_eq!(Catalog { objects }.search("t").len(), PER_KIND);
    }

    #[test]
    fn identifiers_are_quoted_only_when_needed() {
        assert_eq!(quote("orders"), "orders");
        assert_eq!(quote("Orders"), "\"Orders\"");
        assert_eq!(quote("my table"), "\"my table\"");
        assert_eq!(quote("order"), "\"order\"");
        assert_eq!(quote("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote("1abc"), "\"1abc\"");
    }

    #[test]
    fn sql_names_are_qualified() {
        let o = object(Kind::Table, "Sales", "orders", None);
        assert_eq!(o.sql_name(), "\"Sales\".orders");
        let c = object(Kind::Column, "public", "id", Some("orders"));
        assert_eq!(c.sql_name(), "orders.id");
    }

    #[test]
    fn rows_from_the_server_are_read() {
        let o = object_from_row(["column", "public", "id", "orders", "integer"]).unwrap();
        assert_eq!(o.kind, Kind::Column);
        assert_eq!(o.parent.as_deref(), Some("orders"));
        assert!(object_from_row(["bogus", "", "", "", ""]).is_none());
    }
}
