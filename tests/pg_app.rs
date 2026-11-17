//! End to end: keystrokes in, pixels out, against a real PostgreSQL.
//!
//! These drive the real `App` and `Runtime` and read the rendered screen, so
//! they check what a user would see. They run only when
//! `PGPECK_TEST_DATABASE_URL` is set.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pgpeck::app::{App, ConnStatus, Dialog};
use pgpeck::config::Config;
use pgpeck::events::Event;
use pgpeck::keymap::Keymap;
use pgpeck::profile::{parse_uri, Profile};
use pgpeck::runtime::Runtime;
use pgpeck::secrets::Source;
use pgpeck::theme::Theme;
use pgpeck::ui;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::mpsc::UnboundedReceiver;

fn url() -> Option<String> {
    std::env::var("PGPECK_TEST_DATABASE_URL")
        .ok()
        .filter(|u| !u.is_empty())
}

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

const WIDTH: u16 = 110;
const HEIGHT: u16 = 32;

struct Harness {
    app: App,
    rx: UnboundedReceiver<Event>,
    runtime: Runtime,
}

impl Harness {
    fn new() -> Self {
        Self::with(vec![Source::Env, Source::Prompt], 10_000, None)
    }

    fn with(
        sources: Vec<Source>,
        row_limit: usize,
        profiles_path: Option<std::path::PathBuf>,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut config = Config::default();
        config.connections.password_sources = sources.clone();
        let runtime = Runtime::new(tx, sources, row_limit, profiles_path);
        let mut app = App::new(config, Keymap::default_keymap());
        let commands = app.update(Event::Resize(WIDTH, HEIGHT));
        assert!(commands.is_empty());
        Self { app, rx, runtime }
    }

    fn feed(&mut self, event: Event) {
        for command in self.app.update(event) {
            self.runtime.execute(command);
        }
    }

    fn key(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.feed(Event::Key(KeyEvent::new(code, mods)));
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.key(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    fn enter(&mut self) {
        self.key(KeyCode::Enter, KeyModifiers::NONE);
    }

    /// Processes background events until `done` is true, or fails after 10 s.
    async fn until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !done(&self.app) {
            let event = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}\n{}", self.screen()))
                .expect("event channel closed");
            self.feed(event);
        }
    }

    async fn connect_with_uri(&mut self, url: &str) {
        self.key(KeyCode::Char('c'), KeyModifiers::NONE);
        self.type_text(url);
        self.enter();
        self.until("the connection", |a| {
            a.state
                .active_connection()
                .is_some_and(|c| c.status == ConnStatus::Connected)
        })
        .await;
    }

    /// Runs the text in the editor and waits for the answer.
    async fn run(&mut self, sql: &str) {
        self.key(KeyCode::Char('a'), KeyModifiers::CONTROL);
        self.key(KeyCode::Delete, KeyModifiers::NONE);
        self.feed(Event::Paste(sql.to_string()));
        // With the text selected, everything runs, not just one statement.
        self.key(KeyCode::Char('a'), KeyModifiers::CONTROL);
        self.key(KeyCode::Char('r'), KeyModifiers::CONTROL);
        self.until("the results", |a| {
            a.state.running.is_none() && a.state.doc().results.is_some()
        })
        .await;
    }

