//! The relationship view: the foreign keys that leave and enter one table, and
//! a trail of the tables you followed to get there. Pure logic.

use crate::catalog::{Catalog, ForeignKey};

/// A table, as `(schema, name)`.
pub type Table = (String, String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// The tables followed so far. The last one is shown.
    pub trail: Vec<Table>,
    pub selected: usize,
}

/// One foreign key seen from the table on show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Link<'a> {
    /// True when the shown table holds the key and points elsewhere.
    pub outgoing: bool,
    pub key: &'a ForeignKey,
}

impl Link<'_> {
    /// The table on the other end.
    pub fn other(&self) -> Table {
        if self.outgoing {
            (self.key.ref_schema.clone(), self.key.ref_table.clone())
        } else {
            (self.key.schema.clone(), self.key.table.clone())
        }
    }

    /// The columns on the shown table's side, then the other side's.
    pub fn columns(&self) -> (&[String], &[String]) {
        if self.outgoing {
            (&self.key.columns, &self.key.ref_columns)
        } else {
            (&self.key.ref_columns, &self.key.columns)
        }
    }
}

impl View {
    pub fn new(schema: &str, table: &str) -> Self {
        Self {
            trail: vec![(schema.to_string(), table.to_string())],
            selected: 0,
        }
    }

    pub fn current(&self) -> &Table {
        self.trail.last().expect("the trail is never empty")
    }

    /// The keys that leave the table, then the keys that enter it.
    pub fn links<'a>(&self, catalog: &'a Catalog) -> Vec<Link<'a>> {
        let (schema, table) = self.current();
        let out = catalog
            .references_from(schema, table)
            .into_iter()
            .map(|key| Link {
                outgoing: true,
                key,
            });
        let incoming = catalog
            .referenced_by(schema, table)
            .into_iter()
            .map(|key| Link {
                outgoing: false,
                key,
            });
        out.chain(incoming).collect()
    }

    pub fn select_next(&mut self, count: usize) {
        if count > 0 {
            self.selected = (self.selected + 1) % count;
        }
    }

    pub fn select_previous(&mut self, count: usize) {
        if count > 0 {
            self.selected = (self.selected + count - 1) % count;
        }
    }

    /// Shows the table on the other end of `link`.
    pub fn follow(&mut self, link: &Link<'_>) {
        self.trail.push(link.other());
        self.selected = 0;
    }

    /// Goes back one table. False when already at the first.
    pub fn back(&mut self) -> bool {
        if self.trail.len() > 1 {
            self.trail.pop();
            self.selected = 0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::foreign_key_from_row;

    fn catalog() -> Catalog {
        let key = |row: [&str; 7]| foreign_key_from_row(row).expect("key");
        Catalog {
            objects: Vec::new(),
            foreign_keys: vec![
                key([
                    "items_order_fk",
                    "public",
                    "items",
                    "order_id",
                    "public",
                    "orders",
                    "id",
                ]),
                key([
                    "orders_customer_fk",
                    "public",
                    "orders",
                    "customer_id",
                    "sales",
                    "customers",
                    "id",
                ]),
                key([
                    "ledger_fk",
                    "public",
                    "ledger",
                    "tenant\u{1f}ref",
                    "public",
                    "orders",
                    "tenant\u{1f}number",
                ]),
            ],
        }
    }

    #[test]
    fn a_row_with_unequal_column_lists_is_refused() {
        assert!(foreign_key_from_row(["k", "s", "t", "a", "s", "u", "a\u{1f}b"]).is_none());
        assert!(foreign_key_from_row(["k", "s", "t", "", "s", "u", ""]).is_none());
    }

    #[test]
    fn links_list_what_leaves_the_table_then_what_enters_it() {
        let catalog = catalog();
        let view = View::new("public", "orders");
        let links = view.links(&catalog);
        let seen: Vec<(bool, &str)> = links
            .iter()
            .map(|l| (l.outgoing, l.key.name.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                (true, "orders_customer_fk"),
                (false, "items_order_fk"),
                (false, "ledger_fk")
            ]
        );
        assert!(View::new("public", "nothing").links(&catalog).is_empty());
    }

    #[test]
    fn following_a_link_shows_the_other_table_and_back_returns() {
        let catalog = catalog();
        let mut view = View::new("public", "orders");
        let links = view.links(&catalog);
        view.follow(&links[0]);
        assert_eq!(
            view.current(),
            &("sales".to_string(), "customers".to_string())
        );
        let links = view.links(&catalog);
        assert_eq!(links.len(), 1);
        assert!(!links[0].outgoing, "orders points at customers");
        assert!(view.back());
        assert_eq!(view.current().1, "orders");
        assert!(!view.back(), "already at the first table");
    }

    #[test]
    fn selection_wraps_and_an_empty_list_stays_put() {
        let mut view = View::new("public", "orders");
        view.select_previous(3);
        assert_eq!(view.selected, 2);
        view.select_next(3);
        assert_eq!(view.selected, 0);
        view.select_next(0);
        assert_eq!(view.selected, 0);
    }
}
