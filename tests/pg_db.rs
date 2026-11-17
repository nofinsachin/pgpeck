//! Integration tests against a real PostgreSQL. They run only when
//! `PGPECK_TEST_DATABASE_URL` is set, for example:
//!
//! ```text
//! PGPECK_TEST_DATABASE_URL=postgres://pgpeck:pgpeck@127.0.0.1:54329/pgpeck cargo test --test pg_db
//! ```

use std::time::Duration;

use pgpeck::db::{self, Session};
use pgpeck::profile::parse_uri;

fn url() -> Option<String> {
    std::env::var("PGPECK_TEST_DATABASE_URL")
        .ok()
        .filter(|u| !u.is_empty())
}

/// Skips the test when no database is configured.
macro_rules! database {
    () => {
        match url() {
            Some(url) => url,
            None => {
                eprintln!("skipped: PGPECK_TEST_DATABASE_URL is not set");
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
async fn the_connection_is_named_pgpeck() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(
        &session,
        "select current_setting('application_name')",
        10,
        &Default::default(),
    )
    .await;
    assert_eq!(
        out.sets[0].rows[0][0].as_deref(),
        Some(db::APPLICATION_NAME)
    );
}

#[tokio::test]
async fn a_wrong_password_is_an_auth_failure_that_does_not_echo_it() {
    let url = database!();
    let mut uri = parse_uri(&url).expect("parse");
    uri.password = Some(pgpeck::secrets::Secret::new("definitely-wrong-pw-123"));
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
        &Default::default(),
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
    let out = db::run(
        &session,
        "select 1 as a, 2 as b where false",
        10,
        &Default::default(),
    )
    .await;
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
    let out = db::run(&session, sql, 10, &Default::default()).await;
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
        &Default::default(),
    )
    .await;
    assert_eq!(out.sets.len(), 1, "the statement before the error is kept");
    let error = out.error.expect("error");
    assert_eq!(error.sqlstate.as_deref(), Some("42P01"));
    assert!(error.message.contains("no_such_table_xyz"));
    assert!(error.position.is_some());
    // The connection is still usable afterwards.
    let again = db::run(&session, "select 42", 10, &Default::default()).await;
    assert_eq!(again.sets[0].rows[0][0].as_deref(), Some("42"));
}

#[tokio::test]
async fn syntax_errors_report_the_hint_fields() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(&session, "selct 1", 10, &Default::default()).await;
    let error = out.error.expect("error");
    assert_eq!(error.sqlstate.as_deref(), Some("42601"));
    assert_eq!(error.position, Some(1));
}

#[tokio::test]
async fn memory_is_bounded_by_the_row_limit_but_the_count_is_exact() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(
        &session,
        "select g from generate_series(1, 5000) g",
        100,
        &Default::default(),
    )
    .await;
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
    let out = db::run(
        &session,
        "select repeat('x', 100000)",
        10,
        &Default::default(),
    )
    .await;
    let cell = out.sets[0].rows[0][0].as_deref().expect("value");
    assert!(cell.len() < 100_000 && cell.ends_with('…'));
}

#[tokio::test]
async fn a_running_statement_can_be_cancelled() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let runner = {
        let session = session.clone();
        tokio::spawn(async move {
            db::run(&session, "select pg_sleep(30)", 10, &Default::default()).await
        })
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
    let again = db::run(&session, "select 1", 10, &Default::default()).await;
    assert!(again.error.is_none());
}

#[tokio::test]
async fn text_values_round_trip_unicode_and_odd_types() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(
        &session,
        "select 'naïve 日本 💡', now() is not null, '{1,2}'::int[], '{\"a\":1}'::jsonb, 1.50::numeric, '\\xdead'::bytea",
        10, &Default::default())
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
    let out = db::run(&session, "select pg_backend_pid()", 1, &Default::default()).await;
    let pid = out.sets[0].rows[0][0].clone().expect("pid");
    let kill = db::run(
        &other,
        &format!("select pg_terminate_backend({pid})"),
        1,
        &Default::default(),
    )
    .await;
    assert!(kill.error.is_none());
    let reason = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("reported")
        .expect("sent");
    assert!(!reason.is_empty());
}

