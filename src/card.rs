//! The card shown when an object is pecked: its facts laid out in sections.
//! This module holds the card's state (what is open, which key is selected,
//! how far it is scrolled) and turns the facts into rows of styled text. The
//! `ui` module only maps tones to colours. Pure logic.

use crate::describe::{Column, Detail, FunctionDetail, Index, Reference, RelationDetail, Target};
use crate::ui::format_bytes;

/// How a piece of text is meant to look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    /// Names that matter.
    Strong,
    /// Section titles.
    Heading,
    /// Labels and things that are not the point.
    Dim,
    /// Data types.
    Type,
    /// Primary keys.
    Key,
    /// Foreign keys and the tables they lead to.
    Link,
    /// Something wrong or unusual.
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg(pub String, pub Tone);

/// One line of the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub segs: Vec<Seg>,
    /// The foreign key this row stands for, when it can be followed.
    pub link: Option<usize>,
    /// A character that fills the rest of the line, for section rules.
    pub fill: Option<char>,
}

impl Row {
    fn new(segs: Vec<Seg>) -> Self {
        Self {
            segs,
            link: None,
            fill: None,
        }
    }

    fn blank() -> Self {
        Self::new(Vec::new())
    }

    /// The row's text without styling, for tests.
    pub fn text(&self) -> String {
        self.segs.iter().map(|s| s.0.as_str()).collect()
    }
}

fn seg(text: impl Into<String>, tone: Tone) -> Seg {
    Seg(text.into(), tone)
}

fn heading(title: &str, count: Option<usize>) -> Row {
    let mut segs = vec![seg("── ", Tone::Dim), seg(title, Tone::Heading)];
    if let Some(n) = count {
        segs.push(seg(format!(" {n} "), Tone::Dim));
    } else {
        segs.push(seg(" ", Tone::Dim));
    }
    Row {
        segs,
        link: None,
        fill: Some('─'),
    }
}

fn pad(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - count))
    }
}

/// A short count: `1.2M`, `340k`, `87`.
fn short_count(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{}k", n / 1_000),
        1_000_000..=9_999_999 => format!("{:.1}M", n as f64 / 1e6),
        10_000_000..=999_999_999 => format!("{}M", n / 1_000_000),
        _ => format!("{:.1}B", n as f64 / 1e9),
    }
}

