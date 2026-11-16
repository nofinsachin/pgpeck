mod app;
mod config;
mod diag;
mod events;
mod keymap;
mod logging;
mod paths;
mod theme;
mod ui;

use std::path::Path;
use std::process::ExitCode;

use crate::app::App;
use crate::config::Config;
use crate::diag::Diagnostic;
use crate::keymap::Keymap;
use crate::theme::Theme;

const HELP: &str = "\
oxpecker: a terminal client for PostgreSQL

Usage: oxpecker [OPTIONS]

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
    let dir = paths::config_dir();

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
        problems,
    }
}

fn check_config() -> ExitCode {
    let settings = load_settings();
    match paths::config_dir() {
        Some(dir) => println!("Configuration directory: {}", dir.display()),
        None => println!("Configuration directory: not found (HOME is not set). Using defaults"),
    }
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
        problems,
    } = settings;

    for problem in &problems {
        tracing::warn!(%problem, "configuration problem");
    }
    let mut app = App::new(config, keymap);
    if !problems.is_empty() {
        app.state.notice = Some(format!(
            "{} configuration problem(s). Run oxpecker --check-config",
            problems.len()
        ));
    }

    // ratatui installs a panic hook that restores the terminal.
    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, &mut app, &theme).await;
    ratatui::restore();
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    theme: &Theme,
) -> std::io::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    events::spawn_input(tx);

    let size = terminal.size()?;
    app.update(events::Event::Resize(size.width, size.height));

    while !app.state.quit {
        terminal.draw(|frame| ui::draw(frame, app, theme))?;
        match rx.recv().await {
            Some(event) => app.update(event),
            None => break, // input thread stopped, and it has logged why
        }
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--check-config") => return check_config(),
        Some("-V" | "--version") => {
            println!("oxpecker {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("-h" | "--help") => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("oxpecker: unknown option `{other}`\n\n{HELP}");
            return ExitCode::from(2);
        }
        None => {}
    }

    let _log_guard = logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "oxpecker starting");
    match run(load_settings()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "terminal error");
            eprintln!("oxpecker: {e}");
            ExitCode::FAILURE
        }
    }
}
