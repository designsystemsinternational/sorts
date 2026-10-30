//! The commands. Each command takes its dependencies as parameters.

use crate::activator::FontActivator;
use crate::collections::{self, Collections};
use crate::scan::{self, Library, ScanOptions};
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
        Self::load_with(config, &ScanOptions::default())
    }

    /// Like [`Context::load`], with options for the scan (e.g. a progress display).
    pub fn load_with(config: &ConfigDir, options: &ScanOptions) -> anyhow::Result<Self> {
        let settings = config.load_settings()?;
        let root = settings.root.clone().context("No Fonts folder is set. Run `sorts init` first.")?;
        let library = scan::scan_with(&root, options)
            .with_context(|| format!("Cannot scan the Fonts folder {}", root.display()))?;
        let collections = collections::load(&root);
        Ok(Self { state: config.load_state()?, settings, library, collections })
    }

    /// Fails if the Fonts folder has no font files, because the interactive list would be empty.
    pub fn ensure_fonts(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.library.fonts.is_empty(), "{}", describe_fonts(&self.library));
        Ok(())
    }

    /// Entries in the personal settings and in the collections that match no font, because
    /// the files or folders were moved, renamed, or deleted.
    pub fn missing_entries(&self) -> Vec<MissingEntry> {
        let missing = |owner: Owner, entries: &[String]| -> Vec<MissingEntry> {
            entries
                .iter()
                .filter(|e| !entry::exists(e, &self.library))
                .map(|e| MissingEntry { owner: owner.clone(), entry: e.clone() })
                .collect()
        };
        let mut result = missing(Owner::Settings, &self.settings.enabled);
        for c in &self.collections.items {
            result.extend(missing(Owner::Collection { id: c.id.clone(), name: c.name.clone() }, &c.fonts));
        }
        result
    }

    /// Absolute paths of the fonts that should be registered.
    pub fn desired_paths(&self) -> BTreeSet<PathBuf> {
        active::active_fonts(&self.library, &self.settings, &self.collections)
            .iter()
            .map(|id| self.library.path_of(id))
            .collect()
    }
}

/// Where an entry is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// `enabled` in the personal settings.
    Settings,
    /// The `fonts` of a collection file, shared with the team.
    Collection { id: String, name: String },
}

/// An entry that matches no font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingEntry {
    pub owner: Owner,
    pub entry: String,
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

/// "Font files: N", or a message that the Fonts folder has no font files.
pub fn describe_fonts(library: &Library) -> String {
    match library.fonts.len() {
        0 => format!("No font files found in {}.", library.root.display()),
        count => format!("Font files: {count}"),
    }
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

pub fn sync(config: &ConfigDir, scan: &ScanOptions, activator: &mut dyn FontActivator) -> anyhow::Result<SyncReport> {
    let mut context = Context::load_with(config, scan)?;
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

pub fn status(config: &ConfigDir, scan: &ScanOptions) -> anyhow::Result<Status> {
    let context = Context::load_with(config, scan)?;
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
        if !self.missing_entries.is_empty() || !self.missing_collection_entries.is_empty() {
            writeln!(f, "\nRun `sorts fix` to replace or remove the entries that were not found.")?;
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

        let report = sync(&f.config, &ScanOptions::default(), &mut activator).unwrap();

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
        sync(&f.config, &ScanOptions::default(), &mut activator).unwrap();

        write_font(&f.root, "A/New/Two.otf", "A-Two");
        let report = sync(&f.config, &ScanOptions::default(), &mut activator).unwrap();

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
        sync(&f.config, &ScanOptions::default(), &mut activator).unwrap();
        assert_eq!(activator.registered.len(), 1);

        update_settings(&f.config, |s| s.enabled_collections.clear());
        let report = sync(&f.config, &ScanOptions::default(), &mut activator).unwrap();

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

        let status = status(&f.config, &ScanOptions::default()).unwrap();

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
    fn ensure_fonts_fails_without_font_files() {
        let f = fixture();
        write_file(&f.root, "A/Readme.txt", b"no fonts here");
        init(&f.config, &f.root).unwrap();

        let error = Context::load(&f.config).unwrap().ensure_fonts().unwrap_err().to_string();

        assert_eq!(error, format!("No font files found in {}.", f.root.display()));
    }

    #[test]
    fn ensure_fonts_accepts_a_folder_with_fonts() {
        let f = fixture();
        write_font(&f.root, "A/One.otf", "A-One");
        init(&f.config, &f.root).unwrap();
        assert!(Context::load(&f.config).unwrap().ensure_fonts().is_ok());
    }

    #[test]
    fn describe_fonts_counts_the_files_or_says_there_are_none() {
        let f = fixture();
        assert_eq!(
            describe_fonts(&scan::scan(&f.root).unwrap()),
            format!("No font files found in {}.", f.root.display())
        );
        write_font(&f.root, "A/One.otf", "A-One");
        write_font(&f.root, "A/Two.otf", "A-Two");
        assert_eq!(describe_fonts(&scan::scan(&f.root).unwrap()), "Font files: 2");
    }

    #[test]
    fn load_with_passes_the_scan_options() {
        let f = fixture();
        write_font(&f.root, "A/One.otf", "A-One");
        init(&f.config, &f.root).unwrap();
        let online_only = |_: &str, _: &std::fs::Metadata| true;

        let context = Context::load_with(&f.config, &ScanOptions { is_dataless: &online_only, ..Default::default() });

        assert_eq!(context.unwrap().library.downloaded, 1);
    }

    #[test]
    fn missing_entries_lists_the_settings_first_and_then_each_collection() {
        let f = fixture();
        write_font(&f.root, "A/One.otf", "A-One");
        write_file(&f.root, ".sorts/collections/b.json", br#"{"id": "c2", "name": "B", "fonts": ["Gone.otf"]}"#);
        write_file(&f.root, ".sorts/collections/a.json", br#"{"id": "c1", "name": "A", "fonts": ["A/", "Old/"]}"#);
        init(&f.config, &f.root).unwrap();
        update_settings(&f.config, |s| s.enabled = vec!["Moved/".into(), "A/One.otf".into(), "X.otf".into()]);

        let missing = Context::load(&f.config).unwrap().missing_entries();

        let owner = |id: &str, name: &str| Owner::Collection { id: id.into(), name: name.into() };
        assert_eq!(
            missing,
            [
                MissingEntry { owner: Owner::Settings, entry: "Moved/".into() },
                MissingEntry { owner: Owner::Settings, entry: "X.otf".into() },
                MissingEntry { owner: owner("c1", "A"), entry: "Old/".into() },
                MissingEntry { owner: owner("c2", "B"), entry: "Gone.otf".into() },
            ]
        );
    }

    #[test]
    fn status_display_tells_the_user_to_fix_missing_entries() {
        let status = Status { missing_entries: vec!["Gone/".into()], ..Default::default() };
        assert!(status.to_string().contains("sorts fix"), "{status}");
        let collection =
            Status { missing_collection_entries: vec![("B".into(), "Gone/".into())], ..Default::default() };
        assert!(collection.to_string().contains("sorts fix"), "{collection}");
        assert!(!Status::default().to_string().contains("sorts fix"));
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
