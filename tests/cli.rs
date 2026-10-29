//! End-to-end tests of the `sorts` binary with a temporary Fonts folder,
//! a temporary settings folder, and the file-based fake activator.

use assert_cmd::Command;
use predicates::prelude::*;
use sorts::activator::FileFakeActivator;
use sorts::settings::ConfigDir;
use sorts::testing::{write_file, write_font};
use std::path::PathBuf;
use tempfile::TempDir;

struct Env {
    _dir: TempDir,
    root: PathBuf,
    config: PathBuf,
    fake: PathBuf,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Fonts");
    std::fs::create_dir(&root).unwrap();
    Env {
        root: root.canonicalize().unwrap(),
        config: dir.path().join("config"),
        fake: dir.path().join("fake-activator.json"),
        _dir: dir,
    }
}

impl Env {
    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("sorts").unwrap();
        cmd.args(args).env("SORTS_CONFIG_DIR", &self.config).env("SORTS_FAKE_ACTIVATOR", &self.fake);
        cmd
    }

    fn enable(&self, entries: &[&str]) {
        let config = ConfigDir::new(&self.config);
        let mut settings = config.load_settings().unwrap();
        settings.enabled = entries.iter().map(|e| e.to_string()).collect();
        config.save_settings(&settings).unwrap();
    }

    fn registered(&self) -> Vec<PathBuf> {
        FileFakeActivator { path: self.fake.clone() }.registered().into_iter().collect()
    }
}

fn saved_root(env: &Env) -> Option<PathBuf> {
    ConfigDir::new(&env.config).load_settings().unwrap().root
}

#[test]
fn init_without_a_folder_asks_to_use_the_current_folder() {
    let env = env();

    env.cmd(&["init"])
        .current_dir(&env.root)
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("Use {} as the Fonts folder? [y/N]", env.root.display())))
        .stdout(predicate::str::contains(format!("Fonts folder: {}", env.root.display())));

    assert_eq!(saved_root(&env), Some(env.root.clone()));
}

#[test]
fn init_without_a_folder_changes_nothing_if_the_user_says_no() {
    let env = env();

    env.cmd(&["init"])
        .current_dir(&env.root)
        .write_stdin("n\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Cancelled"));

    assert_eq!(saved_root(&env), None);
}

#[test]
fn init_without_a_folder_and_without_an_answer_changes_nothing() {
    let env = env();
    env.cmd(&["init"]).current_dir(&env.root).write_stdin("").assert().failure();
    assert_eq!(saved_root(&env), None);
}

#[test]
fn init_with_a_folder_does_not_ask() {
    let env = env();
    env.cmd(&["init", env.root.to_str().unwrap()])
        .write_stdin("")
        .assert()
        .success()
        .stdout(predicate::str::contains("[y/N]").not());
    assert_eq!(saved_root(&env), Some(env.root.clone()));
}

#[test]
fn the_interactive_list_needs_a_terminal() {
    let env = env();
    env.cmd(&["init", env.root.to_str().unwrap()]).assert().success();
    env.cmd(&[])
        .assert()
        .failure()
        .stderr(predicate::str::contains("needs a terminal"))
        .stderr(predicate::str::contains("sorts --help"));
}

#[test]
fn sync_before_init_fails_with_a_hint() {
    let env = env();
    env.cmd(&["sync"]).assert().failure().stderr(predicate::str::contains("sorts init"));
}

#[test]
fn init_then_sync_registers_the_enabled_fonts() {
    let env = env();
    write_font(&env.root, "Foundry/Family/A-Regular.otf", "A-Regular");
    write_font(&env.root, "Foundry/Family/A-Regular.ttf", "A-Regular");
    write_font(&env.root, "Other/B-Regular.otf", "B-Regular");

    env.cmd(&["init", env.root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(env.root.to_str().unwrap()));
    env.enable(&["Foundry/"]);
    env.cmd(&["sync"]).assert().success().stdout(predicate::str::contains("Registered 1"));

    assert_eq!(env.registered(), [env.root.join("Foundry/Family/A-Regular.otf")]);
}

#[test]
fn sync_unregisters_fonts_that_are_no_longer_enabled() {
    let env = env();
    write_font(&env.root, "Foundry/A-Regular.otf", "A-Regular");
    env.cmd(&["init", env.root.to_str().unwrap()]).assert().success();
    env.enable(&["Foundry/"]);
    env.cmd(&["sync"]).assert().success();

    env.enable(&[]);
    env.cmd(&["sync"]).assert().success().stdout(predicate::str::contains("Unregistered 1"));

    assert!(env.registered().is_empty());
}

#[test]
fn status_shows_the_root_counts_and_problems() {
    let env = env();
    write_font(&env.root, "Foundry/A-Regular.otf", "A-Regular");
    write_file(&env.root, "Foundry/Broken.ttf", b"not a font");
    env.cmd(&["init", env.root.to_str().unwrap()]).assert().success();
    env.enable(&["Foundry/A-Regular.otf", "Gone/"]);

    env.cmd(&["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains(env.root.to_str().unwrap()))
        .stdout(predicate::str::contains("Foundry/Broken.ttf"))
        .stdout(predicate::str::contains("Gone/"))
        .stdout(predicate::str::contains("sorts sync"));
}
