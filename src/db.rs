//! The only module that talks to PostgreSQL. Everything else receives plain
//! data through events (steering: `db` boundary).

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio_postgres::{CancelToken, Client, Config, NoTls, SimpleQueryMessage};

use crate::catalog::{self, Catalog};
use crate::profile::{Profile, SslMode};
use crate::secrets::Secret;
use crate::sql;

/// Identifies a connection inside the application.
pub type ConnId = u64;

/// Set on every connection so sessions can be recognized in `pg_stat_activity`.
pub const APPLICATION_NAME: &str = "pgpeck";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Cells longer than this are cut when stored. The full value viewer (R13)
/// will fetch values on demand.
pub const MAX_CELL_BYTES: usize = 8 * 1024;

/// A live connection.
#[derive(Clone)]
pub struct Session {
    client: Arc<Client>,
    cancel: CancelToken,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session")
    }
}

/// What the user needs to know about a connection (R3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub server_version: String,
    /// Always false until TLS support lands.
    pub tls: bool,
}

/// A problem reported by the server or the driver. Never contains a password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbError {
    pub message: String,
    pub sqlstate: Option<String>,
    pub detail: Option<String>,
    pub hint: Option<String>,
    /// 1-based character offset into the statement.
    pub position: Option<u32>,
}

impl DbError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            sqlstate: None,
            detail: None,
            hint: None,
            position: None,
        }
    }

    /// The server refused the login (wrong password or no password).
    pub fn is_auth_failure(&self) -> bool {
        matches!(self.sqlstate.as_deref(), Some("28P01" | "28000"))
    }

    /// The statement was cancelled by the user or a timeout.
    pub fn is_cancellation(&self) -> bool {
        self.sqlstate.as_deref() == Some("57014")
    }
}

impl From<&tokio_postgres::Error> for DbError {
    fn from(error: &tokio_postgres::Error) -> Self {
        match error.as_db_error() {
            Some(db) => Self {
                message: db.message().to_string(),
                sqlstate: Some(db.code().code().to_string()),
                detail: db.detail().map(str::to_string),
                hint: db.hint().map(str::to_string),
                position: match db.position() {
                    Some(tokio_postgres::error::ErrorPosition::Original(n)) => Some(*n),
                    _ => None,
                },
            },
            None => Self::new(describe_driver_error(error)),
        }
    }
}

/// Driver errors are terse ("db error", "error connecting to server"). The
/// useful part is in the source chain.
fn describe_driver_error(error: &tokio_postgres::Error) -> String {
    use std::error::Error;
    if let Some(db) = error.as_db_error() {
        return format!("{}: {}", db.severity(), db.message());
    }
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

fn is_local(host: &str) -> bool {
    host == "localhost" || host == "::1" || host.starts_with("127.") || host.starts_with('/')
}

/// Connects. `on_close` runs once when the connection ends later.
///
/// TLS is not implemented yet. Instead of silently sending data in the clear,
/// a remote host needs `ssl_mode = "disable"` to be explicit about it.
pub async fn connect(
    profile: &Profile,
    user: &str,
    password: Option<&Secret>,
    on_close: impl FnOnce(String) + Send + 'static,
) -> Result<(Session, ConnectionInfo), DbError> {
    match profile.ssl_mode {
        SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {
            return Err(DbError::new(format!(
                "ssl_mode `{}` needs TLS, which this version does not support yet",
                profile.ssl_mode.name()
            )));
        }
        SslMode::Prefer if !is_local(&profile.host) => {
            return Err(DbError::new(
                "TLS is not supported yet, so a remote host is refused unless its SSL mode is `disable`. \
                 Edit the connection (Ctrl+E in the connect window) and set SSL mode to disable",
            ));
        }
        SslMode::Disable | SslMode::Prefer => {}
    }

    let mut config = Config::new();
    if profile.host.starts_with('/') {
        config.host_path(&profile.host);
    } else {
        config.host(&profile.host);
    }
    config
        .port(profile.port)
        .user(user)
        .application_name(APPLICATION_NAME)
        .connect_timeout(CONNECT_TIMEOUT)
        .ssl_mode(match profile.ssl_mode {
            SslMode::Disable => tokio_postgres::config::SslMode::Disable,
            _ => tokio_postgres::config::SslMode::Prefer,
        });
    let database = profile.database.clone().unwrap_or_else(|| user.to_string());
    config.dbname(&database);
    if let Some(password) = password {
        config.password(password.expose());
    }

    let (client, connection) = config.connect(NoTls).await.map_err(|e| DbError::from(&e))?;
    tokio::spawn(async move {
        let reason = match connection.await {
            Ok(()) => "the connection was closed".to_string(),
            Err(e) => describe_driver_error(&e),
        };
        on_close(reason);
    });
    let session = Session {
        cancel: client.cancel_token(),
        client: Arc::new(client),
    };

    let row = session
        .client
        .simple_query("select current_setting('server_version'), current_user::text, current_database()::text")
        .await
        .map_err(|e| DbError::from(&e))?
        .into_iter()
        .find_map(|m| match m {
            SimpleQueryMessage::Row(r) => Some(r),
            _ => None,
        });
    let field = |i: usize| {
        row.as_ref()
            .and_then(|r| r.get(i))
            .unwrap_or("")
            .to_string()
    };
    let info = ConnectionInfo {
        host: profile.host.clone(),
        port: profile.port,
        database: field(2),
        user: field(1),
        server_version: field(0),
        tls: false,
    };
    Ok((session, info))
}

/// Asks the server to cancel the running statement (R8.5).
pub async fn cancel(session: &Session) -> Result<(), DbError> {
    session
        .cancel
        .cancel_query(NoTls)
        .await
        .map_err(|e| DbError::from(&e))
}

// ---------------------------------------------------------------------------
// Running statements
// ---------------------------------------------------------------------------

/// The result of one statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultSet {
    /// Empty for statements that return no rows.
    pub columns: Vec<String>,
    /// At most the row limit. `None` is SQL NULL, which is not an empty string.
    pub rows: Vec<Vec<Option<String>>>,
    /// Rows the server sent, which can exceed `rows.len()`.
    pub total_rows: u64,
    /// Command tag, for example `INSERT 0 5`.
    pub tag: String,
}

