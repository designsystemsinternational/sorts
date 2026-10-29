use clap::{Parser, Subcommand};
use sorts::activator::{CoreTextActivator, FAKE_ACTIVATOR_ENV, FileFakeActivator, FontActivator};
use sorts::settings::ConfigDir;
use sorts::{app, tui};
use std::io::IsTerminal;
use std::path::PathBuf;
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
}

fn activator() -> Box<dyn FontActivator> {
    match std::env::var_os(FAKE_ACTIVATOR_ENV) {
        Some(path) => Box::new(FileFakeActivator { path: path.into() }),
        None => Box::new(CoreTextActivator::persistent()),
    }
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
        }
        Some(Command::Sync) => {
            let report = app::sync(&config, activator().as_mut())?;
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
        Some(Command::Status) => print!("{}", app::status(&config)?),
        None => {
            anyhow::ensure!(
                std::io::stdout().is_terminal(),
                "The interactive list needs a terminal. Use `sorts --help` for the other commands."
            );
            let context = app::Context::load(&config)?;
            tui::terminal::run(&mut tui::App::new(context, config, activator()))?;
        }
    }
    Ok(())
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
