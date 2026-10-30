use clap::{Parser, Subcommand};
use sorts::activator::{CoreTextActivator, FAKE_ACTIVATOR_ENV, FileFakeActivator, FontActivator};
use sorts::debug::{DebugActivator, DebugLog, WAIT_AFTER};
use sorts::progress::{Progress, TICK_INTERVAL};
use sorts::scan::{self, FAKE_DATALESS_ENV, ScanEvent, ScanOptions};
use sorts::settings::ConfigDir;
use sorts::{app, tui};
use std::io::{IsTerminal, Stderr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Instant;

/// A font manager for a shared Fonts folder.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Log each step of the scan and the sync to stderr, and list the steps that wait.
    #[arg(long, global = true)]
    debug: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Set the Fonts folder. Without a path, asks to use the current folder.
    Init { path: Option<PathBuf> },
    /// Register and unregister fonts so they match your settings.
    Sync,
    /// Show the Fonts folder, counts, and problems.
    Status,
    /// Replace or remove entries that point to fonts that do not exist.
    Fix,
}

fn activator() -> Box<dyn FontActivator> {
    match std::env::var_os(FAKE_ACTIVATOR_ENV) {
        Some(path) => Box::new(FileFakeActivator { path: path.into() }),
        None => Box::new(CoreTextActivator::persistent()),
    }
}

/// The debug log on stderr, if `--debug` is set.
type Debug = Option<DebugLog<Stderr>>;

/// The width of the terminal on stderr in columns, or 0 if it is not known.
fn terminal_width() -> usize {
    ratatui::crossterm::terminal::size().map(|(columns, _)| columns as usize).unwrap_or(0)
}

/// Runs `load` with a progress display of the scan of `root` on stderr.
///
/// A timer thread redraws the progress line, so the time moves also when the scan waits. With `--debug`, the progress
/// line is off, each scan event goes to the debug log, and the timer thread lists the steps that wait.
fn with_progress<T>(
    root: &Path,
    debug: &Debug,
    load: impl FnOnce(&ScanOptions) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let terminal = debug.is_none() && std::io::stderr().is_terminal();
    let progress = Progress::new(terminal, terminal_width(), std::io::stderr());
    let fake = std::env::var(FAKE_DATALESS_ENV).ok();
    let is_dataless = |id: &str, metadata: &std::fs::Metadata| match &fake {
        Some(text) => id.contains(text.as_str()),
        None => scan::is_dataless(id, metadata),
    };
    let report = |event: ScanEvent| {
        if let Some(log) = debug {
            log.event(&event);
        }
        progress.event(event);
    };
    let (stop, stopped) = mpsc::channel::<()>();
    let result = std::thread::scope(|scope| {
        let progress = &progress;
        scope.spawn(move || {
            let mut last_waits = Instant::now();
            while let Err(mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(TICK_INTERVAL) {
                progress.tick();
                if let Some(log) = debug
                    && last_waits.elapsed() >= WAIT_AFTER
                {
                    log.waits();
                    last_waits = Instant::now();
                }
            }
        });
        if let Some(log) = debug {
            log.line(&format!("Fonts folder: {}", root.display()));
            log.line(&format!("online-only check: {}", if fake.is_some() { "fake (tests)" } else { "st_flags" }));
            log.root(root);
        }
        let result = load(&ScanOptions { is_dataless: &is_dataless, progress: &report });
        drop(stop);
        result
    });
    progress.finish();
    result
}

/// Loads the context with a progress display.
fn load(config: &ConfigDir, debug: &Debug) -> anyhow::Result<app::Context> {
    let root = config.load_settings()?.root.unwrap_or_default();
    with_progress(&root, debug, |options| app::Context::load_with(config, options))
}

/// The macOS version, for the debug log.
fn macos_version() -> String {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("unknown ({e})"))
}

fn fonts(count: usize) -> String {
    format!("{count} {}", if count == 1 { "font" } else { "fonts" })
}

fn run(cli: Cli, debug: &Debug) -> anyhow::Result<()> {
    let config = ConfigDir::from_env()?;
    if let Some(log) = debug {
        log.line(&format!("sorts {}", env!("CARGO_PKG_VERSION")));
        log.line(&format!("macOS {}", macos_version()));
        log.line(&format!("settings folder: {}", config.path.display()));
        log.line(&format!("command: {}", std::env::args().skip(1).collect::<Vec<_>>().join(" ")));
    }
    match cli.command {
        Some(Command::Init { path }) => {
            let path = match path {
                Some(path) => path,
                None => {
                    let current = std::env::current_dir()?;
                    let question = format!("Use {} as the Fonts folder?", current.display());
                    let yes = app::confirm(&question, &mut std::io::stdin().lock(), &mut std::io::stdout())?;
                    if !yes {
                        println!();
                    }
                    anyhow::ensure!(yes, "Cancelled. Nothing changed.");
                    current
                }
            };
            let root = app::init(&config, &path)?;
            println!("Fonts folder: {}", root.display());
            let library = with_progress(&root, debug, |options| Ok(scan::scan_with(&root, options)?))?;
            println!("{}", app::describe_fonts(&library));
        }
        Some(Command::Sync) => {
            let root = config.load_settings()?.root.unwrap_or_default();
            let mut activator = activator();
            let report = with_progress(&root, debug, |options| match debug {
                Some(log) => app::sync(&config, options, &mut DebugActivator { activator: activator.as_mut(), log }),
                None => app::sync(&config, options, activator.as_mut()),
            })?;
            println!(
                "Registered {}. Unregistered {}.",
                fonts(report.registered.len()),
                fonts(report.unregistered.len())
            );
            for (path, error) in &report.errors {
                eprintln!("Error: {}: {error}", path.display());
            }
            anyhow::ensure!(report.errors.is_empty(), "{} fonts failed", report.errors.len());
        }
        Some(Command::Status) => {
            let root = config.load_settings()?.root.unwrap_or_default();
            print!("{}", with_progress(&root, debug, |options| app::status(&config, options))?);
        }
        Some(Command::Fix) => {
            let context = load(&config, debug)?;
            context.ensure_fonts()?;
            if context.missing_entries().is_empty() {
                println!("All entries point to fonts. Nothing to fix.");
                return Ok(());
            }
            interactive(context, config, tui::Tab::Missing)?;
        }
        None => {
            let context = load(&config, debug)?;
            context.ensure_fonts()?;
            interactive(context, config, tui::Tab::Fonts)?;
        }
    }
    Ok(())
}

/// Opens the interactive list on `tab`.
fn interactive(context: app::Context, config: ConfigDir, tab: tui::Tab) -> anyhow::Result<()> {
    anyhow::ensure!(
        std::io::stdout().is_terminal(),
        "The interactive list needs a terminal. Use `sorts --help` for the other commands."
    );
    let mut app = tui::App::new(context, config, activator());
    app.tab = tab;
    tui::terminal::run(&mut app)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let debug: Debug = cli.debug.then(|| DebugLog::new(std::io::stderr()));
    let result = run(cli, &debug);
    if let Some(log) = &debug {
        log.line(&format!("done in {:.3}s", log.start().elapsed().as_secs_f64()));
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