#[tokio::test]
async fn the_transaction_state_is_reported_after_each_run() {
    use pgpeck::db::TxState;
    let url = database!();
    let (session, _) = connect(&url).await;

    let out = db::run(&session, "select 1", 10, &Default::default()).await;
    assert_eq!(out.tx, TxState::Idle);

    let out = db::run(&session, "begin", 10, &Default::default()).await;
    assert_eq!(out.tx, TxState::Open, "an empty transaction counts");

    let out = db::run(&session, "select 1", 10, &Default::default()).await;
    assert_eq!(out.tx, TxState::Open);

    let out = db::run(&session, "select 1/0", 10, &Default::default()).await;
    assert!(out.error.is_some());
    assert_eq!(out.tx, TxState::Failed);

    let out = db::run(&session, "select 1", 10, &Default::default()).await;
    assert_eq!(out.tx, TxState::Failed, "stays failed until rolled back");

    let out = db::run(&session, "rollback", 10, &Default::default()).await;
    assert_eq!(out.tx, TxState::Idle);

    let out = db::run(&session, "begin; select 1; commit", 10, &Default::default()).await;
    assert_eq!(out.tx, TxState::Idle);

    // A multi-statement run that fails outside BEGIN leaves nothing open.
    let out = db::run(&session, "select 1; select 1/0", 10, &Default::default()).await;
    assert_eq!(out.tx, TxState::Idle);
}

#[tokio::test]
async fn statistics_count_only_the_connected_database() {
    let url = database!();
    let (admin, _) = connect(&url).await;
    let basic = db::stats(&admin).await.expect("stats");
    assert!(basic.connections >= 1 && basic.connections <= basic.server_connections);
    assert!(basic.max_connections >= basic.server_connections);
    assert!(basic.size_bytes > 0);
    assert!(basic.uptime_secs < 10 * 365 * 86400);
    if basic.sees_all_sessions {
        assert!(
            basic.active.expect("active") >= 1,
            "this session runs the query"
        );
    }

    // Other tests share the server, so count in a database of our own.
    let name = format!("pgpeck_stats_{}", std::process::id());
    let created = db::run(
        &admin,
        &format!("create database {name}"),
        1,
        &Default::default(),
    )
    .await;
    if created.error.is_some() {
        eprintln!("skipped the per-database part: this role cannot create databases");
        return;
    }
    let mut profile = parse_uri(&url).expect("url").profile;
    profile.database = Some(name.clone());
    let user = profile.user.clone().expect("user");
    let password = parse_uri(&url).expect("url").password;
    let (mine, _) = db::connect(&profile, &user, password.as_ref(), |_| {})
        .await
        .expect("connect to the new database");
    let (_second, _) = db::connect(&profile, &user, password.as_ref(), |_| {})
        .await
        .expect("second connection");

    let stats = db::stats(&mine).await.expect("stats");
    assert_eq!(stats.connections, 2, "only this database's connections");
    assert!(
        stats.server_connections > stats.connections,
        "the server count includes the other databases: {stats:?}"
    );
    assert!(stats.size_bytes > 0);

    drop(mine);
    drop(_second);
    let dropped = db::run(
        &admin,
        &format!("drop database {name} with (force)"),
        1,
        &Default::default(),
    )
    .await;
    assert!(dropped.error.is_none(), "{:?}", dropped.error);
}

#[tokio::test]
async fn statistics_do_not_disturb_a_transaction() {
    use pgpeck::db::TxState;
    let url = database!();
    let (session, _) = connect(&url).await;
    db::run(&session, "begin", 10, &Default::default()).await;
    // The app skips statistics inside a transaction, but reading them must
    // still leave it open and usable.
    db::stats(&session).await.expect("stats");
    let out = db::run(&session, "select 1", 10, &Default::default()).await;
    assert_eq!(out.tx, TxState::Open);
    db::run(&session, "rollback", 10, &Default::default()).await;
}

#[tokio::test]
async fn a_plain_role_gets_numbers_but_not_other_roles_activity() {
    let url = database!();
    let (admin, info) = connect(&url).await;
    let role = format!("pgpeck_plain_{}", std::process::id());
    let created = db::run(
        &admin,
        &format!("create role {role} login password 'plain-pw'"),
        1,
        &Default::default(),
    )
    .await;
    if created.error.is_some() {
        eprintln!("skipped: this role cannot create roles");
        return;
    }
    let mut profile = parse_uri(&url).expect("url").profile;
    profile.database = Some(info.database.clone());
    let password = pgpeck::secrets::Secret::new("plain-pw");
    let result = db::connect(&profile, &role, Some(&password), |_| {}).await;
    let stats = match &result {
        Ok((session, _)) => Some(db::stats(session).await),
        Err(_) => None,
    };
    drop(result);
    db::run(&admin, &format!("drop role {role}"), 1, &Default::default()).await;
    let Some(stats) = stats else {
        eprintln!("skipped: the server does not let the new role log in");
        return;
    };
    let stats = stats.expect("a plain role can read the statistics");
    assert!(!stats.sees_all_sessions);
    assert!(stats.connections >= 1 && stats.size_bytes > 0);
}