/// A bar `width` cells wide with `filled` of them solid.
fn bar(part: u64, whole: u64, width: usize) -> String {
    if whole == 0 || width == 0 {
        return "░".repeat(width);
    }
    let filled = ((part as f64 / whole as f64) * width as f64).round() as usize;
    let filled = if part > 0 { filled.clamp(1, width) } else { 0 };
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn facts(pairs: &[(&str, String)]) -> Row {
    let mut segs = vec![seg("  ", Tone::Plain)];
    for (n, (label, value)) in pairs.iter().enumerate() {
        if n > 0 {
            segs.push(seg("  ·  ", Tone::Dim));
        }
        segs.push(seg(format!("{label} "), Tone::Dim));
        segs.push(seg(value.clone(), Tone::Plain));
    }
    Row::new(segs)
}

fn column_rows(columns: &[Column]) -> Vec<Row> {
    let name_width = columns
        .iter()
        .map(|c| c.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(28);
    let type_width = columns
        .iter()
        .map(|c| c.data_type.chars().count())
        .max()
        .unwrap_or(0)
        .min(24);
    columns
        .iter()
        .map(|c| {
            let marker = if c.primary_key {
                seg("PK ", Tone::Key)
            } else if c.foreign_key {
                seg("FK ", Tone::Link)
            } else {
                seg("   ", Tone::Plain)
            };
            let mut segs = vec![
                seg("  ", Tone::Plain),
                marker,
                seg(
                    pad(&c.name, name_width),
                    if c.primary_key {
                        Tone::Strong
                    } else {
                        Tone::Plain
                    },
                ),
                seg("  ", Tone::Plain),
                seg(pad(&c.data_type, type_width), Tone::Type),
                seg("  ", Tone::Plain),
            ];
            segs.push(if c.not_null {
                seg("NOT NULL", Tone::Plain)
            } else {
                seg("null    ", Tone::Dim)
            });
            match (c.generated, &c.default) {
                (Some("stored"), Some(expr)) => {
                    segs.push(seg(format!("  generated: {expr}"), Tone::Dim));
                }
                (Some(how), _) => segs.push(seg(format!("  identity ({how})"), Tone::Dim)),
                (None, Some(expr)) => segs.push(seg(format!("  default {expr}"), Tone::Dim)),
                (None, None) => {}
            }
            Row::new(segs)
        })
        .collect()
}

fn index_rows(indexes: &[Index]) -> Vec<Row> {
    let largest = indexes.iter().map(|i| i.bytes).max().unwrap_or(0);
    let name_width = indexes
        .iter()
        .map(|i| i.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(34);
    let columns: Vec<String> = indexes
        .iter()
        .map(|i| format!("({})", i.columns.join(", ")))
        .collect();
    let column_width = columns
        .iter()
        .map(|c| c.chars().count())
        .max()
        .unwrap_or(0)
        .min(36);
    let method_width = indexes
        .iter()
        .map(|i| i.method.chars().count())
        .max()
        .unwrap_or(0);
    indexes
        .iter()
        .zip(&columns)
        .map(|(i, cols)| {
            let badge = if i.primary {
                ("PRIMARY", Tone::Key)
            } else if i.unique {
                ("UNIQUE ", Tone::Link)
            } else {
                ("       ", Tone::Plain)
            };
            let mut segs = vec![
                seg("  ", Tone::Plain),
                seg(pad(&i.name, name_width), Tone::Strong),
                seg("  ", Tone::Plain),
                seg(badge.0, badge.1),
                seg("  ", Tone::Plain),
                seg(pad(&i.method, method_width), Tone::Dim),
                seg("  ", Tone::Plain),
                seg(pad(cols, column_width), Tone::Plain),
                seg("  ", Tone::Plain),
                seg(format!("{:>9}", format_bytes(i.bytes)), Tone::Dim),
                seg(" ", Tone::Plain),
                seg(bar(i.bytes, largest, 8), Tone::Link),
            ];
            if let Some(predicate) = &i.predicate {
                segs.push(seg(format!("  where {predicate}"), Tone::Dim));
            }
            if let Some(space) = &i.tablespace {
                segs.push(seg(format!("  in {space}"), Tone::Dim));
            }
            if !i.valid {
                segs.push(seg("  INVALID", Tone::Warn));
            }
            Row::new(segs)
        })
        .collect()
}

fn reference_rows(references: &[Reference]) -> Vec<Row> {
    references
        .iter()
        .enumerate()
        .map(|(index, r)| {
            let mut segs = vec![
                seg("  ", Tone::Plain),
                seg(format!("({})", r.columns.join(", ")), Tone::Plain),
                seg(" → ", Tone::Link),
                seg(format!("{}.{}", r.ref_schema, r.ref_table), Tone::Strong),
                seg(format!(" ({})", r.ref_columns.join(", ")), Tone::Plain),
                seg(format!("   {}", r.name), Tone::Dim),
            ];
            if r.on_delete != "no action" {
                segs.push(seg(format!("  on delete {}", r.on_delete), Tone::Dim));
            }
            if r.on_update != "no action" {
                segs.push(seg(format!("  on update {}", r.on_update), Tone::Dim));
            }
            if !r.validated {
                segs.push(seg("  NOT VALID", Tone::Warn));
            }
            Row {
                segs,
                link: Some(index),
                fill: None,
            }
        })
        .collect()
}

/// Lines of SQL or source, capped so a huge body does not bury the card.
fn code_rows(text: &str, limit: usize) -> Vec<Row> {
    let lines: Vec<&str> = text.lines().collect();
    let mut rows: Vec<Row> = lines
        .iter()
        .take(limit)
        .map(|l| {
            Row::new(vec![seg(
                format!("  {}", l.replace('\t', "    ")),
                Tone::Dim,
            )])
        })
        .collect();
    if lines.len() > limit {
        rows.push(Row::new(vec![seg(
            format!("  … {} more lines", lines.len() - limit),
            Tone::Dim,
        )]));
    }
    rows
}

const CODE_LINES: usize = 40;

fn relation_rows(d: &RelationDetail) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut first = vec![
        ("owner", d.owner.clone()),
        (
            "tablespace",
            d.tablespace
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        ),
    ];
    if d.unlogged {
        first.push(("storage", "unlogged".to_string()));
    }
    if let Some(populated) = d.populated {
        first.push((
            "data",
            if populated {
                "populated"
            } else {
                "not populated"
            }
            .to_string(),
        ));
    }
    rows.push(facts(&first));
    if !d.is_view || d.total_bytes.is_some() {
        let mut second = Vec::new();
        if let Some(n) = d.estimated_rows {
            second.push(("rows", format!("~{}", short_count(n))));
        }
        if let Some(total) = d.total_bytes {
            second.push(("size", format_bytes(total)));
        }
        if !second.is_empty() {
            rows.push(facts(&second));
        }
    }
    if let (Some(table), Some(index)) = (d.table_bytes, d.index_bytes) {
        let whole = table + index;
        let width = 28;
        let data = if whole == 0 {
            0
        } else {
            (((table as f64 / whole as f64) * width as f64).round() as usize)
                .clamp(usize::from(table > 0), width)
        };
        rows.push(Row::new(vec![
            seg("  ", Tone::Plain),
            seg("█".repeat(data), Tone::Type),
            seg("▒".repeat(width - data), Tone::Link),
            seg(format!("  data {}", format_bytes(table)), Tone::Dim),
            seg(format!("  ·  indexes {}", format_bytes(index)), Tone::Dim),
        ]));
    }
    if let Some(comment) = &d.comment {
        rows.push(Row::new(vec![seg(format!("  “{comment}”"), Tone::Dim)]));
    }

    rows.push(Row::blank());
    rows.push(heading("COLUMNS", Some(d.columns.len())));
    rows.extend(column_rows(&d.columns));

    if !d.is_view || !d.indexes.is_empty() {
        rows.push(Row::blank());
        rows.push(heading("INDEXES", Some(d.indexes.len())));
        if d.indexes.is_empty() {
            rows.push(Row::new(vec![seg("  none", Tone::Dim)]));
        } else {
            rows.extend(index_rows(&d.indexes));
        }
    }
    if !d.is_view {
        rows.push(Row::blank());
        rows.push(heading("REFERENCES", Some(d.references.len())));
        if d.references.is_empty() {
            rows.push(Row::new(vec![seg(
                "  none: this table points at no other",
                Tone::Dim,
            )]));
        } else {
            rows.extend(reference_rows(&d.references));
        }
    }
    if let Some(definition) = &d.definition {
        rows.push(Row::blank());
        rows.push(heading("DEFINITION", None));
        rows.extend(code_rows(definition.trim(), CODE_LINES));
    }
    rows
}

/// Splits a declaration's argument list at the commas that are not inside
/// brackets, for example `a numeric(10, 2), b text`.
fn split_arguments(arguments: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in arguments.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

fn function_rows(d: &FunctionDetail) -> Vec<Row> {
    let mut rows = Vec::new();
    rows.push(facts(&[
        ("language", d.language.clone()),
        ("owner", d.owner.clone()),
        (
            "runs as",
            if d.security_definer {
                "definer"
            } else {
                "caller"
            }
            .to_string(),
        ),
    ]));
    let mut traits = vec![d.volatility.to_string()];
    if d.strict {
        traits.push("strict (null in, null out)".to_string());
    }
    rows.push(facts(&[("behaviour", traits.join(", "))]));
    if let Some(comment) = &d.comment {
        rows.push(Row::new(vec![seg(format!("  “{comment}”"), Tone::Dim)]));
    }
    rows.push(Row::blank());
    let arguments = split_arguments(&d.arguments);
    rows.push(heading("ARGUMENTS", Some(arguments.len())));
    if arguments.is_empty() {
        rows.push(Row::new(vec![seg("  none", Tone::Dim)]));
    }
    for argument in arguments {
        rows.push(Row::new(vec![
            seg("  ", Tone::Plain),
            seg(argument, Tone::Type),
        ]));
    }
    if !d.returns.is_empty() {
        rows.push(Row::blank());
        rows.push(heading("RETURNS", None));
        rows.push(Row::new(vec![
            seg("  ", Tone::Plain),
            seg(d.returns.clone(), Tone::Type),
        ]));
    }
    rows.push(Row::blank());
    rows.push(heading("SOURCE", None));
    match &d.source {
        Some(source) => rows.extend(code_rows(source.trim_matches('\n'), CODE_LINES)),
        None => rows.push(Row::new(vec![seg(
            "  compiled or built in: no source to show",
            Tone::Dim,
        )])),
    }
    rows
}

/// The rows of a described object.
pub fn rows(detail: &Detail) -> Vec<Row> {
    match detail {
        Detail::Relation(d) => relation_rows(d),
        Detail::Function(d) => function_rows(d),
    }
}

/// The foreign keys of a described object, in the order its link rows use.
pub fn references(detail: &Detail) -> &[Reference] {
    match detail {
        Detail::Relation(d) if !d.is_view => &d.references,
        _ => &[],
    }
}

/// What reading one object came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load {
    Loading,
    Ready(Detail),
    Failed(String),
}

/// One object on the trail, with where the view of it was left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub target: Target,
    pub load: Load,
    /// Index among the foreign keys.
    pub selected: usize,
    /// First visible row.
    pub scroll: usize,
}

impl Entry {
    fn new(target: Target) -> Self {
        Self {
            target,
            load: Load::Loading,
            selected: 0,
            scroll: 0,
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        match &self.load {
            Load::Ready(detail) => rows(detail),
            _ => Vec::new(),
        }
    }

    pub fn references(&self) -> &[Reference] {
        match &self.load {
            Load::Ready(detail) => references(detail),
            _ => &[],
        }
    }
}

/// The objects pecked so far. The last one is on show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub trail: Vec<Entry>,
}

impl Card {
    pub fn new(target: Target) -> Self {
        Self {
            trail: vec![Entry::new(target)],
        }
    }

    pub fn current(&self) -> &Entry {
        self.trail.last().expect("the trail is never empty")
    }

    fn current_mut(&mut self) -> &mut Entry {
        self.trail.last_mut().expect("the trail is never empty")
    }

    /// Shows another object, keeping the way back.
    pub fn follow(&mut self, target: Target) {
        self.trail.push(Entry::new(target));
    }

    /// Back to the previous object. False when already at the first.
    pub fn back(&mut self) -> bool {
        if self.trail.len() > 1 {
            self.trail.pop();
            true
        } else {
            false
        }
    }

    /// Stores what the server said about `target`. False when nothing on the
    /// trail asked for it, for example because it was closed meanwhile.
    pub fn set_load(&mut self, target: &Target, result: Result<Detail, String>) -> bool {
        let Some(entry) = self
            .trail
            .iter_mut()
            .rev()
            .find(|e| &e.target == target && e.load == Load::Loading)
        else {
            return false;
        };
        entry.load = match result {
            Ok(detail) => Load::Ready(detail),
            Err(message) => Load::Failed(message),
        };
        true
    }

    /// The foreign key under the selection.
    pub fn selected_target(&self) -> Option<Target> {
        let entry = self.current();
        entry
            .references()
            .get(entry.selected)
            .map(Reference::target)
    }

    /// Moves the selection through the foreign keys, wrapping at the ends.
    /// `visible` is how many rows fit, so the selection can be kept in view.
    pub fn select(&mut self, step: isize, visible: usize) {
        let count = self.current().references().len();
        if count == 0 {
            self.scroll(step, visible);
            return;
        }
        let entry = self.current_mut();
        entry.selected = (entry.selected as isize + step).rem_euclid(count as isize) as usize;
        self.reveal(visible);
    }

    /// Scrolls the card by `delta` rows, within its length.
    pub fn scroll(&mut self, delta: isize, visible: usize) {
        let total = self.current().rows().len();
        let last = total.saturating_sub(visible.max(1));
        let entry = self.current_mut();
        entry.scroll = (entry.scroll as isize + delta).clamp(0, last as isize) as usize;
    }

    pub fn to_top(&mut self) {
        self.current_mut().scroll = 0;
    }

    pub fn to_bottom(&mut self, visible: usize) {
        let total = self.current().rows().len();
        self.current_mut().scroll = total.saturating_sub(visible.max(1));
    }

    /// Scrolls just enough to bring the selected foreign key into view.
    pub fn reveal(&mut self, visible: usize) {
        let entry = self.current();
        let Some(position) = entry
            .rows()
            .iter()
            .position(|r| r.link == Some(entry.selected))
        else {
            return;
        };
        let visible = visible.max(1);
        let entry = self.current_mut();
        if position < entry.scroll {
            entry.scroll = position;
        } else if position >= entry.scroll + visible {
            entry.scroll = position + 1 - visible;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::describe::{relation_from_rows, Row as Cells};

    fn cells(items: &[&str]) -> Cells {
        items.iter().map(|c| Some((*c).to_string())).collect()
    }

    fn table() -> Detail {
        let sep = crate::describe::LIST_SEPARATOR;
        let rows = vec![
            cells(&[
                "head",
                "r",
                "app",
                "",
                "1200000",
                "400000",
                "300000",
                "100000",
                "Customer orders",
                "p",
            ]),
            cells(&[
                "col",
                "1",
                "id",
                "bigint",
                "true",
                "nextval('s'::regclass)",
                "",
                "true",
                "false",
                "",
            ]),
            cells(&[
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
            cells(&[
                "col", "3", "note", "text", "false", "", "", "false", "false", "",
            ]),
            cells(&[
                "idx",
                "orders_pkey",
                "btree",
                "id",
                "true",
                "true",
                "",
                "40000",
                "true",
                "",
            ]),
            cells(&[
                "idx",
                "orders_note_idx",
                "btree",
                &format!("note{sep}id"),
                "false",
                "false",
                "(note IS NOT NULL)",
                "10000",
                "false",
                "",
            ]),
            cells(&[
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
            cells(&[
                "fk",
                "orders_region_fk",
                "customer_id",
                "sales",
                "regions",
                "id",
                "n",
                "a",
                "false",
                "",
            ]),
        ];
        Detail::Relation(Box::new(
            relation_from_rows("public", "orders", &rows).expect("found"),
        ))
    }

    fn texts(detail: &Detail) -> Vec<String> {
        rows(detail).iter().map(Row::text).collect()
    }

    #[test]
    fn a_table_card_has_facts_columns_indexes_and_references_in_that_order() {
        let lines = texts(&table());
        let at = |needle: &str| lines.iter().position(|l| l.contains(needle)).expect(needle);
        assert!(
            lines[0].contains("owner app") && lines[0].contains("tablespace default"),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("rows ~1.2M") && l.contains("size 390.6 kB")),
            "{lines:?}"
        );
        assert!(lines
            .iter()
            .any(|l| l.contains("data 293.0 kB") && l.contains("indexes 97.7 kB")));
        assert!(lines.iter().any(|l| l.contains("“Customer orders”")));
        assert!(at("COLUMNS") < at("INDEXES") && at("INDEXES") < at("REFERENCES"));
        assert!(at("COLUMNS 3") < at("PK id"));
    }

    #[test]
    fn columns_show_keys_types_nullability_and_defaults_in_aligned_columns() {
        let lines = texts(&table());
        let id = lines.iter().find(|l| l.contains("PK id")).expect("pk row");
        assert!(
            id.contains("bigint")
                && id.contains("NOT NULL")
                && id.contains("default nextval('s'::regclass)"),
            "{id}"
        );
        let fk = lines
            .iter()
            .find(|l| l.contains("FK customer_id"))
            .expect("fk row");
        assert!(fk.contains("NOT NULL"));
        let note = lines
            .iter()
            .find(|l| l.contains("   note "))
            .expect("nullable row");
        assert!(note.contains("null"), "{note}");
        // Types start in the same cell on every column row.
        let at = |l: &String, t: &str| l.find(t).expect(t);
        assert_eq!(at(id, "bigint"), at(fk, "bigint"));
    }

    #[test]
    fn indexes_show_kind_columns_size_and_trouble() {
        let lines = texts(&table());
        let pkey = lines
            .iter()
            .find(|l| l.contains("orders_pkey"))
            .expect("pkey");
        assert!(
            pkey.contains("PRIMARY")
                && pkey.contains("btree")
                && pkey.contains("(id)")
                && pkey.contains('█'),
            "{pkey}"
        );
        let note = lines
            .iter()
            .find(|l| l.contains("orders_note_idx"))
            .expect("idx");
        assert!(
            note.contains("(note, id)")
                && note.contains("where (note IS NOT NULL)")
                && note.contains("INVALID"),
            "{note}"
        );
    }

    #[test]
    fn references_are_followable_rows_in_order_with_their_rules() {
        let detail = table();
        let all = rows(&detail);
        let links: Vec<(usize, String)> = all
            .iter()
            .filter_map(|r| r.link.map(|n| (n, r.text())))
            .collect();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0, 0);
        assert!(
            links[0].1.contains("(customer_id) → sales.customers (id)")
                && links[0].1.contains("on delete cascade"),
            "{}",
            links[0].1
        );
        assert!(links[1].1.contains("NOT VALID") && links[1].1.contains("on delete set null"));
        assert_eq!(references(&detail).len(), 2);
    }

    #[test]
    fn a_view_card_shows_columns_and_its_query_but_no_references() {
        let rows_in = vec![
            cells(&["head", "v", "app", "", "-1", "", "", "", "", "p"]),
            cells(&[
                "col", "1", "n", "integer", "false", "", "", "false", "false", "",
            ]),
            cells(&[
                "def",
                " SELECT 1 AS n\n FROM t;",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
        ];
        let detail = Detail::Relation(Box::new(
            relation_from_rows("public", "v", &rows_in).expect("found"),
        ));
        let lines = texts(&detail);
        assert!(lines.iter().any(|l| l.contains("DEFINITION")));
        assert!(lines.iter().any(|l| l.contains("SELECT 1 AS n")));
        assert!(!lines
            .iter()
            .any(|l| l.contains("REFERENCES") || l.contains("INDEXES")));
        assert!(references(&detail).is_empty());
    }

    #[test]
    fn a_long_definition_is_cut_with_a_count_of_the_rest() {
        let body = (0..50)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let shown = code_rows(&body, 40);
        assert_eq!(shown.len(), 41);
        assert!(shown[40].text().contains("10 more lines"));
    }

    #[test]
    fn a_function_card_splits_arguments_at_top_level_commas() {
        assert_eq!(
            split_arguments("a numeric(10, 2), b text DEFAULT 'x, y', c integer[]"),
            ["a numeric(10, 2)", "b text DEFAULT 'x", "y'", "c integer[]"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        let detail = Detail::Function(Box::new(crate::describe::function_from_row(
            "public",
            "refresh",
            &cells(&[
                "f",
                "since date, n integer",
                "void",
                "plpgsql",
                "v",
                "false",
                "false",
                "app",
                "",
                "BEGIN\n  NULL;\nEND",
            ]),
        )));
        let lines = texts(&detail);
        assert!(lines.iter().any(|l| l.contains("language plpgsql")));
        assert!(lines.iter().any(|l| l.contains("ARGUMENTS 2")));
        assert!(lines.iter().any(|l| l.trim() == "since date"));
        assert!(lines.iter().any(|l| l.trim() == "void"));
        assert!(lines.iter().any(|l| l.contains("NULL;")));
        let compiled = Detail::Function(Box::new(crate::describe::function_from_row(
            "public",
            "f",
            &cells(&["f", "", "int4", "c", "i", "true", "false", "app", "", ""]),
        )));
        assert!(texts(&compiled)
            .iter()
            .any(|l| l.contains("no source to show")));
    }

    #[test]
    fn the_trail_follows_and_goes_back_and_results_find_their_entry() {
        let mut card = Card::new(Target::relation("public", "orders"));
        assert!(card.set_load(&Target::relation("public", "orders"), Ok(table())));
        assert_eq!(
            card.selected_target(),
            Some(Target::relation("sales", "customers"))
        );
        card.follow(Target::relation("sales", "customers"));
        assert_eq!(card.current().load, Load::Loading);
        // A result for something not on the trail is ignored.
        assert!(!card.set_load(&Target::relation("x", "y"), Ok(table())));
        assert!(card.set_load(&Target::relation("sales", "customers"), Err("gone".into())));
        assert_eq!(card.current().load, Load::Failed("gone".into()));
        assert!(card.back());
        assert_eq!(card.current().target.name, "orders");
        assert!(!card.back());
    }

    #[test]
    fn selection_wraps_through_the_references_and_keeps_its_row_in_view() {
        let mut card = Card::new(Target::relation("public", "orders"));
        card.set_load(&Target::relation("public", "orders"), Ok(table()));
        card.select(1, 4);
        assert_eq!(card.current().selected, 1);
        let rows = card.current().rows();
        let position = rows.iter().position(|r| r.link == Some(1)).expect("row");
        let scroll = card.current().scroll;
        assert!(
            position >= scroll && position < scroll + 4,
            "{position} in {scroll}.."
        );
        card.select(1, 4);
        assert_eq!(card.current().selected, 0, "wraps");
        assert!(card.current().scroll <= rows.iter().position(|r| r.link == Some(0)).expect("row"));
    }

    #[test]
    fn without_references_the_arrows_scroll_and_scrolling_stays_within_the_card() {
        let mut card = Card::new(Target::relation("public", "v"));
        let rows_in = vec![
            cells(&["head", "v", "app", "", "-1", "", "", "", "", "p"]),
            cells(&[
                "col", "1", "n", "integer", "false", "", "", "false", "false", "",
            ]),
            cells(&[
                "def",
                &(0..30)
                    .map(|n| format!("l{n}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
        ];
        let detail = Detail::Relation(Box::new(
            relation_from_rows("public", "v", &rows_in).expect("found"),
        ));
        card.set_load(&Target::relation("public", "v"), Ok(detail));
        let total = card.current().rows().len();
        card.select(1, 10);
        assert_eq!(card.current().scroll, 1);
        card.scroll(1000, 10);
        assert_eq!(card.current().scroll, total - 10);
        card.scroll(-1000, 10);
        assert_eq!(card.current().scroll, 0);
        card.to_bottom(10);
        assert_eq!(card.current().scroll, total - 10);
        card.to_top();
        assert_eq!(card.current().scroll, 0);
    }

    #[test]
    fn counts_and_bars_read_naturally() {
        assert_eq!(short_count(87), "87");
        assert_eq!(short_count(340_000), "340k");
        assert_eq!(short_count(1_234_567), "1.2M");
        assert_eq!(short_count(45_000_000), "45M");
        assert_eq!(bar(0, 100, 8), "░░░░░░░░");
        assert_eq!(bar(100, 100, 8), "████████");
        assert_eq!(bar(1, 1000, 8), "█░░░░░░░", "a small index keeps one block");
        assert_eq!(bar(5, 0, 4), "░░░░");
    }
}
