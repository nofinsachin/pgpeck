//! Query history: the statements you ran, newest last. It keeps the text and a
//! few facts about the run, never the rows. Lines are appended to a file in the
//! state directory, one entry per line, with tabs and newlines in the text
//! escaped.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::fuzzy;

pub const FILE: &str = "history.tsv";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Seconds since the Unix epoch.
    pub when: u64,
    /// The connection's name.
    pub connection: String,
    pub ok: bool,
    pub millis: u64,
    /// Rows the last row-returning statement produced.
    pub rows: Option<u64>,
    pub sql: String,
}

pub fn path() -> Option<PathBuf> {
    crate::paths::state_dir().map(|d| d.join(FILE))
}

/// Statements that probably carry a secret are not recorded.
pub fn should_record(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    !sql.trim().is_empty()
        && !["password", "passwd", "identified by"]
            .iter()
            .any(|word| lower.contains(word))
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

impl Entry {
    /// One line, without the final newline.
    pub fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.when,
            u8::from(self.ok),
            self.millis,
            self.rows.map_or_else(String::new, |r| r.to_string()),
            escape(&self.connection),
            escape(&self.sql)
        )
    }

    /// `None` for a line that is damaged: it is skipped, not fatal.
    pub fn from_line(line: &str) -> Option<Self> {
        let mut parts = line.splitn(6, '\t');
        let when = parts.next()?.parse().ok()?;
        let ok = parts.next()? == "1";
        let millis = parts.next()?.parse().ok()?;
        let rows = match parts.next()? {
            "" => None,
            n => Some(n.parse().ok()?),
        };
        let connection = unescape(parts.next()?);
        let sql = unescape(parts.next()?);
        Some(Self {
            when,
            connection,
            ok,
            millis,
            rows,
            sql,
        })
    }
}

/// Reads the history, newest last. Damaged lines are skipped. Only the newest
/// `max` entries are kept, and a file much longer than that is cut down.
pub fn load(path: &Path, max: usize) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = text.lines().filter_map(Entry::from_line).collect();
    if entries.len() > max {
        entries.drain(..entries.len() - max);
        if entries.len() * 2 < text.lines().count() {
            let contents: String = entries.iter().map(|e| e.to_line() + "\n").collect();
            let _ = crate::profile::write_private(path, &contents);
        }
    }
    entries
}

/// Adds one line to the file, creating it (and its directory) as private.
pub fn append(path: &Path, line: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        writeln!(file, "{line}")
    }
    #[cfg(not(unix))]
    {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{line}")
    }
}

/// Entries that match `query`, newest first when the query is empty and best
/// match first otherwise. The search looks at the statement and the connection.
pub fn search<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    if query.trim().is_empty() {
        return entries.iter().rev().collect();
    }
    let mut scored: Vec<(i64, usize, &Entry)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, e)| {
            let text = format!(
                "{} {}",
                e.sql.split_whitespace().collect::<Vec<_>>().join(" "),
                e.connection
            );
            fuzzy::score(query, &text).map(|s| (s, index, e))
        })
        .collect();
    // Best score first, and the newer of two equal scores first.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    scored.into_iter().map(|(_, _, e)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sql: &str) -> Entry {
        Entry {
            when: 1_700_000_000,
            connection: "prod\tdb".to_string(),
            ok: true,
            millis: 42,
            rows: Some(7),
            sql: sql.to_string(),
        }
    }

    #[test]
    fn an_entry_survives_a_round_trip_with_tabs_newlines_and_backslashes() {
        let e = entry("select 'a\tb',\n  'c\\d'\r\nfrom t");
        let line = e.to_line();
        assert!(!line.contains('\n') && !line.contains('\r'));
        assert_eq!(line.matches('\t').count(), 5);
        assert_eq!(Entry::from_line(&line), Some(e));
        let mut failed = entry("select 1/0");
        failed.ok = false;
        failed.rows = None;
        assert_eq!(Entry::from_line(&failed.to_line()), Some(failed));
    }

    #[test]
    fn damaged_lines_are_skipped() {
        assert_eq!(Entry::from_line(""), None);
        assert_eq!(Entry::from_line("not\tan\tentry"), None);
        assert_eq!(Entry::from_line("x\t1\t2\t3\tc\ts"), None);
    }

    #[test]
    fn statements_that_may_hold_a_secret_are_not_recorded() {
        assert!(should_record("select * from users"));
        assert!(!should_record("ALTER ROLE app PASSWORD 'x'"));
        assert!(!should_record("create user bob with password 'x'"));
        assert!(!should_record("   "));
    }

    #[test]
    fn loading_keeps_the_newest_and_a_long_file_is_cut_down() {
        let dir = std::env::temp_dir().join(format!("pgpeck-history-{}", std::process::id()));
        let path = dir.join(FILE);
        let _ = std::fs::remove_dir_all(&dir);
        for n in 0..10 {
            append(&path, &entry(&format!("select {n}")).to_line()).expect("append");
        }
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut f| writeln!(f, "garbage"))
            .expect("garbage");
        let entries = load(&path, 3);
        let sql: Vec<&str> = entries.iter().map(|e| e.sql.as_str()).collect();
        assert_eq!(sql, ["select 7", "select 8", "select 9"]);
        // 11 lines on disk for 3 kept: the file was rewritten.
        assert_eq!(
            std::fs::read_to_string(&path)
                .expect("read")
                .lines()
                .count(),
            3
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_ranks_by_match_and_lists_newest_first_without_a_query() {
        let entries = vec![
            entry("select * from orders"),
            entry("select * from customers"),
            entry("update orders set a = 1"),
        ];
        let all: Vec<&str> = search(&entries, "")
            .iter()
            .map(|e| e.sql.as_str())
            .collect();
        assert_eq!(all[0], "update orders set a = 1");
        let hits = search(&entries, "orders");
        assert_eq!(hits.len(), 2);
        assert!(search(&entries, "zzz").is_empty());
    }
}