impl ResultSet {
    pub fn returns_rows(&self) -> bool {
        !self.columns.is_empty()
    }

    pub fn truncated(&self) -> bool {
        self.total_rows > self.rows.len() as u64
    }
}

/// What the server is doing, narrowed to the database this session is in.
/// CPU and memory are not here: PostgreSQL cannot report them to a client.
/// What it can report is the share of the shared resources this database
/// uses, such as its connections out of the server's limit.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionStats {
    /// Client connections to this database, including this one.
    pub connections: u64,
    /// Client connections to the whole server, and its limit.
    pub server_connections: u64,
    pub max_connections: u64,
    /// `None` when the role may not see other roles' sessions.
    pub active: Option<u64>,
    pub idle_in_transaction: Option<u64>,
    /// Age in seconds of the oldest open transaction in this database.
    pub oldest_transaction_secs: Option<u64>,
    pub size_bytes: u64,
    /// Share of block reads served from memory, since the statistics reset.
    pub cache_hit_percent: Option<f64>,
    pub deadlocks: u64,
    pub uptime_secs: u64,
    /// True when the role sees every session's state.
    pub sees_all_sessions: bool,
    /// The biggest tables, largest first. Empty when they cannot be read.
    pub top_tables: Vec<TableSize>,
}

/// A table and what it takes on disk, indexes and TOAST included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSize {
    pub schema: String,
    pub name: String,
    pub bytes: u64,
    /// The planner's estimate, `None` before the first vacuum or analyze.
    pub rows_estimate: Option<u64>,
}

/// How many tables the trends pane lists.
pub const TOP_TABLES: usize = 5;

const TOP_TABLES_SQL: &str = "
select n.nspname, c.relname, pg_total_relation_size(c.oid), c.reltuples::bigint
from pg_class c join pg_namespace n on n.oid = c.relnamespace
where c.relkind in ('r', 'p', 'm')
  and n.nspname !~ '^pg_' and n.nspname <> 'information_schema'
  and not c.relispartition
order by 3 desc, 1, 2
limit 5
";

