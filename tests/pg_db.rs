//! Integration tests against a real PostgreSQL. They run only when
//! `OXPECKER_TEST_DATABASE_URL` is set, for example:
//!
//! ```text
//! OXPECKER_TEST_DATABASE_URL=postgres://oxpecker:oxpecker@127.0.0.1:54329/oxpecker cargo test --test pg_db
//! ```

use std::time::Duration;

use oxpecker::db::{self, Session};
use oxpecker::profile::parse_uri;

fn url() -> Option<String> {
    std::env::var("OXPECKER_TEST_DATABASE_URL")
        .ok()
        .filter(|u| !u.is_empty())
}

/// Skips the test when no database is configured.
macro_rules! database {
    () => {
        match url() {
            Some(url) => url,
            None => {
                eprintln!("skipped: OXPECKER_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

async fn connect(url: &str) -> (Session, db::ConnectionInfo) {
    let uri = parse_uri(url).expect("test URL parses");
    let user = uri.profile.user.clone().expect("test URL names a user");
    db::connect(&uri.profile, &user, uri.password.as_ref(), |_| {})
        .await
        .expect("connect")
}

#[tokio::test]
async fn connects_and_reports_the_server() {
    let url = database!();
    let (_, info) = connect(&url).await;
    assert!(!info.server_version.is_empty());
    assert!(!info.user.is_empty() && !info.database.is_empty());
    assert!(!info.tls);
}

#[tokio::test]
async fn the_connection_is_named_oxpecker() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(&session, "select current_setting('application_name')", 10).await;
    assert_eq!(
        out.sets[0].rows[0][0].as_deref(),
        Some(db::APPLICATION_NAME)
    );
}

#[tokio::test]
async fn a_wrong_password_is_an_auth_failure_that_does_not_echo_it() {
    let url = database!();
    let mut uri = parse_uri(&url).expect("parse");
    uri.password = Some(oxpecker::secrets::Secret::new("definitely-wrong-pw-123"));
    let user = uri.profile.user.clone().expect("user");
    let error = db::connect(&uri.profile, &user, uri.password.as_ref(), |_| {})
        .await
        .expect_err("must fail");
    assert!(error.is_auth_failure(), "{error:?}");
    assert!(!format!("{error:?}").contains("definitely-wrong-pw-123"));
}

#[tokio::test]
async fn an_unreachable_server_fails_with_a_message() {
    let url = database!();
    let mut uri = parse_uri(&url).expect("parse");
    uri.profile.port = 1;
    let user = uri.profile.user.clone().expect("user");
    let error = db::connect(&uri.profile, &user, None, |_| {})
        .await
        .expect_err("must fail");
    assert!(!error.message.is_empty());
}

#[tokio::test]
async fn null_is_not_an_empty_string() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(
        &session,
        "select null::text as n, ''::text as e, 'x' as v",
        10,
    )
    .await;
    assert!(out.error.is_none());
    let set = &out.sets[0];
    assert_eq!(set.columns, ["n", "e", "v"]);
    assert_eq!(
        set.rows[0],
        [None, Some(String::new()), Some("x".to_string())]
    );
    assert_eq!(set.tag, "SELECT 1");
}

#[tokio::test]
async fn a_select_with_no_rows_still_has_its_columns() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(&session, "select 1 as a, 2 as b where false", 10).await;
    let set = &out.sets[0];
    assert_eq!(
        (set.columns.len(), set.rows.len(), set.total_rows),
        (2, 0, 0)
    );
}

#[tokio::test]
async fn several_statements_give_one_result_each() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let sql = "create temp table t (a int); insert into t values (1),(2),(3); select a from t order by a; \
               update t set a = a + 1; delete from t where a > 3; select count(*) from t";
    let out = db::run(&session, sql, 10).await;
    assert!(out.error.is_none(), "{:?}", out.error);
    let tags: Vec<&str> = out.sets.iter().map(|s| s.tag.as_str()).collect();
    assert_eq!(
        tags,
        [
            "CREATE TABLE",
            "INSERT 0 3",
            "SELECT 3",
            "UPDATE 3",
            "DELETE 1",
            "SELECT 1"
        ]
    );
    assert_eq!(out.sets[5].rows[0][0].as_deref(), Some("2"));
}

#[tokio::test]
async fn an_error_keeps_earlier_results_and_reports_a_position() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(
        &session,
        "select 1; select * from no_such_table_xyz; select 3",
        10,
    )
    .await;
    assert_eq!(out.sets.len(), 1, "the statement before the error is kept");
    let error = out.error.expect("error");
    assert_eq!(error.sqlstate.as_deref(), Some("42P01"));
    assert!(error.message.contains("no_such_table_xyz"));
    assert!(error.position.is_some());
    // The connection is still usable afterwards.
    let again = db::run(&session, "select 42", 10).await;
    assert_eq!(again.sets[0].rows[0][0].as_deref(), Some("42"));
}

