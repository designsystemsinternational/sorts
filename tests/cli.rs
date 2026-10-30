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
    write_font(&env.root, "A/One.otf", "A-One");
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

#[test]
fn fix_says_so_when_nothing_is_missing() {
    let env = env();
    write_font(&env.root, "A/One.otf", "A-One");
    env.cmd(&["init", env.root.to_str().unwrap()]).assert().success();
    env.enable(&["A/"]);

    env.cmd(&["fix"]).assert().success().stdout("All entries point to fonts. Nothing to fix.\n");
}

#[test]
fn status_and_fix_show_missing_entries() {
    let env = env();
    write_font(&env.root, "A/One.otf", "A-One");
    env.cmd(&["init", env.root.to_str().unwrap()]).assert().success();
    env.enable(&["Moved/"]);

    env.cmd(&["status"]).assert().success().stdout(predicate::str::contains("Run `sorts fix`"));
    // The tests have no terminal, so `fix` cannot open the interactive list.
    env.cmd(&["fix"]).assert().failure().stderr(predicate::str::contains("needs a terminal"));
}

#[test]
fn init_counts_the_font_files() {
    let env = env();
    write_font(&env.root, "A/One.otf", "A-One");
    env.cmd(&["init", env.root.to_str().unwrap()]).assert().success().stdout(predicate::str::contains("Font files: 1"));
}

#[test]
fn init_says_so_when_the_folder_has_no_font_files() {
    let env = env();
    env.cmd(&["init", env.root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("No font files found in {}.", env.root.display())));
    assert_eq!(saved_root(&env), Some(env.root.clone()));
}

#[test]
fn the_interactive_list_and_fix_say_so_when_there_are_no_font_files() {
    let env = env();
    env.cmd(&["init", env.root.to_str().unwrap()]).assert().success();
    let message = format!("No font files found in {}.", env.root.display());
    env.cmd(&[]).assert().failure().stderr(predicate::str::contains(&message));
    env.cmd(&["fix"]).assert().failure().stderr(predicate::str::contains(&message));
}

#[test]
fn commands_show_the_downloads_of_online_only_files() {
    let env = env();
    write_font(&env.root, "A/Local.otf", "Local-Regular");
    write_font(&env.root, "B/Cloud.otf", "Cloud-Regular");
    env.cmd(&["init", env.root.to_str().unwrap()])
        .env("SORTS_FAKE_DATALESS", "Cloud")
        .assert()
        .success()
        .stderr(predicate::str::contains("Downloading 1 file ("))
        .stderr(predicate::str::contains("Downloaded 1 file ("))
        .stderr(predicate::str::contains("available offline"));
    for args in [&["sync"][..], &["status"]] {
        env.cmd(args)
            .env("SORTS_FAKE_DATALESS", "Cloud")
            .assert()
            .success()
            .stderr(predicate::str::contains("Downloading 1 file"));
    }
    env.cmd(&["sync"]).assert().success().stderr(predicate::str::contains("Downloading").not());
}

#[test]
fn debug_logs_each_step_of_the_scan() {
    let env = env();
    write_font(&env.root, "A/Local.otf", "Local-Regular");
    write_font(&env.root, "B/Cloud.otf", "Cloud-Regular");
    let output = env
        .cmd(&["init", env.root.to_str().unwrap(), "--debug"])
        .env("SORTS_FAKE_DATALESS", "Cloud")
        .assert()
        .success()
        .stdout(predicate::str::contains("Font files: 2"))
        .get_output()
        .stderr
        .clone();
    let log = String::from_utf8(output).unwrap();
    for text in [
        &format!("sorts {}", env!("CARGO_PKG_VERSION")),
        "macOS ",
        "] [main] command: init",
        "end stat of the Fonts folder",
        "folder (root) flags=",
        "folder A/ flags=",
        "found #2 B/Cloud.otf size=",
        "online-only",
        "walk done. 2 font files to read",
        "start read A/Local.otf",
        "end read B/Cloud.otf",
        "downloaded",
        "Downloaded 1 file (",
        "done",
    ] {
        assert!(log.contains(text), "{text:?} is not in the log:\n{log}");
    }
    assert!(!log.contains("\x1b[2K"), "the progress line is off in debug mode:\n{log}");
}

#[test]
fn debug_logs_each_registration_of_sync() {
    let env = env();
    write_font(&env.root, "A/Local.otf", "Local-Regular");
    env.cmd(&["init", env.root.to_str().unwrap()]).assert().success();
    env.enable(&["A/"]);
    env.cmd(&["--debug", "sync"])
        .assert()
        .success()
        .stderr(predicate::str::contains(format!("start register {}", env.root.join("A/Local.otf").display())))
        .stderr(predicate::str::contains("end register").and(predicate::str::contains(" ok")));
    env.cmd(&["sync"]).assert().success().stderr(predicate::str::contains("register").not());
}