const STATS_SQL: &str = "
with me as (
  select (select rolsuper from pg_roles where rolname = current_user)
         or pg_has_role(current_user, 'pg_read_all_stats', 'USAGE') as all_sessions
), a as (
  select pid, state, xact_start
  from pg_stat_activity
  where datname = current_database() and backend_type = 'client backend'
)
select
  (select count(*) from a),
  (select count(*) from pg_stat_activity where backend_type = 'client backend'),
  current_setting('max_connections'),
  (select count(*) from a where state = 'active'),
  (select count(*) from a where state like 'idle in transaction%'),
  (select extract(epoch from now() - min(xact_start))::bigint from a
     where xact_start is not null and pid <> pg_backend_pid()),
  pg_database_size(current_database()),
  (select round(100.0 * blks_hit / nullif(blks_hit + blks_read, 0), 1)
     from pg_stat_database where datname = current_database()),
  (select deadlocks from pg_stat_database where datname = current_database()),
  extract(epoch from now() - pg_postmaster_start_time())::bigint,
  (select all_sessions from me)
";

/// Reads `SessionStats`. Call it only when the session is idle: it runs on the
/// user's own connection and must not start or break a transaction.
pub async fn stats(session: &Session) -> Result<SessionStats, DbError> {
    // One round trip, so nothing the user runs can slip in between the two
    // reads and keep the connection busy longer than the numbers need.
    let messages = session
        .client
        .simple_query(&format!("{STATS_SQL};{TOP_TABLES_SQL}"))
        .await
        .map_err(|e| DbError::from(&e))?;
    let mut rows = messages.into_iter().filter_map(|m| match m {
        SimpleQueryMessage::Row(r) => Some(r),
        _ => None,
    });
    let row = rows
        .next()
        .ok_or_else(|| DbError::new("the server returned no statistics"))?;
    let top_tables = rows
        .filter_map(|r| {
            Some(TableSize {
                schema: r.get(0)?.to_string(),
                name: r.get(1)?.to_string(),
                bytes: r.get(2)?.parse().ok()?,
                // Negative estimates mean "never analyzed".
                rows_estimate: r.get(3).and_then(|v| v.parse::<u64>().ok()),
            })
        })
        .take(TOP_TABLES)
        .collect();
    let int = |i: usize| row.get(i).and_then(|v| v.parse::<u64>().ok());
    Ok(SessionStats {
        connections: int(0).unwrap_or(0),
        server_connections: int(1).unwrap_or(0),
        max_connections: int(2).unwrap_or(0),
        active: int(3),
        idle_in_transaction: int(4),
        oldest_transaction_secs: int(5),
        size_bytes: int(6).unwrap_or(0),
        cache_hit_percent: row.get(7).and_then(|v| v.parse::<f64>().ok()),
        deadlocks: int(8).unwrap_or(0),
        uptime_secs: int(9).unwrap_or(0),
        sees_all_sessions: row.get(10) == Some("t"),
        top_tables,
    })
}

const CATALOG_SQL: &str = r"
with ns as (
  select oid, nspname from pg_namespace
  where nspname !~ '^pg_' and nspname <> 'information_schema'
), rel as (
  select c.oid, c.relname, c.relkind, ns.nspname
  from pg_class c join ns on ns.oid = c.relnamespace
  where c.relkind in ('r', 'p', 'v', 'm', 'f')
)
select 'schema', nspname, nspname, '', '' from ns
union all
select case relkind when 'v' then 'view' when 'm' then 'matview'
                    when 'f' then 'foreign' else 'table' end,
       nspname, relname, '', ''
from rel
union all
select 'column', rel.nspname, a.attname, rel.relname, format_type(a.atttypid, a.atttypmod)
from rel join pg_attribute a on a.attrelid = rel.oid
where a.attnum > 0 and not a.attisdropped
union all
select 'function', ns.nspname, p.proname, '', pg_get_function_identity_arguments(p.oid)
from pg_proc p join ns on ns.oid = p.pronamespace
where p.prokind in ('f', 'p')
union all
select 'type', ns.nspname, t.typname, '', t.typtype::text
from pg_type t join ns on ns.oid = t.typnamespace
where t.typtype in ('e', 'd')
limit 50000
";

/// Reads the schemas, relations, columns, functions and types the user can
/// see. Call it only when the session is idle, like `stats`.
pub async fn catalog(session: &Session) -> Result<Catalog, DbError> {
    let messages = session
        .client
        .simple_query(CATALOG_SQL)
        .await
        .map_err(|e| DbError::from(&e))?;
    let mut objects = Vec::new();
    for message in messages {
        let SimpleQueryMessage::Row(row) = message else {
            continue;
        };
        let text = |i: usize| row.get(i).unwrap_or("");
        if let Some(object) =
            catalog::object_from_row([text(0), text(1), text(2), text(3), text(4)])
        {
            objects.push(object);
        }
    }
    Ok(Catalog { objects })
}