#[tokio::test]
async fn syntax_errors_report_the_hint_fields() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(&session, "selct 1", 10).await;
    let error = out.error.expect("error");
    assert_eq!(error.sqlstate.as_deref(), Some("42601"));
    assert_eq!(error.position, Some(1));
}

#[tokio::test]
async fn memory_is_bounded_by_the_row_limit_but_the_count_is_exact() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(&session, "select g from generate_series(1, 5000) g", 100).await;
    let set = &out.sets[0];
    assert_eq!(set.rows.len(), 100);
    assert_eq!(set.total_rows, 5000);
    assert!(set.truncated());
    assert_eq!(set.rows[99][0].as_deref(), Some("100"));
}

#[tokio::test]
async fn very_long_values_are_clipped() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(&session, "select repeat('x', 100000)", 10).await;
    let cell = out.sets[0].rows[0][0].as_deref().expect("value");
    assert!(cell.len() < 100_000 && cell.ends_with('…'));
}

#[tokio::test]
async fn a_running_statement_can_be_cancelled() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let runner = {
        let session = session.clone();
        tokio::spawn(async move { db::run(&session, "select pg_sleep(30)", 10).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    db::cancel(&session).await.expect("cancel request");
    let out = tokio::time::timeout(Duration::from_secs(5), runner)
        .await
        .expect("the run ends soon after cancelling")
        .expect("task");
    let error = out.error.expect("error");
    assert!(error.is_cancellation(), "{error:?}");
    assert!(out.elapsed < Duration::from_secs(5));
    // And the connection works again.
    let again = db::run(&session, "select 1", 10).await;
    assert!(again.error.is_none());
}

#[tokio::test]
async fn text_values_round_trip_unicode_and_odd_types() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(
        &session,
        "select 'naïve 日本 💡', now() is not null, '{1,2}'::int[], '{\"a\":1}'::jsonb, 1.50::numeric, '\\xdead'::bytea",
        10,
    )
    .await;
    assert!(out.error.is_none(), "{:?}", out.error);
    let row = &out.sets[0].rows[0];
    assert_eq!(row[0].as_deref(), Some("naïve 日本 💡"));
    assert_eq!(row[1].as_deref(), Some("t"));
    assert_eq!(row[2].as_deref(), Some("{1,2}"));
    assert_eq!(row[3].as_deref(), Some("{\"a\": 1}"));
    assert_eq!(row[4].as_deref(), Some("1.50"));
    assert_eq!(row[5].as_deref(), Some("\\xdead"));
}

#[tokio::test]
async fn a_dropped_connection_is_reported_once() {
    let url = database!();
    let uri = parse_uri(&url).expect("parse");
    let user = uri.profile.user.clone().expect("user");
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (session, _) = db::connect(&uri.profile, &user, uri.password.as_ref(), move |reason| {
        let _ = tx.send(reason);
    })
    .await
    .expect("connect");
    // A second connection ends the first one's backend.
    let (other, _) = connect(&url).await;
    let out = db::run(&session, "select pg_backend_pid()", 1).await;
    let pid = out.sets[0].rows[0][0].clone().expect("pid");
    let kill = db::run(&other, &format!("select pg_terminate_backend({pid})"), 1).await;
    assert!(kill.error.is_none());
    let reason = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("reported")
        .expect("sent");
    assert!(!reason.is_empty());
}