#[tokio::test]
async fn the_catalog_lists_user_objects_with_their_schemas() {
    use pgpeck::catalog::Kind;
    let url = database!();
    let (session, _) = connect(&url).await;
    let setup = "
        drop schema if exists oxp_cat cascade;
        create schema oxp_cat;
        create table oxp_cat.\"Mixed Case\" (id integer primary key, note text);
        create view oxp_cat.v as select id from oxp_cat.\"Mixed Case\";
        create materialized view oxp_cat.m as select id from oxp_cat.\"Mixed Case\";
        create function oxp_cat.f(a integer, b text) returns integer language sql as 'select 1';
        create type oxp_cat.mood as enum ('ok', 'bad');";
    let out = db::run(&session, setup, 100, &Default::default()).await;
    assert!(out.error.is_none(), "{:?}", out.error);

    let catalog = db::catalog(&session).await.expect("catalog");
    let find = |kind: Kind, name: &str| {
        catalog
            .objects
            .iter()
            .find(|o| o.kind == kind && o.schema == "oxp_cat" && o.name == name)
    };
    assert!(find(Kind::Schema, "oxp_cat").is_some());
    assert!(find(Kind::Table, "Mixed Case").is_some());
    assert!(find(Kind::View, "v").is_some());
    assert!(find(Kind::MaterializedView, "m").is_some());
    assert!(find(Kind::Type, "mood").is_some());
    let function = find(Kind::Function, "f").expect("function");
    assert_eq!(function.detail, "a integer, b text");
    let column = find(Kind::Column, "note").expect("column");
    assert_eq!(column.parent.as_deref(), Some("Mixed Case"));
    assert_eq!(column.detail, "text");
    assert_eq!(column.sql_name(), "\"Mixed Case\".note");
    assert!(
        catalog
            .objects
            .iter()
            .all(|o| !o.schema.starts_with("pg_") && o.schema != "information_schema"),
        "system schemas are left out"
    );

    let out = db::run(
        &session,
        "drop schema oxp_cat cascade",
        100,
        &Default::default(),
    )
    .await;
    assert!(out.error.is_none(), "{:?}", out.error);
}

#[tokio::test]
async fn the_largest_tables_are_listed_biggest_first() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let setup = "
        drop schema if exists oxp_top cascade;
        create schema oxp_top;
        create table oxp_top.small as select generate_series(1, 10) as n;
        create table oxp_top.big as select generate_series(1, 200000) as n, md5(random()::text) as t;";
    let out = db::run(&session, setup, 100, &Default::default()).await;
    assert!(out.error.is_none(), "{:?}", out.error);

    let stats = db::stats(&session).await.expect("stats");
    assert!(stats.top_tables.len() <= db::TOP_TABLES);
    let sizes: Vec<u64> = stats.top_tables.iter().map(|t| t.bytes).collect();
    assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
    let position = |name: &str| stats.top_tables.iter().position(|t| t.name == name);
    let big = position("big").expect("the big table is among the largest");
    assert!(position("small").is_none_or(|small| big < small));
    assert!(stats
        .top_tables
        .iter()
        .all(|t| !t.schema.starts_with("pg_") && t.schema != "information_schema"));

    let out = db::run(
        &session,
        "drop schema oxp_top cascade",
        100,
        &Default::default(),
    )
    .await;
    assert!(out.error.is_none(), "{:?}", out.error);
}

#[tokio::test]
async fn a_large_query_is_paged_through_a_cursor() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let paging = db::Paging {
        cursor: Some("pgpeck_test_paged".to_string()),
        stale: None,
    };
    let out = db::run(
        &session,
        "select g as n, g * 2 as double from generate_series(1, 250) g",
        100,
        &paging,
    )
    .await;
    assert!(out.error.is_none(), "{:?}", out.error);
    let set = &out.sets[0];
    assert_eq!(set.columns, ["n", "double"]);
    assert_eq!((set.rows.len(), set.total_rows, set.offset), (100, 250, 0));
    assert!(set.is_paged() && !set.truncated());
    assert_eq!(set.rows[0][0].as_deref(), Some("1"));
    assert_eq!(set.tag, "SELECT 250");
    let cursor = set.cursor.clone().expect("cursor");

    // Any window can be read, in any order, and the end is short.
    let (_, rows) = db::fetch_page(&session, &cursor, 200, 100)
        .await
        .expect("last window");
    assert_eq!(rows.len(), 50);
    assert_eq!(rows[0][0].as_deref(), Some("201"));
    assert_eq!(rows[49][1].as_deref(), Some("500"));
    let (_, rows) = db::fetch_page(&session, &cursor, 100, 100)
        .await
        .expect("middle window");
    assert_eq!(rows[0][0].as_deref(), Some("101"));
    let (_, rows) = db::fetch_page(&session, &cursor, 0, 100)
        .await
        .expect("first window");
    assert_eq!(rows[0][0].as_deref(), Some("1"));

    // The next run closes the old cursor, which then cannot be read.
    let next = db::Paging {
        cursor: Some("pgpeck_test_paged_2".to_string()),
        stale: Some(cursor.clone()),
    };
    let out = db::run(&session, "select 1", 100, &next).await;
    assert!(out.error.is_none());
    assert!(db::fetch_page(&session, &cursor, 0, 10).await.is_err());
}