/// Where the session is in a transaction (R16.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TxState {
    /// No transaction is open.
    #[default]
    Idle,
    /// `BEGIN` was run and the transaction is still open.
    Open,
    /// An error aborted the transaction. Only a rollback gets out of it.
    Failed,
}

/// Asks the server which transaction state the session is in. The driver does
/// not report it, so this compares two clocks: `now()` is fixed when the
/// transaction starts, `statement_timestamp()` when this statement starts.
/// They are equal only when the statement began the transaction itself.
/// An aborted transaction refuses the question with SQLSTATE 25P02.
pub async fn transaction_state(session: &Session) -> TxState {
    match session
        .client
        .simple_query("select now() <> statement_timestamp()")
        .await
    {
        Ok(messages) => {
            let open = messages
                .iter()
                .any(|m| matches!(m, SimpleQueryMessage::Row(r) if r.get(0) == Some("t")));
            if open {
                TxState::Open
            } else {
                TxState::Idle
            }
        }
        Err(e) if e.code().is_some_and(|c| c.code() == "25P02") => TxState::Failed,
        Err(_) => TxState::Idle,
    }
}

/// Everything a run produced. Statements before an error still have results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryOutput {
    pub sets: Vec<ResultSet>,
    pub error: Option<DbError>,
    pub elapsed: Duration,
    /// The transaction state after the run.
    pub tx: TxState,
}

/// Runs one or more statements. Memory stays bounded: only `row_limit` rows
/// per result are kept, and the rest are counted and dropped.
pub async fn run(session: &Session, sql_text: &str, row_limit: usize) -> QueryOutput {
    let started = Instant::now();
    let statements = sql::statements(sql_text);
    let mut sets = Vec::new();
    let mut error = None;

    match session.client.simple_query_raw(sql_text).await {
        Err(e) => error = Some(DbError::from(&e)),
        Ok(stream) => {
            let mut stream = std::pin::pin!(stream);
            let mut current: Option<ResultSet> = None;
            while let Some(message) = stream.next().await {
                match message {
                    Ok(SimpleQueryMessage::RowDescription(columns)) => {
                        current = Some(ResultSet {
                            columns: columns.iter().map(|c| c.name().to_string()).collect(),
                            rows: Vec::new(),
                            total_rows: 0,
                            tag: String::new(),
                        });
                    }
                    Ok(SimpleQueryMessage::Row(row)) => {
                        let set = current.get_or_insert_with(|| ResultSet {
                            columns: row.columns().iter().map(|c| c.name().to_string()).collect(),
                            rows: Vec::new(),
                            total_rows: 0,
                            tag: String::new(),
                        });
                        set.total_rows += 1;
                        if set.rows.len() < row_limit {
                            set.rows
                                .push((0..row.len()).map(|i| row.get(i).map(clip_cell)).collect());
                        }
                    }
                    Ok(SimpleQueryMessage::CommandComplete(count)) => {
                        let mut set = current.take().unwrap_or_else(|| ResultSet {
                            columns: Vec::new(),
                            rows: Vec::new(),
                            total_rows: 0,
                            tag: String::new(),
                        });
                        let statement = statements.get(sets.len()).map(|s| s.text(sql_text));
                        set.tag = command_tag(statement, count, set.returns_rows());
                        sets.push(set);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error = Some(DbError::from(&e));
                        break;
                    }
                }
            }
        }
    }
    let elapsed = started.elapsed();
    QueryOutput {
        sets,
        error,
        elapsed,
        tx: transaction_state(session).await,
    }
}

