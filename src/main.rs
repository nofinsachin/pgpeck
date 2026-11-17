use std::path::Path;
use std::process::ExitCode;

use pgpeck::app::App;
use pgpeck::config::Config;
use pgpeck::diag::Diagnostic;
use pgpeck::keymap::Keymap;
use pgpeck::profile::Profiles;
use pgpeck::runtime::Runtime;
use pgpeck::theme::Theme;
use pgpeck::{events, logging, terminal, ui};

const HELP: &str = "\
pgpeck: a terminal client for PostgreSQL

Usage: pgpeck [OPTIONS]

Options:
      --check-config  Validate configuration, keymap and theme, then exit
  -V, --version       Print the version
  -h, --help          Print this help
";

/// Everything read from the user's configuration directory.
struct Settings {
    config: Config,
    keymap: Keymap,
    theme: Theme,
    profiles: Profiles,
    problems: Vec<Diagnostic>,
}

/// Reads a file that may legitimately be absent.
fn read_optional(path: &Path, problems: &mut Vec<Diagnostic>) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            problems.push(Diagnostic::new(
                &path.display().to_string(),
                None,
                e.to_string(),
            ));
            None
        }
    }
}

fn load_settings() -> Settings {
    let mut problems = Vec::new();
    let dir = pgpeck::paths::config_dir();

    let config = match dir
        .as_ref()
        .and_then(|d| read_optional(&d.join("config.toml"), &mut problems))
    {
        Some(src) => {
            let (config, diags) = Config::parse(&src);
            problems.extend(diags);
            config
        }
        None => Config::default(),
    };

    let keymap = match dir
        .as_ref()
        .and_then(|d| read_optional(&d.join("keymap.toml"), &mut problems))
    {
        Some(src) => {
            let (keymap, diags) = Keymap::with_user(&src, "keymap.toml");
            problems.extend(diags);
            keymap
        }
        None => Keymap::default_keymap(),
    };

    let profiles = match dir.as_ref().map(|d| d.join(pgpeck::profile::FILE)) {
        Some(path) => match read_optional(&path, &mut problems) {
            Some(src) => {
                let (profiles, diags) = Profiles::parse(&src);
                problems.extend(diags);
                problems.extend(pgpeck::profile::permission_problem(&path));
                profiles
            }
            None => Profiles::default(),
        },
        None => Profiles::default(),
    };

    let theme_file = format!("{}.toml", config.ui.theme);
    let user_theme = dir
        .as_ref()
        .and_then(|d| read_optional(&d.join("themes").join(&theme_file), &mut problems));
    let theme = match (user_theme, Theme::builtin(&config.ui.theme)) {
        (Some(src), base) => {
            let (theme, diags) = Theme::parse(&src, &theme_file, &base.unwrap_or_default());
            problems.extend(diags);
            theme
        }
        (None, Some(builtin)) => builtin,
        (None, None) => {
            problems.push(Diagnostic::new(
                "config.toml",
                None,
                format!("unknown theme `{}`. Using `dark`", config.ui.theme),
            ));
            Theme::default()
        }
    };

    Settings {
        config,
        keymap,
        theme,
        profiles,
        problems,
    }
}

fn check_config() -> ExitCode {
    let settings = load_settings();
    match pgpeck::paths::config_dir() {
        Some(dir) => println!("Configuration directory: {}", dir.display()),
        None => println!("Configuration directory: not found (HOME is not set). Using defaults"),
    }
    println!("Connection profiles: {}", settings.profiles.len());
    if settings.problems.is_empty() {
        println!("Configuration is valid.");
        ExitCode::SUCCESS
    } else {
        for problem in &settings.problems {
            println!("{problem}");
        }
        println!("{} problem(s) found.", settings.problems.len());
        ExitCode::FAILURE
    }
}

async fn run(settings: Settings) -> std::io::Result<()> {
    let Settings {
        config,
        keymap,
        theme,
        profiles,
        problems,
    } = settings;

    for problem in &problems {
        tracing::warn!(%problem, "configuration problem");
    }
    let profiles_path = pgpeck::paths::config_dir().map(|d| d.join(pgpeck::profile::FILE));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let runtime = Runtime::new(
        tx.clone(),
        config.connections.password_sources.clone(),
        config.results.row_limit,
        profiles_path,
    );
    let mut app = App::new(config, keymap);
    app.state.profiles = profiles;
    if !problems.is_empty() {
        app.state.notice = Some(format!(
            "{} configuration problem(s). Run pgpeck --check-config",
            problems.len()
        ));
    }

    let mut terminal = terminal::enter()?;
    events::spawn_input(tx.clone());
    let result = event_loop(&mut terminal, &mut app, &theme, rx, &runtime, tx).await;
    terminal::leave();
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    theme: &Theme,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<events::Event>,
    runtime: &Runtime,
    tx: tokio::sync::mpsc::UnboundedSender<events::Event>,
) -> std::io::Result<()> {
    let size = terminal.size()?;
    for command in app.update(events::Event::Resize(size.width, size.height)) {
        runtime.execute(command);
    }

    // While a statement runs, redraw a few times a second so its elapsed
    // time counts up.
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Once a second, so the session numbers refresh on their own schedule.
    let mut slow = tokio::time::interval(std::time::Duration::from_secs(1));
    slow.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Whatever is copied also goes to the system clipboard, through the terminal.
    let mut copied = String::new();
    while !app.state.quit {
        terminal.draw(|frame| ui::draw(frame, app, theme))?;
        if app.state.register != copied {
            copied.clone_from(&app.state.register);
            if let Some(sequence) = pgpeck::clipboard::osc52(&copied) {
                use std::io::Write;
                let mut out = std::io::stdout();
                out.write_all(sequence.as_bytes())?;
                out.flush()?;
            }
        }
        let event = tokio::select! {
            event = rx.recv() => match event {
                Some(event) => event,
                None => break, // input thread stopped, and it has logged why
            },
            _ = ticker.tick(), if app.state.running.is_some() => events::Event::Tick,
            _ = slow.tick() => events::Event::Tick,
        };
        for command in app.update(event) {
            runtime.execute(command);
        }
    }
    // Close connections cleanly. Nothing else is waiting on them.
    drop(tx);
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--check-config") => return check_config(),
        Some("-V" | "--version") => {
            println!("pgpeck {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("-h" | "--help") => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("pgpeck: unknown option `{other}`\n\n{HELP}");
            return ExitCode::from(2);
        }
        None => {}
    }

    let _log_guard = logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "pgpeck starting");
    match run(load_settings()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "terminal error");
            eprintln!("pgpeck: {e}");
            ExitCode::FAILURE
        }
    }
}