    fn screen(&self) -> String {
        let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal");
        terminal
            .draw(|f| ui::draw(f, &self.app, &Theme::default()))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..HEIGHT)
            .map(|y| {
                (0..WIDTH)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The test URL without its password.
fn url_without_password(url: &str) -> Profile {
    let mut profile = parse_uri(url).expect("parse").profile;
    profile.name = "test".to_string();
    profile
}

fn password_of(url: &str) -> String {
    parse_uri(url)
        .expect("parse")
        .password
        .map(|p| p.expose().to_string())
        .expect("the test URL has a password")
}

#[tokio::test]
async fn connect_run_and_see_a_grid() {
    let url = database!();
    let mut h = Harness::new();
    assert!(h.screen().contains("Press C to connect to a database."));

    h.connect_with_uri(&url).await;
    let screen = h.screen();
    assert!(screen.contains("no TLS"), "{screen}");
    assert!(
        screen.contains("PostgreSQL 1"),
        "the context pane shows the server:\n{screen}"
    );

    h.run("select 1 as n, null as missing, '' as empty, 'text' as word")
        .await;
    let screen = h.screen();
    for expected in [
        "n │ missing │ empty │ word",
        "NULL",
        "SELECT 1  ·  1 row",
        "text",
    ] {
        assert!(screen.contains(expected), "missing {expected:?}:\n{screen}");
    }
    assert!(
        !screen.contains(&password_of(&url)),
        "the password is never drawn:\n{screen}"
    );
}

#[tokio::test]
async fn the_statement_under_the_cursor_runs_not_the_whole_buffer() {
    let url = database!();
    let mut h = Harness::new();
    h.connect_with_uri(&url).await;
    h.key(KeyCode::Char('a'), KeyModifiers::CONTROL);
    h.key(KeyCode::Delete, KeyModifiers::NONE);
    h.feed(Event::Paste(
        "select 'first' as a;\nselect 'second' as b;\nselect 'third' as c".to_string(),
    ));
    h.key(KeyCode::Up, KeyModifiers::NONE);
    h.key(KeyCode::Char('r'), KeyModifiers::CONTROL);
    h.until("the results", |a| a.state.doc().results.is_some())
        .await;
    let screen = h.screen();
    // The editor still shows all three lines, so look at the results pane
    // only: the lines from its title down to the editor's title.
    let results: String = screen
        .lines()
        .skip_while(|l| !l.contains("┌ Results"))
        .take_while(|l| !l.contains("Editor"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(results.contains("second"), "{screen}");
    assert!(
        !results.contains("first") && !results.contains("third"),
        "{screen}"
    );
}

#[tokio::test]
async fn several_statements_make_tabs() {
    let url = database!();
    let mut h = Harness::new();
    h.connect_with_uri(&url).await;
    h.run("create temp table t (a int); insert into t values (1),(2); select a from t order by a")
        .await;
    let screen = h.screen();
    for expected in [
        "1 CREATE TABLE",
        "2 INSERT 0 2",
        "3 SELECT 2",
        "SELECT 2  ·  2 rows",
    ] {
        assert!(screen.contains(expected), "missing {expected:?}:\n{screen}");
    }
    // Switch to the INSERT tab in the results pane.
    h.key(KeyCode::Char('3'), KeyModifiers::ALT);
    h.key(KeyCode::Char('['), KeyModifiers::NONE);
    assert!(h.screen().contains("INSERT 0 2  ·  "), "{}", h.screen());
}

#[tokio::test]
async fn an_error_shows_its_code_message_and_position() {
    let url = database!();
    let mut h = Harness::new();
    h.connect_with_uri(&url).await;
    h.run("select * from no_such_table_xyz").await;
    let screen = h.screen();
    assert!(
        screen.contains("ERROR 42P01: relation \"no_such_table_xyz\" does not exist"),
        "{screen}"
    );
    assert!(screen.contains("Position: character 15"), "{screen}");
    // The buffer and connection are fine, and the next statement works.
    h.run("select 42 as answer").await;
    assert!(h.screen().contains("42"));
}

#[tokio::test]
async fn the_row_limit_is_announced() {
    let url = database!();
    let mut h = Harness::with(vec![Source::Env, Source::Prompt], 50, None);
    h.connect_with_uri(&url).await;
    h.run("select g from generate_series(1, 200) g").await;
    assert!(
        h.screen()
            .contains("showing the first 50 of 200 rows (row limit)"),
        "{}",
        h.screen()
    );
}

#[tokio::test]
async fn a_long_statement_shows_progress_and_can_be_cancelled() {
    let url = database!();
    let mut h = Harness::new();
    h.connect_with_uri(&url).await;
    h.key(KeyCode::Char('a'), KeyModifiers::CONTROL);
    h.key(KeyCode::Delete, KeyModifiers::NONE);
    h.feed(Event::Paste("select pg_sleep(30)".to_string()));
    h.key(KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert!(h.app.state.running.is_some());
    let screen = h.screen();
    assert!(
        screen.contains("Running…") && screen.contains("Ctrl+C cancels"),
        "{screen}"
    );

    // The editor stays usable meanwhile.
    h.type_text(" -- typing while running");
    assert!(h
        .app
        .state
        .editor()
        .text()
        .ends_with("typing while running"));

    let started = std::time::Instant::now();
    h.key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    h.until("the cancellation", |a| a.state.running.is_none())
        .await;
    assert!(started.elapsed() < Duration::from_secs(5));
    let screen = h.screen();
    assert!(
        screen.contains("ERROR 57014") && screen.contains("Query cancelled"),
        "{screen}"
    );
}

#[tokio::test]
async fn a_saved_profile_asks_for_the_password_and_remembers_nothing() {
    let url = database!();
    let mut h = Harness::with(vec![Source::Prompt], 10_000, None);
    h.app
        .state
        .profiles
        .add(url_without_password(&url))
        .expect("add");
    h.key(KeyCode::Char('c'), KeyModifiers::NONE);
    h.type_text("test");
    h.enter();
    h.until("the password prompt", |a| {
        matches!(a.state.dialog, Some(Dialog::Password(_)))
    })
    .await;
    let screen = h.screen();
    assert!(screen.contains("Password for"), "{screen}");

    let password = password_of(&url);
    h.type_text(&password);
    let screen = h.screen();
    assert!(
        !screen.contains(&password),
        "the typed password is masked:\n{screen}"
    );
    assert!(screen.contains(&"•".repeat(password.len())));
    h.enter();
    h.until("the connection", |a| {
        a.state
            .active_connection()
            .is_some_and(|c| c.status == ConnStatus::Connected)
    })
    .await;
    assert!(h.app.state.dialog.is_none());
}

#[tokio::test]
async fn a_wrong_password_is_rejected_and_asked_for_again() {
    let url = database!();
    let mut h = Harness::with(vec![Source::Prompt], 10_000, None);
    h.app
        .state
        .profiles
        .add(url_without_password(&url))
        .expect("add");
    h.key(KeyCode::Char('c'), KeyModifiers::NONE);
    h.type_text("test");
    h.enter();
    h.until("the prompt", |a| {
        matches!(a.state.dialog, Some(Dialog::Password(_)))
    })
    .await;
    h.type_text("not-the-password");
    h.enter();
    h.until(
        "the rejection",
        |a| matches!(&a.state.dialog, Some(Dialog::Password(p)) if p.error.is_some()),
    )
    .await;
    let screen = h.screen();
    assert!(
        screen.contains("Rejected: password authentication failed"),
        "{screen}"
    );
    assert!(!screen.contains("not-the-password"));

    h.type_text(&password_of(&url));
    h.enter();
    h.until("the connection", |a| {
        a.state
            .active_connection()
            .is_some_and(|c| c.status == ConnStatus::Connected)
    })
    .await;
}

#[tokio::test]
async fn an_environment_password_is_used_without_asking() {
    let url = database!();
    let mut h = Harness::with(vec![Source::Env, Source::Prompt], 10_000, None);
    let mut profile = url_without_password(&url);
    profile.password_env = Some("PGPECK_TEST_PW_FOR_PROFILE".to_string());
    h.app.state.profiles.add(profile).expect("add");
    // The runtime reads the real environment, so set the variable for this process.
    std::env::set_var("PGPECK_TEST_PW_FOR_PROFILE", password_of(&url));
    h.key(KeyCode::Char('c'), KeyModifiers::NONE);
    h.type_text("test");
    h.enter();
    h.until("the connection", |a| {
        a.state
            .active_connection()
            .is_some_and(|c| c.status == ConnStatus::Connected)
    })
    .await;
    assert!(h.app.state.dialog.is_none(), "no prompt was needed");
}

#[tokio::test]
async fn a_dropped_connection_is_reported_and_reconnect_works() {
    let url = database!();
    let mut h = Harness::new();
    h.connect_with_uri(&url).await;
    h.run("select pg_backend_pid()::text as pid").await;
    let pid = h
        .app
        .state
        .doc()
        .results
        .as_ref()
        .expect("results")
        .output
        .sets[0]
        .rows[0][0]
        .clone()
        .expect("pid");

    // Another session ends this one's backend.
    let uri = parse_uri(&url).expect("parse");
    let user = uri.profile.user.clone().expect("user");
    let (other, _) = pgpeck::db::connect(&uri.profile, &user, uri.password.as_ref(), |_| {})
        .await
        .expect("second connection");
    let kill = pgpeck::db::run(&other, &format!("select pg_terminate_backend({pid})"), 1).await;
    assert!(kill.error.is_none());

    h.until("the loss to be noticed", |a| {
        a.state
            .active_connection()
            .is_some_and(|c| matches!(c.status, ConnStatus::Closed(_)))
    })
    .await;
    let screen = h.screen();
    assert!(
        screen.contains("disconnected") && screen.contains("Press Alt+O to reconnect"),
        "{screen}"
    );
    assert!(
        screen.contains("pg_backend_pid"),
        "the editor text is kept:\n{screen}"
    );

    // The simple way back: Esc to the commands pane, then C, then Enter on "Reconnect".
    h.key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        h.screen().contains("disconnected") && h.screen().contains("C connect"),
        "{}",
        h.screen()
    );
    h.key(KeyCode::Char('c'), KeyModifiers::NONE);
    assert!(h.screen().contains("▶ Reconnect to"), "{}", h.screen());
    // The password came from the URI and was not kept, so reconnecting asks again.
    h.enter();
    h.until("the password prompt", |a| {
        matches!(a.state.dialog, Some(Dialog::Password(_)))
    })
    .await;
    h.type_text(&password_of(&url));
    h.enter();
    h.until("the new connection", |a| {
        a.state
            .active_connection()
            .is_some_and(|c| c.status == ConnStatus::Connected)
    })
    .await;
    h.run("select 'back' as state").await;
    assert!(h.screen().contains("back"));
}

#[tokio::test]
async fn an_unreachable_server_fails_clearly_and_does_not_hang_the_ui() {
    let url = database!();
    let mut h = Harness::new();
    let good = parse_uri(&url).expect("parse").profile;
    let broken = format!(
        "postgres://{}:any-password@{}:1/{}",
        good.user.as_deref().unwrap_or("u"),
        good.host,
        good.database.as_deref().unwrap_or("d")
    );
    h.key(KeyCode::Char('c'), KeyModifiers::NONE);
    h.type_text(&broken);
    h.enter();
    // The UI answers at once, even though the connection is still in progress.
    assert!(h.screen().contains("Connecting to"));
    h.type_text("x");
    h.until("the failure", |a| {
        a.state
            .active_connection()
            .is_some_and(|c| matches!(c.status, ConnStatus::Failed(_)))
    })
    .await;
    assert!(h.screen().contains("connection failed"), "{}", h.screen());
}

#[tokio::test]
async fn saving_a_profile_writes_a_private_file_without_the_password() {
    use std::os::unix::fs::PermissionsExt;
    let url = database!();
    let dir = std::env::temp_dir().join(format!("pgpeck-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("connections.toml");
    let mut h = Harness::with(vec![Source::Prompt], 10_000, Some(path.clone()));
    h.key(KeyCode::Char('c'), KeyModifiers::NONE);
    h.type_text(&url);
    h.key(KeyCode::Char('s'), KeyModifiers::CONTROL);

    // The runtime writes the file in the background.
    for _ in 0..100 {
        if path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(path.exists(), "the profile file was never written");
    let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(
        text.contains("127.0.0.1") && !text.contains(&password_of(&url)),
        "{text}"
    );
    // The picker screen never shows the password either.
    assert!(!h.screen().contains(&password_of(&url)), "{}", h.screen());
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[tokio::test]
async fn connect_by_typing_the_details() {
    let url = database!();
    let good = parse_uri(&url).expect("parse");
    let profile = good.profile;
    let mut h = Harness::with(vec![Source::Prompt], 10_000, None);

    // Landing screen: one key opens the window, Enter picks "New connection".
    h.key(KeyCode::Char('c'), KeyModifiers::NONE);
    h.enter();
    assert!(
        h.screen().contains("Passwords are never saved"),
        "{}",
        h.screen()
    );

    let clear = |h: &mut Harness| h.key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    let tab = |h: &mut Harness| h.key(KeyCode::Tab, KeyModifiers::NONE);
    clear(&mut h);
    h.type_text(&profile.host);
    tab(&mut h);
    clear(&mut h);
    h.type_text(&profile.port.to_string());
    tab(&mut h);
    h.type_text(profile.database.as_deref().expect("database"));
    tab(&mut h);
    clear(&mut h);
    h.type_text(profile.user.as_deref().expect("user"));
    h.enter();

    h.until("the password prompt", |a| {
        matches!(a.state.dialog, Some(Dialog::Password(_)))
    })
    .await;
    h.type_text(&password_of(&url));
    h.enter();
    h.until("the connection", |a| {
        a.state
            .active_connection()
            .is_some_and(|c| c.status == ConnStatus::Connected)
    })
    .await;
    // Straight to the editor, ready to type.
    assert_eq!(h.app.state.focus, pgpeck::app::Pane::Editor);
    h.run("select 'typed by hand' as how").await;
    assert!(h.screen().contains("typed by hand"));
    assert!(
        h.app.state.profiles.is_empty(),
        "nothing was saved without a name"
    );
}

#[tokio::test]
async fn a_connection_made_in_the_form_can_be_saved_and_is_listed_next_time() {
    let url = database!();
    let profile = parse_uri(&url).expect("parse").profile;
    let dir = std::env::temp_dir().join(format!("pgpeck-e2e-form-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("connections.toml");
    let mut h = Harness::with(vec![Source::Prompt], 10_000, Some(path.clone()));

    h.key(KeyCode::Char('c'), KeyModifiers::NONE);
    h.enter();
    h.key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    h.type_text(&profile.host);
    for _ in 0..4 {
        h.key(KeyCode::Tab, KeyModifiers::NONE);
    }
    h.type_text("Test DB");
    h.enter();
    h.until("the prompt", |a| {
        matches!(a.state.dialog, Some(Dialog::Password(_)))
    })
    .await;
    h.key(KeyCode::Esc, KeyModifiers::NONE);

    for _ in 0..100 {
        if path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let text = std::fs::read_to_string(&path).expect("the profile file");
    assert!(
        text.contains("Test DB") && text.contains(&profile.host),
        "{text}"
    );

    // A new session sees it in the list.
    let (loaded, problems) = pgpeck::profile::Profiles::parse(&text);
    assert!(problems.is_empty(), "{problems:?}");
    let mut next = Harness::with(vec![Source::Prompt], 10_000, None);
    next.app.state.profiles = loaded;
    next.key(KeyCode::Char('c'), KeyModifiers::NONE);
    assert!(next.screen().contains("▶ Test DB"), "{}", next.screen());
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[tokio::test]
async fn transactions_are_flagged_and_rolled_back_with_one_key() {
    let url = database!();
    let mut h = Harness::new();
    h.connect_with_uri(&url).await;

    h.run("begin").await;
    let screen = h.screen();
    assert!(screen.contains("TX OPEN"), "{screen}");
    assert!(screen.contains("Transaction open"), "{screen}");

    h.run("select 1/0").await;
    let screen = h.screen();
    assert!(screen.contains("TX FAILED"), "{screen}");
    assert!(screen.contains("rolls back"), "{screen}");

    h.key(KeyCode::Char('z'), KeyModifiers::ALT);
    h.until("the rollback", |a| {
        a.state.running.is_none()
            && a.state
                .active_connection()
                .is_some_and(|c| c.tx == pgpeck::db::TxState::Idle)
    })
    .await;
    let screen = h.screen();
    assert!(
        !screen.contains("TX FAILED") && !screen.contains("TX OPEN"),
        "{screen}"
    );
}

#[tokio::test]
async fn a_delete_without_where_waits_for_a_yes() {
    let url = database!();
    let mut h = Harness::new();
    h.connect_with_uri(&url).await;
    h.run("create temp table pgpeck_confirm (a int)").await;

    h.feed(Event::Paste("delete from pgpeck_confirm".to_string()));
    h.key(KeyCode::Char('a'), KeyModifiers::CONTROL);
    h.key(KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert!(
        h.app.state.running.is_none(),
        "nothing runs before the answer"
    );
    let screen = h.screen();
    assert!(
        screen.contains("Confirm") && screen.contains("delete from pgpeck_confirm"),
        "{screen}"
    );

    h.enter();
    assert!(
        h.app.state.running.is_none(),
        "Enter alone does not confirm"
    );
    h.type_text("yes");
    h.enter();
    h.until("the delete", |a| {
        a.state.running.is_none()
            && a.state
                .doc()
                .results
                .as_ref()
                .is_some_and(|r| r.output.sets.iter().any(|s| s.tag == "DELETE 0"))
    })
    .await;
}