fn clip_cell(value: &str) -> String {
    if value.len() <= MAX_CELL_BYTES {
        return value.to_string();
    }
    let mut end = MAX_CELL_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

/// The driver reports only a row count, so the tag is rebuilt from the
/// statement's first words. Close to what `psql` prints, not always exact.
pub fn command_tag(statement: Option<&str>, count: u64, returned_rows: bool) -> String {
    let words: Vec<String> = statement
        .map(|s| {
            s.split_whitespace()
                .filter(|w| !w.starts_with("--"))
                .take(8)
                .map(|w| w.trim_end_matches(';').to_ascii_uppercase())
                .collect()
        })
        .unwrap_or_default();
    let first = words.first().map_or("", String::as_str);
    match first {
        "INSERT" => format!("INSERT 0 {count}"),
        "SELECT" | "UPDATE" | "DELETE" | "MERGE" | "COPY" | "FETCH" | "MOVE" | "VALUES"
        | "TABLE" => {
            let verb = if matches!(first, "VALUES" | "TABLE") {
                "SELECT"
            } else {
                first
            };
            format!("{verb} {count}")
        }
        "WITH" if returned_rows => format!("SELECT {count}"),
        "CREATE" | "ALTER" | "DROP" => {
            // Skip words that qualify the object but are not part of the tag.
            const MODIFIERS: [&str; 9] = [
                "OR",
                "REPLACE",
                "TEMP",
                "TEMPORARY",
                "UNLOGGED",
                "UNIQUE",
                "GLOBAL",
                "LOCAL",
                "RECURSIVE",
            ];
            let mut rest = words[1..]
                .iter()
                .filter(|w| !MODIFIERS.contains(&w.as_str()));
            match rest.next().map(String::as_str) {
                Some("MATERIALIZED") => format!("{first} MATERIALIZED VIEW"),
                Some(object) => format!("{first} {object}"),
                None => first.to_string(),
            }
        }
        "" if returned_rows => format!("SELECT {count}"),
        "" => "OK".to_string(),
        _ => first.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_follow_the_statement() {
        let tag = |s: &str, n: u64, rows: bool| command_tag(Some(s), n, rows);
        assert_eq!(tag("insert into t values (1)", 1, false), "INSERT 0 1");
        assert_eq!(tag("select 1", 1, true), "SELECT 1");
        assert_eq!(tag("UPDATE t SET a = 1", 5, false), "UPDATE 5");
        assert_eq!(tag("delete from t", 0, false), "DELETE 0");
        assert_eq!(tag("create table t (a int)", 0, false), "CREATE TABLE");
        assert_eq!(tag("drop index i", 0, false), "DROP INDEX");
        assert_eq!(tag("create temp table t (a int)", 0, false), "CREATE TABLE");
        assert_eq!(
            tag("create unique index i on t (a)", 0, false),
            "CREATE INDEX"
        );
        assert_eq!(
            tag("create or replace function f() ...", 0, false),
            "CREATE FUNCTION"
        );
        assert_eq!(
            tag("create materialized view v as select 1", 0, false),
            "CREATE MATERIALIZED VIEW"
        );
        assert_eq!(tag("begin", 0, false), "BEGIN");
        assert_eq!(tag("commit;", 0, false), "COMMIT");
        assert_eq!(
            tag("with x as (select 1) select * from x", 1, true),
            "SELECT 1"
        );
        assert_eq!(tag("values (1), (2)", 2, true), "SELECT 2");
        assert_eq!(command_tag(None, 3, true), "SELECT 3");
        assert_eq!(command_tag(None, 0, false), "OK");
    }

    #[test]
    fn long_cells_are_clipped_on_a_character_boundary() {
        let long = "é".repeat(MAX_CELL_BYTES);
        let clipped = clip_cell(&long);
        assert!(clipped.len() <= MAX_CELL_BYTES + '…'.len_utf8());
        assert!(clipped.ends_with('…'));
        assert_eq!(clip_cell("short"), "short");
    }

    #[test]
    fn local_hosts() {
        for host in ["localhost", "127.0.0.1", "::1", "/var/run/postgresql"] {
            assert!(is_local(host), "{host}");
        }
        for host in ["db.example.com", "10.0.0.5", "localhost.example.com"] {
            assert!(!is_local(host), "{host}");
        }
    }

    #[test]
    fn error_classification() {
        let mut e = DbError::new("x");
        assert!(!e.is_auth_failure() && !e.is_cancellation());
        e.sqlstate = Some("28P01".to_string());
        assert!(e.is_auth_failure());
        e.sqlstate = Some("57014".to_string());
        assert!(e.is_cancellation());
    }

    #[test]
    fn tls_modes_are_refused_before_any_network_use() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let mut p = Profile::new("x");
            p.ssl_mode = SslMode::Require;
            let e = connect(&p, "u", None, |_| {}).await.expect_err("refused");
            assert!(e.message.contains("TLS"));

            let mut remote = Profile::new("x");
            remote.host = "db.example.com".to_string();
            remote.ssl_mode = SslMode::Prefer;
            let e = connect(&remote, "u", None, |_| {})
                .await
                .expect_err("refused");
            assert!(e.message.contains("SSL mode to disable"));
        });
    }
}
