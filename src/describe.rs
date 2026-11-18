//! What pecking an object shows: the facts the server holds about a table,
//! view or function. This module holds the types and turns the rows the
//! server sends into them. Reading them is `db::describe`. Pure logic.

use crate::catalog::{Kind, Object};

/// Separates the items of a list inside one field of the query result.
pub const LIST_SEPARATOR: char = '\u{1f}';

/// The object to describe.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    pub kind: Kind,
    pub schema: String,
    pub name: String,
    /// A function's identity arguments, which tell overloads apart.
    pub args: String,
}

impl Target {
    pub fn relation(schema: &str, name: &str) -> Self {
        Self {
            kind: Kind::Table,
            schema: schema.to_string(),
            name: name.to_string(),
            args: String::new(),
        }
    }

    pub fn from_object(object: &Object) -> Option<Self> {
        if !(object.kind.is_relation() || object.kind == Kind::Function) {
            return None;
        }
        Some(Self {
            kind: object.kind,
            schema: object.schema.clone(),
            name: object.name.clone(),
            args: if object.kind == Kind::Function {
                object.detail.clone()
            } else {
                String::new()
            },
        })
    }

    /// `schema.name`, with the arguments for a function.
    pub fn label(&self) -> String {
        if self.kind == Kind::Function {
            format!("{}.{}({})", self.schema, self.name, self.args)
        } else {
            format!("{}.{}", self.schema, self.name)
        }
    }

    pub fn is_function(&self) -> bool {
        self.kind == Kind::Function
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub not_null: bool,
    pub default: Option<String>,
    /// `always` or `by default` for an identity column, `stored` for a
    /// generated one. For a generated column `default` holds the expression.
    pub generated: Option<&'static str>,
    pub primary_key: bool,
    pub foreign_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    pub method: String,
    /// Columns or expressions, in key order.
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    pub predicate: Option<String>,
    pub bytes: u64,
    pub valid: bool,
    pub tablespace: Option<String>,
}

/// A foreign key this table holds: the tables it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_delete: &'static str,
    pub on_update: &'static str,
    pub validated: bool,
}

