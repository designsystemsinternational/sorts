use clap::{Parser, Subcommand};
use sorts::activator::{CoreTextActivator, FAKE_ACTIVATOR_ENV, FileFakeActivator, FontActivator};
use sorts::progress::Progress;
use sorts::scan::{self, FAKE_DATALESS_ENV, ScanOptions};
use sorts::settings::ConfigDir;
use sorts::{app, tui};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// A font manager for a shared Fonts folder.
#[derive(Parser)]
#[command(version)]
struct Cli {
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

/// Runs `load` with a progress display of the scan of `root` on stderr.
fn with_progress<T>(root: &Path, load: impl FnOnce(&ScanOptions) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let progress = Progress::new(root, std::io::stderr().is_terminal(), std::io::stderr());
    let fake = std::env::var(FAKE_DATALESS_ENV).ok();
    let is_dataless = |id: &str, metadata: &std::fs::Metadata| match &fake {
        Some(text) => id.contains(text.as_str()),
        None => scan::is_dataless(id, metadata),
    };
    let report = |event| progress.event(event);
    let result = load(&ScanOptions { is_dataless: &is_dataless, progress: &report });
    progress.finish();
    result
}

/// Loads the context with a progress display.
fn load(config: &ConfigDir) -> anyhow::Result<app::Context> {
    let root = config.load_settings()?.root.unwrap_or_default();
    with_progress(&root, |options| app::Context::load_with(config, options))
}

fn fonts(count: usize) -> String {
    format!("{count} {}", if count == 1 { "font" } else { "fonts" })
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let config = ConfigDir::from_env()?;
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
            let library = with_progress(&root, |options| Ok(scan::scan_with(&root, options)?))?;
            println!("{}", app::describe_fonts(&library));
        }
        Some(Command::Sync) => {
            let root = config.load_settings()?.root.unwrap_or_default();
            let report = with_progress(&root, |options| app::sync(&config, options, activator().as_mut()))?;
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
            print!("{}", with_progress(&root, |options| app::status(&config, options))?);
        }
        Some(Command::Fix) => {
            let context = load(&config)?;
            context.ensure_fonts()?;
            if context.missing_entries().is_empty() {
                println!("All entries point to fonts. Nothing to fix.");
                return Ok(());
            }
            interactive(context, config, tui::Tab::Missing)?;
        }
        None => {
            let context = load(&config)?;
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
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
