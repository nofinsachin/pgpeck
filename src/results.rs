//! What the results pane shows for a buffer, and how it is navigated.
//! Pure logic: rendering lives in `ui`.

use crate::db::{DbError, QueryOutput, ResultSet};

/// One tab: a statement's result, or the error that stopped the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab<'a> {
    Set(&'a ResultSet),
    Error(&'a DbError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Results {
    pub output: QueryOutput,
    pub tab: usize,
    /// First visible row and first visible column of the current tab.
    pub row: usize,
    pub col: usize,
}

impl Results {
    pub fn new(output: QueryOutput) -> Self {
        let mut results = Self {
            output,
            tab: 0,
            row: 0,
            col: 0,
        };
        // Show the error first, since it is what the user must act on.
        // Otherwise show the last statement that returned rows, or the last one.
        results.tab = match (&results.output.error, results.output.sets.len()) {
            (Some(_), n) => n,
            (None, 0) => 0,
            (None, n) => results
                .output
                .sets
                .iter()
                .rposition(ResultSet::returns_rows)
                .unwrap_or(n - 1),
        };
        results
    }

    pub fn tab_count(&self) -> usize {
        self.output.sets.len() + usize::from(self.output.error.is_some())
    }

    pub fn tab(&self, index: usize) -> Option<Tab<'_>> {
        match self.output.sets.get(index) {
            Some(set) => Some(Tab::Set(set)),
            None if index == self.output.sets.len() => self.output.error.as_ref().map(Tab::Error),
            None => None,
        }
    }

    pub fn current(&self) -> Option<Tab<'_>> {
        self.tab(self.tab)
    }

    fn current_set(&self) -> Option<&ResultSet> {
        match self.current()? {
            Tab::Set(set) => Some(set),
            Tab::Error(_) => None,
        }
    }

    pub fn row_count(&self) -> usize {
        self.current_set().map_or(0, |s| s.rows.len())
    }

    fn column_count(&self) -> usize {
        self.current_set().map_or(0, |s| s.columns.len())
    }

    pub fn select_tab(&mut self, step: isize) {
        let count = self.tab_count() as isize;
        if count > 0 {
            self.tab = (self.tab as isize + step).rem_euclid(count) as usize;
            self.row = 0;
            self.col = 0;
        }
    }

    /// Moves the first visible row. `page` is how many rows fit.
    pub fn scroll_rows(&mut self, delta: isize, page: usize) {
        let last_start = self.row_count().saturating_sub(page.max(1));
        let moved = (self.row as isize + delta).clamp(0, last_start as isize);
        self.row = moved as usize;
    }

    pub fn first_row(&mut self) {
        self.row = 0;
    }

    pub fn last_row(&mut self, page: usize) {
        self.row = self.row_count().saturating_sub(page.max(1));
    }

    pub fn scroll_columns(&mut self, delta: isize) {
        let last = self.column_count().saturating_sub(1);
        self.col = (self.col as isize + delta).clamp(0, last as isize) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn set(rows: usize, columns: usize) -> ResultSet {
        ResultSet {
            columns: (0..columns).map(|i| format!("c{i}")).collect(),
            rows: (0..rows)
                .map(|r| (0..columns).map(|c| Some(format!("{r}.{c}"))).collect())
                .collect(),
            total_rows: rows as u64,
            tag: format!("SELECT {rows}"),
        }
    }

    fn command_only() -> ResultSet {
        ResultSet {
            columns: Vec::new(),
            rows: Vec::new(),
            total_rows: 0,
            tag: "CREATE TABLE".to_string(),
        }
    }

    fn output(sets: Vec<ResultSet>, error: Option<DbError>) -> QueryOutput {
        QueryOutput {
            sets,
            error,
            elapsed: Duration::from_millis(5),
        }
    }

    #[test]
    fn opens_on_the_last_statement_that_returned_rows() {
        let r = Results::new(output(
            vec![set(2, 1), command_only(), set(3, 1), command_only()],
            None,
        ));
        assert_eq!(r.tab, 2);
        let r = Results::new(output(vec![command_only(), command_only()], None));
        assert_eq!(r.tab, 1);
    }

    #[test]
    fn an_error_gets_its_own_last_tab_and_opens_first() {
        let r = Results::new(output(vec![set(1, 1)], Some(DbError::new("boom"))));
        assert_eq!(r.tab_count(), 2);
        assert!(matches!(r.current(), Some(Tab::Error(e)) if e.message == "boom"));
        assert!(matches!(r.tab(0), Some(Tab::Set(_))));
        let only_error = Results::new(output(Vec::new(), Some(DbError::new("x"))));
        assert_eq!((only_error.tab_count(), only_error.tab), (1, 0));
    }

    #[test]
    fn tabs_wrap_and_reset_the_scroll() {
        let mut r = Results::new(output(vec![set(50, 5), set(50, 5)], None));
        r.scroll_rows(10, 10);
        r.scroll_columns(2);
        r.select_tab(1);
        assert_eq!((r.tab, r.row, r.col), (0, 0, 0));
        r.select_tab(-1);
        assert_eq!(r.tab, 1);
    }

    #[test]
    fn rows_scroll_within_bounds() {
        let mut r = Results::new(output(vec![set(30, 2)], None));
        r.scroll_rows(-5, 10);
        assert_eq!(r.row, 0);
        r.scroll_rows(7, 10);
        assert_eq!(r.row, 7);
        r.scroll_rows(1000, 10);
        assert_eq!(r.row, 20, "the last page stays full");
        r.first_row();
        assert_eq!(r.row, 0);
        r.last_row(10);
        assert_eq!(r.row, 20);
    }

    #[test]
    fn a_short_result_does_not_scroll() {
        let mut r = Results::new(output(vec![set(3, 2)], None));
        r.scroll_rows(5, 10);
        r.last_row(10);
        assert_eq!(r.row, 0);
    }

    #[test]
    fn columns_scroll_within_bounds() {
        let mut r = Results::new(output(vec![set(3, 4)], None));
        r.scroll_columns(10);
        assert_eq!(r.col, 3);
        r.scroll_columns(-10);
        assert_eq!(r.col, 0);
    }

    #[test]
    fn navigation_on_an_error_or_empty_tab_is_harmless() {
        let mut r = Results::new(output(Vec::new(), Some(DbError::new("x"))));
        r.scroll_rows(3, 10);
        r.scroll_columns(3);
        r.last_row(10);
        assert_eq!((r.row, r.col), (0, 0));
        let mut empty = Results::new(output(Vec::new(), None));
        empty.select_tab(1);
        assert_eq!(empty.tab_count(), 0);
    }
}