impl Reference {
    pub fn target(&self) -> Target {
        Target::relation(&self.ref_schema, &self.ref_table)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationDetail {
    pub schema: String,
    pub name: String,
    /// What it is, in words: `table`, `view`, `materialized view`, and so on.
    pub kind: &'static str,
    pub is_view: bool,
    pub owner: String,
    pub tablespace: Option<String>,
    /// The planner's row estimate. `None` before the first analyze.
    pub estimated_rows: Option<u64>,
    pub total_bytes: Option<u64>,
    pub table_bytes: Option<u64>,
    pub index_bytes: Option<u64>,
    pub comment: Option<String>,
    pub unlogged: bool,
    /// For a materialized view: whether it holds data.
    pub populated: Option<bool>,
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
    pub references: Vec<Reference>,
    /// A view's query.
    pub definition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDetail {
    pub schema: String,
    pub name: String,
    pub kind: &'static str,
    /// As written in the declaration, for example `since date, n integer DEFAULT 5`.
    pub arguments: String,
    pub returns: String,
    pub language: String,
    pub volatility: &'static str,
    pub strict: bool,
    pub security_definer: bool,
    pub owner: String,
    pub comment: Option<String>,
    /// The body. Left out for functions written in C or built in.
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    Relation(Box<RelationDetail>),
    Function(Box<FunctionDetail>),
}

/// One row of a result, as text.
pub type Row = Vec<Option<String>>;

fn text(row: &Row, i: usize) -> &str {
    row.get(i).and_then(|v| v.as_deref()).unwrap_or("")
}

fn flag(row: &Row, i: usize) -> bool {
    text(row, i) == "true" || text(row, i) == "t"
}

fn number(row: &Row, i: usize) -> Option<u64> {
    text(row, i).parse().ok()
}

fn some(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_string())
}

fn list(text: &str) -> Vec<String> {
    text.split(LIST_SEPARATOR)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn action(code: &str) -> &'static str {
    match code {
        "c" => "cascade",
        "n" => "set null",
        "d" => "set default",
        "r" => "restrict",
        _ => "no action",
    }
}

fn relation_kind(code: &str) -> (&'static str, bool) {
    match code {
        "v" => ("view", true),
        "m" => ("materialized view", true),
        "f" => ("foreign table", false),
        "p" => ("partitioned table", false),
        _ => ("table", false),
    }
}

/// Builds the description of a relation from the rows of `db::RELATION_SQL`.
/// Each row starts with what it is: `head`, `col`, `idx`, `fk` or `def`.
/// `None` when the relation was not found.
pub fn relation_from_rows(schema: &str, name: &str, rows: &[Row]) -> Option<RelationDetail> {
    let head = rows.iter().find(|r| text(r, 0) == "head")?;
    let (kind, is_view) = relation_kind(text(head, 1));
    let mut detail = RelationDetail {
        schema: schema.to_string(),
        name: name.to_string(),
        kind,
        is_view,
        owner: text(head, 2).to_string(),
        tablespace: some(text(head, 3)),
        estimated_rows: text(head, 4)
            .parse::<i64>()
            .ok()
            .and_then(|n| u64::try_from(n).ok()),
        total_bytes: number(head, 5),
        table_bytes: number(head, 6),
        index_bytes: number(head, 7),
        comment: some(text(head, 8)),
        unlogged: text(head, 1) != "m" && text(head, 9) == "u",
        populated: (text(head, 1) == "m").then(|| flag(head, 9)),
        columns: Vec::new(),
        indexes: Vec::new(),
        references: Vec::new(),
        definition: None,
    };
    let mut columns: Vec<(i64, Column)> = Vec::new();
    for row in rows {
        match text(row, 0) {
            "col" => {
                let generated = match text(row, 6) {
                    "a" => Some("always"),
                    "d" => Some("by default"),
                    s if s.ends_with('s') => Some("stored"),
                    _ => None,
                };
                columns.push((
                    text(row, 1).parse().unwrap_or(0),
                    Column {
                        name: text(row, 2).to_string(),
                        data_type: text(row, 3).to_string(),
                        not_null: flag(row, 4),
                        default: some(text(row, 5)),
                        generated,
                        primary_key: flag(row, 7),
                        foreign_key: flag(row, 8),
                    },
                ));
            }
            "idx" => detail.indexes.push(Index {
                name: text(row, 1).to_string(),
                method: text(row, 2).to_string(),
                columns: list(text(row, 3)),
                unique: flag(row, 4),
                primary: flag(row, 5),
                predicate: some(text(row, 6)),
                bytes: number(row, 7).unwrap_or(0),
                valid: flag(row, 8),
                tablespace: some(text(row, 9)),
            }),
            "fk" => detail.references.push(Reference {
                name: text(row, 1).to_string(),
                columns: list(text(row, 2)),
                ref_schema: text(row, 3).to_string(),
                ref_table: text(row, 4).to_string(),
                ref_columns: list(text(row, 5)),
                on_delete: action(text(row, 6)),
                on_update: action(text(row, 7)),
                validated: flag(row, 8),
            }),
            "def" => detail.definition = some(text(row, 1)),
            _ => {}
        }
    }
    columns.sort_by_key(|(n, _)| *n);
    detail.columns = columns.into_iter().map(|(_, c)| c).collect();
    // The primary key first, then the rest by name.
    detail
        .indexes
        .sort_by(|a, b| b.primary.cmp(&a.primary).then_with(|| a.name.cmp(&b.name)));
    detail.references.sort_by(|a, b| a.name.cmp(&b.name));
    Some(detail)
}

/// Builds the description of a function from the one row of `db::FUNCTION_SQL`.
pub fn function_from_row(schema: &str, name: &str, row: &Row) -> FunctionDetail {
    FunctionDetail {
        schema: schema.to_string(),
        name: name.to_string(),
        kind: match text(row, 0) {
            "p" => "procedure",
            "a" => "aggregate",
            "w" => "window function",
            _ => "function",
        },
        arguments: text(row, 1).to_string(),
        returns: text(row, 2).to_string(),
        language: text(row, 3).to_string(),
        volatility: match text(row, 4) {
            "i" => "immutable",
            "s" => "stable",
            _ => "volatile",
        },
        strict: flag(row, 5),
        security_definer: flag(row, 6),
        owner: text(row, 7).to_string(),
        comment: some(text(row, 8)),
        source: some(text(row, 9)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Row {
        cells.iter().map(|c| Some((*c).to_string())).collect()
    }

    fn rows() -> Vec<Row> {
        let sep = LIST_SEPARATOR.to_string();
        vec![
            row(&[
                "col",
                "2",
                "customer_id",
                "bigint",
                "true",
                "",
                "",
                "false",
                "true",
                "",
            ]),
            row(&[
                "head",
                "r",
                "app",
                "fast",
                "1200000",
                "400",
                "300",
                "100",
                "Customer orders",
                "p",
            ]),
            row(&[
                "col",
                "1",
                "id",
                "bigint",
                "true",
                "nextval('orders_id_seq'::regclass)",
                "",
                "true",
                "false",
                "",
            ]),
            row(&[
                "col",
                "3",
                "total",
                "numeric(12,2)",
                "false",
                "",
                "",
                "false",
                "false",
                "",
            ]),
            row(&[
                "col",
                "4",
                "gross",
                "numeric",
                "false",
                "(total * 1.2)",
                "s",
                "false",
                "false",
                "",
            ]),
            row(&[
                "col", "5", "ref", "bigint", "true", "", "d", "false", "false", "",
            ]),
            row(&[
                "idx",
                "orders_customer_idx",
                "btree",
                "customer_id",
                "false",
                "false",
                "",
                "50",
                "true",
                "",
            ]),
            row(&[
                "idx",
                "orders_pkey",
                "btree",
                "id",
                "true",
                "true",
                "",
                "40",
                "true",
                "",
            ]),
            row(&[
                "idx",
                "orders_ref_idx",
                "btree",
                &format!("customer_id{sep}ref"),
                "true",
                "false",
                "(total > 0)",
                "10",
                "false",
                "archive",
            ]),
            row(&[
                "fk",
                "orders_customer_fk",
                "customer_id",
                "sales",
                "customers",
                "id",
                "c",
                "a",
                "true",
                "",
            ]),
        ]
    }

    #[test]
    fn a_table_is_built_from_its_rows_with_columns_in_table_order() {
        let d = relation_from_rows("public", "orders", &rows()).expect("found");
        assert_eq!(
            (d.kind, d.is_view, d.owner.as_str()),
            ("table", false, "app")
        );
        assert_eq!(d.tablespace.as_deref(), Some("fast"));
        assert_eq!(d.estimated_rows, Some(1_200_000));
        assert_eq!(
            (d.total_bytes, d.table_bytes, d.index_bytes),
            (Some(400), Some(300), Some(100))
        );
        assert_eq!(d.comment.as_deref(), Some("Customer orders"));
        let names: Vec<&str> = d.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "customer_id", "total", "gross", "ref"]);
        assert!(d.columns[0].primary_key && d.columns[0].not_null);
        assert!(d.columns[0]
            .default
            .as_deref()
            .is_some_and(|x| x.starts_with("nextval")));
        assert!(d.columns[1].foreign_key && !d.columns[1].primary_key);
        assert!(!d.columns[2].not_null);
        // A generated column keeps its expression, and says how it is made.
        assert_eq!(d.columns[3].generated, Some("stored"));
        assert_eq!(d.columns[3].default.as_deref(), Some("(total * 1.2)"));
        assert_eq!(d.columns[4].generated, Some("by default"));
    }

    #[test]
    fn indexes_list_the_primary_key_first_with_their_columns_predicate_and_state() {
        let d = relation_from_rows("public", "orders", &rows()).expect("found");
        let names: Vec<&str> = d.indexes.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            ["orders_pkey", "orders_customer_idx", "orders_ref_idx"]
        );
        assert!(d.indexes[0].primary && d.indexes[0].unique);
        assert_eq!(d.indexes[2].columns, ["customer_id", "ref"]);
        assert_eq!(d.indexes[2].predicate.as_deref(), Some("(total > 0)"));
        assert!(!d.indexes[2].valid);
        assert_eq!(d.indexes[2].tablespace.as_deref(), Some("archive"));
    }

    #[test]
    fn references_name_the_table_they_point_at_and_what_happens_on_delete() {
        let d = relation_from_rows("public", "orders", &rows()).expect("found");
        assert_eq!(d.references.len(), 1);
        let r = &d.references[0];
        assert_eq!((r.on_delete, r.on_update), ("cascade", "no action"));
        assert_eq!(r.columns, ["customer_id"]);
        assert_eq!(r.ref_columns, ["id"]);
        assert_eq!(r.target(), Target::relation("sales", "customers"));
    }

    #[test]
    fn a_view_has_a_definition_and_no_size_and_a_missing_relation_is_none() {
        let rows = vec![
            row(&["head", "m", "app", "", "-1", "", "", "", "", "false"]),
            row(&[
                "col", "1", "n", "integer", "false", "", "", "false", "false", "",
            ]),
            row(&["def", " SELECT 1 AS n;", "", "", "", "", "", "", "", ""]),
        ];
        let d = relation_from_rows("public", "v", &rows).expect("found");
        assert_eq!((d.kind, d.is_view), ("materialized view", true));
        assert_eq!(d.populated, Some(false));
        assert_eq!(
            (d.estimated_rows, d.total_bytes, d.tablespace.clone()),
            (None, None, None)
        );
        assert_eq!(d.definition.as_deref(), Some(" SELECT 1 AS n;"));
        assert!(relation_from_rows("public", "gone", &[]).is_none());
    }

    #[test]
    fn a_function_is_built_from_its_row() {
        let f = function_from_row(
            "public",
            "refresh",
            &row(&[
                "f",
                "since date",
                "void",
                "plpgsql",
                "v",
                "false",
                "true",
                "app",
                "",
                "BEGIN END",
            ]),
        );
        assert_eq!(
            (f.kind, f.volatility, f.language.as_str()),
            ("function", "volatile", "plpgsql")
        );
        assert!(f.security_definer && !f.strict);
        assert_eq!(f.source.as_deref(), Some("BEGIN END"));
        assert_eq!(f.comment, None);
        let p = function_from_row(
            "p",
            "n",
            &row(&["p", "", "", "sql", "i", "true", "false", "o", "", ""]),
        );
        assert_eq!(
            (p.kind, p.volatility, p.source),
            ("procedure", "immutable", None)
        );
    }

    #[test]
    fn targets_come_from_relations_and_functions_only() {
        let object = |kind, detail: &str| Object {
            kind,
            schema: "public".to_string(),
            name: "x".to_string(),
            parent: None,
            detail: detail.to_string(),
        };
        assert!(Target::from_object(&object(Kind::Column, "int")).is_none());
        assert!(Target::from_object(&object(Kind::Schema, "")).is_none());
        let t = Target::from_object(&object(Kind::Table, "")).expect("table");
        assert_eq!(t.label(), "public.x");
        let f = Target::from_object(&object(Kind::Function, "a integer")).expect("function");
        assert_eq!(f.label(), "public.x(a integer)");
        assert!(f.is_function());
    }
}