#[tokio::test]
async fn small_results_and_other_statements_are_not_paged() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let paging = db::Paging {
        cursor: Some("pgpeck_test_small".to_string()),
        stale: None,
    };
    // Fits in one window: no cursor is kept.
    let out = db::run(
        &session,
        "select g from generate_series(1, 5) g",
        100,
        &paging,
    )
    .await;
    let set = &out.sets[0];
    assert_eq!((set.rows.len(), set.total_rows), (5, 5));
    assert!(!set.is_paged());
    // An empty result still has its columns.
    let out = db::run(&session, "select 1 as a where false", 100, &paging).await;
    assert_eq!(out.sets[0].columns, ["a"]);
    // Several statements, and anything that changes data, run as before.
    let out = db::run(&session, "select 1; select 2", 100, &paging).await;
    assert_eq!(out.sets.len(), 2);
    let out = db::run(
        &session,
        "select g into temp pgpeck_tmp from generate_series(1, 300) g",
        100,
        &paging,
    )
    .await;
    assert!(out.error.is_none(), "{:?}", out.error);
    // A bad query reports its own error, not one about cursors.
    let out = db::run(&session, "select * from no_such_table_here", 100, &paging).await;
    let message = out.error.expect("error").message;
    assert!(message.contains("no_such_table_here"), "{message}");
    // Too long for the window, and not pageable: the old behaviour.
    let out = db::run(
        &session,
        "select g from generate_series(1, 300) g; select 1",
        100,
        &paging,
    )
    .await;
    assert!(out.sets[0].truncated() && !out.sets[0].is_paged());
}

#[tokio::test]
async fn foreign_keys_are_read_with_their_columns_in_both_directions() {
    let url = database!();
    let (session, _) = connect(&url).await;
    let out = db::run(
        &session,
        "drop schema if exists pgpeck_fk_test cascade;
         create schema pgpeck_fk_test;
         create table pgpeck_fk_test.parent (a int, b int, primary key (a, b));
         create table pgpeck_fk_test.child (
             id int primary key, pa int, pb int,
             constraint child_parent_fk foreign key (pa, pb)
                 references pgpeck_fk_test.parent (a, b));
         create table pgpeck_fk_test.selfref (
             id int primary key, up int references pgpeck_fk_test.selfref (id))",
        10,
        &Default::default(),
    )
    .await;
    assert!(out.error.is_none(), "{:?}", out.error);
    let catalog = db::catalog(&session).await.expect("catalog");

    let from_child = catalog.references_from("pgpeck_fk_test", "child");
    assert_eq!(from_child.len(), 1);
    let key = from_child[0];
    assert_eq!(key.name, "child_parent_fk");
    assert_eq!(key.columns, ["pa", "pb"]);
    assert_eq!(
        (key.ref_schema.as_str(), key.ref_table.as_str()),
        ("pgpeck_fk_test", "parent")
    );
    assert_eq!(key.ref_columns, ["a", "b"]);
    let to_parent = catalog.referenced_by("pgpeck_fk_test", "parent");
    assert_eq!(to_parent.len(), 1);
    assert_eq!(to_parent[0].table, "child");
    // A table that points at itself is both ends of its own key.
    assert_eq!(
        catalog.references_from("pgpeck_fk_test", "selfref").len(),
        1
    );
    assert_eq!(catalog.referenced_by("pgpeck_fk_test", "selfref").len(), 1);

    let out = db::run(
        &session,
        "drop schema pgpeck_fk_test cascade",
        10,
        &Default::default(),
    )
    .await;
    assert!(out.error.is_none(), "{:?}", out.error);
}
