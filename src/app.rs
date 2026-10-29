//! The commands. Each command takes its dependencies as parameters.

use crate::activator::FontActivator;
use crate::collections::{self, Collections};
use crate::scan::{self, Library};
use crate::settings::{ConfigDir, Settings, State};
use crate::sync::{self, SyncReport};
use crate::{active, entry};
use anyhow::Context as _;
use std::collections::BTreeSet;
use std::fmt;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// Everything a command needs to know about the current Fonts folder and settings.
pub struct Context {
    pub settings: Settings,
    pub state: State,
    pub library: Library,
    pub collections: Collections,
}

impl Context {
    /// Loads the settings and scans the Fonts root. Fails if `init` was not run.
    pub fn load(config: &ConfigDir) -> anyhow::Result<Self> {
        let settings = config.load_settings()?;
        let root = settings.root.clone().context("No Fonts folder is set. Run `sorts init` first.")?;
        let library = scan::scan(&root).with_context(|| format!("Cannot scan the Fonts folder {}", root.display()))?;
        let collections = collections::load(&root);
        Ok(Self { state: config.load_state()?, settings, library, collections })
    }

    /// Absolute paths of the fonts that should be registered.
    pub fn desired_paths(&self) -> BTreeSet<PathBuf> {
        active::active_fonts(&self.library, &self.settings, &self.collections)
            .iter()
            .map(|id| self.library.path_of(id))
            .collect()
    }
}

/// Sets the Fonts root folder. Keeps the other settings. Returns the absolute root.
pub fn init(config: &ConfigDir, root: &Path) -> anyhow::Result<PathBuf> {
    let root = root.canonicalize().with_context(|| format!("The folder {} does not exist", root.display()))?;
    anyhow::ensure!(root.is_dir(), "{} is not a folder", root.display());
    let mut settings = config.load_settings()?;
    settings.root = Some(root.clone());
    config.save_settings(&settings)?;
    Ok(root)
}

/// Asks a yes/no question on `output` and reads one line from `input`.
/// Only `y` or `yes` (any case) is yes. An empty line or the end of the input is no.
pub fn confirm(question: &str, input: &mut impl BufRead, output: &mut impl Write) -> std::io::Result<bool> {
    write!(output, "{question} [y/N] ")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

pub fn sync(config: &ConfigDir, activator: &mut dyn FontActivator) -> anyhow::Result<SyncReport> {
    let mut context = Context::load(config)?;
    let report = sync::apply(&context.desired_paths(), &mut context.state, activator);
    config.save_state(&context.state)?;
    Ok(report)
}

#[derive(Debug, Default)]
pub struct Status {
    pub root: PathBuf,
    pub fonts: usize,
    pub unreadable: Vec<(String, String)>,
    pub active: usize,
    pub to_register: usize,
    pub to_unregister: usize,
    pub collections: usize,
    pub enabled_collections: usize,
    /// Entries in `enabled` that match no font.
    pub missing_entries: Vec<String>,
    /// IDs in `enabledCollections` with no collection file.
    pub missing_collections: Vec<String>,
    /// (collection name, entry) for collection entries that match no font.
    pub missing_collection_entries: Vec<(String, String)>,
    pub collection_problems: Vec<String>,
}

pub fn status(config: &ConfigDir) -> anyhow::Result<Status> {
    let context = Context::load(config)?;
    let Context { settings, state, library, collections } = &context;
    let desired = context.desired_paths();
    let (to_register, to_unregister) = sync::plan(&desired, state);
    let missing = |entries: &[String]| -> Vec<String> {
        entries.iter().filter(|e| !entry::exists(e, library)).cloned().collect()
    };

    let enabled_collections: Vec<_> =
        settings.enabled_collections.iter().filter_map(|id| collections.get(id)).collect();
    Ok(Status {
        root: library.root.clone(),
        fonts: library.fonts.len(),
        unreadable: library.fonts.iter().filter_map(|f| f.error.as_ref().map(|e| (f.id.clone(), e.clone()))).collect(),
        active: desired.len(),
        to_register: to_register.len(),
        to_unregister: to_unregister.len(),
        collections: collections.items.len(),
        enabled_collections: enabled_collections.len(),
        missing_entries: missing(&settings.enabled),
        missing_collections: settings
            .enabled_collections
            .iter()
            .filter(|id| collections.get(id).is_none())
            .cloned()
            .collect(),
        missing_collection_entries: collections
            .items
            .iter()
            .flat_map(|c| missing(&c.fonts).into_iter().map(|e| (c.name.clone(), e)))
            .collect(),
        collection_problems: collections.problems.clone(),
    })
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Fonts folder: {}", self.root.display())?;
        writeln!(f, "Font files:   {} ({} unreadable)", self.fonts, self.unreadable.len())?;
        writeln!(f, "Active fonts: {}", self.active)?;
        writeln!(f, "Collections:  {} ({} enabled)", self.collections, self.enabled_collections)?;
        if self.to_register + self.to_unregister > 0 {
            writeln!(
                f,
                "\nNot in sync: {} to register, {} to unregister. Run `sorts sync`.",
                self.to_register, self.to_unregister
            )?;
        }

        let mut problems: Vec<String> = Vec::new();
        problems.extend(self.unreadable.iter().map(|(id, e)| format!("Unreadable font file: {id} ({e})")));
        problems.extend(self.missing_entries.iter().map(|e| format!("Enabled entry not found: {e}")));
        problems.extend(self.missing_collections.iter().map(|id| format!("Enabled collection not found: {id}")));
        problems.extend(
            self.missing_collection_entries
                .iter()
                .map(|(name, e)| format!("Collection \"{name}\": entry not found: {e}")),
        );
        problems.extend(self.collection_problems.iter().cloned());
        if !problems.is_empty() {
            writeln!(f, "\nProblems:")?;
            for problem in problems {
                writeln!(f, "  {problem}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activator::FakeActivator;
    use crate::testing::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        config: ConfigDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Fonts");
        std::fs::create_dir(&root).unwrap();
        let config = ConfigDir::new(dir.path().join("config"));
        Fixture { root: root.canonicalize().unwrap(), config, _dir: dir }
    }

    fn update_settings(config: &ConfigDir, change: impl FnOnce(&mut Settings)) {
        let mut settings = config.load_settings().unwrap();
        change(&mut settings);
        config.save_settings(&settings).unwrap();
    }

    #[test]
    fn init_saves_the_absolute_root_and_keeps_other_settings() {
        let f = fixture();
        update_settings(&f.config, |s| s.enabled = vec!["A/".into()]);
        let relative = pathdiff(&f.root);

        let root = init(&f.config, &relative).unwrap();

        assert_eq!(root, f.root);
        let settings = f.config.load_settings().unwrap();
        assert_eq!(settings.root, Some(f.root.clone()));
        assert_eq!(settings.enabled, ["A/"]);
    }

    /// A path to `target` that is relative to the current directory.
    fn pathdiff(target: &Path) -> PathBuf {
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        let ups = cwd.components().count() - 1;
        let mut path: PathBuf = std::iter::repeat_n("..", ups).collect();
        path.push(target.strip_prefix("/").unwrap());
        path
    }

    fn ask(answer: &str) -> (bool, String) {
        let mut output = Vec::new();
        let yes = confirm("Use /fonts?", &mut answer.as_bytes(), &mut output).unwrap();
        (yes, String::from_utf8(output).unwrap())
    }

    #[test]
    fn confirm_shows_the_question_with_the_choices() {
        assert_eq!(ask("y\n").1, "Use /fonts? [y/N] ");
    }

    #[test]
    fn confirm_accepts_only_yes() {
        for answer in ["y\n", "Y\n", "yes\n", "  YES  \n", "y"] {
            assert!(ask(answer).0, "{answer:?}");
        }
        for answer in ["n\n", "no\n", "\n", "", "yep\n", "sure\n"] {
            assert!(!ask(answer).0, "{answer:?}");
        }
    }

    #[test]
    fn init_fails_if_the_folder_does_not_exist() {
        let f = fixture();
        assert!(init(&f.config, &f.root.join("nope")).is_err());
    }

    #[test]
    fn commands_fail_with_a_hint_before_init() {
        let f = fixture();
        let error = Context::load(&f.config).err().unwrap().to_string();
        assert!(error.contains("sorts init"), "{error}");
    }

    #[test]
    fn sync_registers_the_active_fonts_and_saves_the_state() {
        let f = fixture();
        write_font(&f.root, "A/One.otf", "A-One");
        write_font(&f.root, "A/One.ttf", "A-One");
        write_font(&f.root, "B/Two.otf", "B-Two");
        init(&f.config, &f.root).unwrap();
        update_settings(&f.config, |s| s.enabled = vec!["A/".into()]);
        let mut activator = FakeActivator::default();

        let report = sync(&f.config, &mut activator).unwrap();

        assert_eq!(report.registered, [f.root.join("A/One.otf")]);
        assert_eq!(activator.registered, BTreeSet::from([f.root.join("A/One.otf")]));
        assert_eq!(f.config.load_state().unwrap().registered, activator.registered);
    }

    #[test]
    fn sync_picks_up_new_fonts_in_an_enabled_folder() {
        let f = fixture();
        write_font(&f.root, "A/One.otf", "A-One");
        init(&f.config, &f.root).unwrap();
        update_settings(&f.config, |s| s.enabled = vec!["A/".into()]);
        let mut activator = FakeActivator::default();
        sync(&f.config, &mut activator).unwrap();

        write_font(&f.root, "A/New/Two.otf", "A-Two");
        let report = sync(&f.config, &mut activator).unwrap();

        assert_eq!(report.registered, [f.root.join("A/New/Two.otf")]);
        assert!(report.unregistered.is_empty());
    }

    #[test]
    fn sync_unregisters_fonts_from_a_disabled_collection() {
        let f = fixture();
        write_font(&f.root, "A/One.otf", "A-One");
        write_file(&f.root, ".sorts/collections/brand.json", br#"{"id": "c1", "name": "Brand", "fonts": ["A/"]}"#);
        init(&f.config, &f.root).unwrap();
        update_settings(&f.config, |s| s.enabled_collections = vec!["c1".into()]);
        let mut activator = FakeActivator::default();
        sync(&f.config, &mut activator).unwrap();
        assert_eq!(activator.registered.len(), 1);

        update_settings(&f.config, |s| s.enabled_collections.clear());
        let report = sync(&f.config, &mut activator).unwrap();

        assert_eq!(report.unregistered, [f.root.join("A/One.otf")]);
        assert!(activator.registered.is_empty());
    }

    #[test]
    fn status_reports_counts_and_problems() {
        let f = fixture();
        write_font(&f.root, "A/One.otf", "A-One");
        write_font(&f.root, "A/Two.otf", "A-Two");
        write_file(&f.root, "A/Broken.ttf", b"not a font");
        write_file(
            &f.root,
            ".sorts/collections/brand.json",
            br#"{"id": "c1", "name": "Brand", "fonts": ["A/One.otf", "Gone/"]}"#,
        );
        write_file(&f.root, ".sorts/collections/broken.json", b"{ nope");
        init(&f.config, &f.root).unwrap();
        update_settings(&f.config, |s| {
            s.enabled = vec!["A/Two.otf".into(), "Missing.otf".into()];
            s.enabled_collections = vec!["c1".into(), "deleted".into()];
        });

        let status = status(&f.config).unwrap();

        assert_eq!(status.root, f.root);
        assert_eq!(status.fonts, 3);
        assert_eq!(status.unreadable.len(), 1);
        assert_eq!(status.unreadable[0].0, "A/Broken.ttf");
        assert_eq!(status.active, 2);
        assert_eq!(status.to_register, 2);
        assert_eq!(status.to_unregister, 0);
        assert_eq!(status.collections, 1);
        assert_eq!(status.enabled_collections, 1);
        assert_eq!(status.missing_entries, ["Missing.otf"]);
        assert_eq!(status.missing_collections, ["deleted"]);
        assert_eq!(status.missing_collection_entries, [("Brand".to_string(), "Gone/".to_string())]);
        assert_eq!(status.collection_problems.len(), 1);
    }

    #[test]
    fn status_display_tells_the_user_to_sync_when_needed() {
        let status = Status { root: "/fonts".into(), fonts: 3, active: 2, to_register: 1, ..Default::default() };
        let text = status.to_string();
        assert!(text.contains("/fonts"), "{text}");
        assert!(text.contains("sorts sync"), "{text}");

        let synced = Status { to_register: 0, ..status };
        assert!(!synced.to_string().contains("sorts sync"));
    }
}
