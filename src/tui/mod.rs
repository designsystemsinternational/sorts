//! The interactive list. `App` holds all state and handles keys; `view` draws it;
//! `terminal` runs the event loop.

pub mod terminal;
pub mod view;

use crate::activator::FontActivator;
use crate::app::Context;
use crate::collections::{self, Collection};
use crate::entry;
use crate::settings::{ConfigDir, Settings};
use crate::sync;
use crate::tree::{Kind, ROOT, Row, Tree};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Fonts,
    Collections,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Continue,
    Quit,
}

/// The checkbox of a row. "Enabled" means enabled in the personal settings, or in the
/// collection that is being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// All fonts are enabled.
    On,
    /// Some fonts are enabled.
    Partial,
    /// No fonts are enabled, but some are active through a collection. Not used while editing.
    Collection,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    NewCollection,
    Rename { id: String },
    ConfirmDelete { id: String },
}

/// A question in the message line. While it is open, it gets all keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub prompt: Prompt,
    pub text: String,
}

pub struct App {
    pub context: Context,
    config: ConfigDir,
    activator: Box<dyn FontActivator>,
    pub tree: Tree,
    pub tab: Tab,
    pub expanded: HashSet<usize>,
    pub rows: Vec<Row>,
    pub search: String,
    /// True while the user types a search query.
    pub searching: bool,
    pub message: Option<String>,
    pub input: Option<Input>,
    /// The ID of the collection that the font tree edits. `None`: the tree edits the personal settings.
    pub editing: Option<String>,
    pub font_list: ListState,
    pub collection_list: ListState,
    /// Per font index: enabled in the personal settings (or in the edited collection).
    explicit: Vec<bool>,
    /// Per font index: included in an enabled collection.
    from_collection: Vec<bool>,
}

impl App {
    pub fn new(context: Context, config: ConfigDir, activator: Box<dyn FontActivator>) -> Self {
        let tree = Tree::build(&context.library);
        let mut app = Self {
            context,
            config,
            activator,
            tree,
            tab: Tab::Fonts,
            expanded: HashSet::new(),
            rows: vec![],
            search: String::new(),
            searching: false,
            message: None,
            input: None,
            editing: None,
            font_list: ListState::default(),
            collection_list: ListState::default(),
            explicit: vec![],
            from_collection: vec![],
        };
        app.refresh_marks();
        app.refresh_rows();
        if !app.context.collections.items.is_empty() {
            app.collection_list.select(Some(0));
        }
        app
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        if self.input.is_some() {
            self.handle_input_key(key.code);
            return Action::Continue;
        }
        if self.searching {
            self.handle_search_key(key.code);
            return Action::Continue;
        }
        match (self.tab, key.code) {
            (_, KeyCode::Esc) if !self.search.is_empty() => self.set_search(String::new()),
            (_, KeyCode::Esc) if self.editing.is_some() => self.stop_editing(),
            (_, KeyCode::Esc) => return Action::Quit,
            (_, KeyCode::Char('/')) => {
                self.tab = Tab::Fonts;
                self.searching = true;
            }
            (_, KeyCode::Tab | KeyCode::BackTab) if self.editing.is_some() => self.stop_editing(),
            (Tab::Fonts, KeyCode::Tab | KeyCode::BackTab) => self.tab = Tab::Collections,
            (Tab::Collections, KeyCode::Tab | KeyCode::BackTab) => self.tab = Tab::Fonts,
            (_, KeyCode::Down) | (Tab::Collections, KeyCode::Char('j')) => self.move_selection(1),
            (_, KeyCode::Up) | (Tab::Collections, KeyCode::Char('k')) => self.move_selection(-1),
            (Tab::Fonts, KeyCode::Right) => self.expand(),
            (Tab::Fonts, KeyCode::Left) => self.collapse(),
            (Tab::Fonts, KeyCode::Char(' ') | KeyCode::Enter) => self.toggle_font_row(),
            (Tab::Fonts, KeyCode::Char(c)) if c.is_alphanumeric() => self.jump_to(c),
            (Tab::Collections, KeyCode::Char(' ')) => self.toggle_collection_row(),
            (Tab::Collections, KeyCode::Enter | KeyCode::Char('e')) => self.start_editing(),
            (Tab::Collections, KeyCode::Char('n')) => {
                self.input = Some(Input { prompt: Prompt::NewCollection, text: String::new() })
            }
            (Tab::Collections, KeyCode::Char('r')) => {
                if let Some(c) = self.selected_collection() {
                    self.input = Some(Input { prompt: Prompt::Rename { id: c.id.clone() }, text: c.name.clone() })
                }
            }
            (Tab::Collections, KeyCode::Char('d')) => {
                if let Some(c) = self.selected_collection() {
                    self.input = Some(Input { prompt: Prompt::ConfirmDelete { id: c.id.clone() }, text: String::new() })
                }
            }
            _ => {}
        }
        Action::Continue
    }

    fn handle_input_key(&mut self, code: KeyCode) {
        let Some(input) = self.input.as_mut() else { return };
        if let Prompt::ConfirmDelete { id } = &input.prompt {
            let id = id.clone();
            self.input = None;
            if code == KeyCode::Char('y') {
                self.delete_collection(&id);
            }
            return;
        }
        match code {
            KeyCode::Char(c) => input.text.push(c),
            KeyCode::Backspace => {
                input.text.pop();
            }
            KeyCode::Esc => self.input = None,
            KeyCode::Enter => {
                let input = self.input.take().expect("input is open");
                match input.prompt {
                    Prompt::NewCollection => self.create_collection(&input.text),
                    Prompt::Rename { id } => self.rename_collection(&id, &input.text),
                    Prompt::ConfirmDelete { .. } => unreachable!("handled above"),
                }
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => self.searching = false,
            KeyCode::Esc => {
                self.searching = false;
                self.set_search(String::new());
            }
            KeyCode::Backspace => {
                let mut search = self.search.clone();
                search.pop();
                self.set_search(search);
            }
            KeyCode::Char(c) => self.set_search(format!("{}{c}", self.search)),
            _ => {}
        }
    }

    fn set_search(&mut self, search: String) {
        self.search = search;
        self.refresh_rows();
        self.font_list.select((!self.rows.is_empty()).then_some(0));
    }

    /// The node of the selected row in the Fonts tab.
    pub fn selected_node(&self) -> Option<usize> {
        self.font_list.selected().and_then(|i| self.rows.get(i)).map(|r| r.node)
    }

    pub fn mark(&self, node: usize) -> Mark {
        let fonts = self.tree.nodes[node].fonts.clone();
        let (enabled, total) = self.counts(node);
        if total > 0 && enabled == total {
            Mark::On
        } else if enabled > 0 {
            Mark::Partial
        } else if self.from_collection[fonts].iter().any(|on| *on) {
            Mark::Collection
        } else {
            Mark::Off
        }
    }

    /// (fonts enabled in the personal settings, all fonts) in a node.
    pub fn counts(&self, node: usize) -> (usize, usize) {
        let fonts = self.tree.nodes[node].fonts.clone();
        (self.explicit[fonts.clone()].iter().filter(|on| **on).count(), fonts.len())
    }

    fn move_selection(&mut self, delta: isize) {
        let (list, len) = match self.tab {
            Tab::Fonts => (&mut self.font_list, self.rows.len()),
            Tab::Collections => (&mut self.collection_list, self.context.collections.items.len()),
        };
        if len == 0 {
            return;
        }
        let current = list.selected().unwrap_or(0) as isize;
        list.select(Some((current + delta).clamp(0, len as isize - 1) as usize));
    }

    /// Selects the next visible row after the selected one (wrapping) whose name starts with `c`.
    fn jump_to(&mut self, c: char) {
        let len = self.rows.len();
        let start = self.font_list.selected().map_or(0, |i| i + 1);
        let prefix = c.to_lowercase().to_string();
        let found = (0..len)
            .map(|offset| (start + offset) % len)
            .find(|&i| self.tree.nodes[self.rows[i].node].name.to_lowercase().starts_with(&prefix));
        if found.is_some() {
            self.font_list.select(found);
        }
    }

    fn expand(&mut self) {
        if let Some(node) = self.selected_node()
            && self.tree.nodes[node].kind == Kind::Folder
        {
            self.expanded.insert(node);
            self.refresh_rows();
        }
    }

    /// Collapses an expanded folder, or moves to the parent folder.
    fn collapse(&mut self) {
        let Some(node) = self.selected_node() else { return };
        if self.expanded.remove(&node) {
            self.refresh_rows();
        } else if let Some(parent) = self.tree.nodes[node].parent.filter(|p| *p != ROOT)
            && let Some(index) = self.rows.iter().position(|r| r.node == parent)
        {
            self.font_list.select(Some(index));
        }
    }

    /// Rebuilds the rows and keeps the selected node selected when it is still visible.
    fn refresh_rows(&mut self) {
        let selected = self.selected_node();
        self.rows = self.tree.rows(&self.context.library, &self.expanded, &self.search);
        let index = selected.and_then(|node| self.rows.iter().position(|r| r.node == node));
        self.font_list.select(index.or((!self.rows.is_empty()).then_some(0)));
    }

    fn refresh_marks(&mut self) {
        let settings = &self.context.settings;
        let fonts = &self.context.library.fonts;
        if let Some(collection) = self.editing.as_deref().and_then(|id| self.context.collections.get(id)) {
            self.explicit = fonts.iter().map(|f| entry::matches_any(&collection.fonts, &f.id)).collect();
            self.from_collection = vec![false; fonts.len()];
            return;
        }
        let collection_entries: Vec<&String> = settings
            .enabled_collections
            .iter()
            .filter_map(|id| self.context.collections.get(id))
            .flat_map(|c| &c.fonts)
            .collect();
        self.explicit = fonts.iter().map(|f| entry::matches_any(&settings.enabled, &f.id)).collect();
        self.from_collection = fonts.iter().map(|f| entry::matches_any(&collection_entries, &f.id)).collect();
    }

    pub fn selected_collection(&self) -> Option<&Collection> {
        self.collection_list.selected().and_then(|i| self.context.collections.items.get(i))
    }

    pub fn edited_collection(&self) -> Option<&Collection> {
        self.editing.as_deref().and_then(|id| self.context.collections.get(id))
    }

    fn toggle_font_row(&mut self) {
        let Some(node) = self.selected_node() else { return };
        let on = self.mark(node) != Mark::On;
        let path = self.tree.nodes[node].path.clone();
        if let Some(collection) = self.edited_collection() {
            let mut collection = collection.clone();
            collection.fonts = self.tree.set_enabled(&self.context.library, &collection.fonts, node, on);
            let label = format!(
                "{} {path} {} {}",
                if on { "Added" } else { "Removed" },
                if on { "to" } else { "from" },
                collection.name
            );
            let root = self.context.library.root.clone();
            self.change(&label, |app| {
                collections::save(&root, &collection)?;
                app.reload_collections(Some(&collection.id));
                Ok(())
            });
        } else {
            let entries = self.tree.set_enabled(&self.context.library, &self.context.settings.enabled, node, on);
            let label = format!("{} {path}", if on { "Enabled" } else { "Disabled" });
            self.update_settings(&label, |s| s.enabled = entries);
        }
    }

    fn start_editing(&mut self) {
        let Some(collection) = self.selected_collection() else { return };
        let (id, name) = (collection.id.clone(), collection.name.clone());
        self.message = Some(format!("Editing {name}. Space adds or removes fonts. Esc when done."));
        self.editing = Some(id);
        self.tab = Tab::Fonts;
        self.refresh_marks();
    }

    fn stop_editing(&mut self) {
        self.editing = None;
        self.tab = Tab::Collections;
        self.message = None;
        self.refresh_marks();
    }

    /// Reloads the collection files and selects the collection with this ID.
    fn reload_collections(&mut self, select: Option<&str>) {
        let previous = self.collection_list.selected();
        self.context.collections = collections::load(&self.context.library.root);
        let items = &self.context.collections.items;
        let index = select.and_then(|id| items.iter().position(|c| c.id == id));
        let fallback = previous.map(|i| i.min(items.len().saturating_sub(1))).filter(|_| !items.is_empty());
        self.collection_list.select(index.or(fallback));
    }

    fn create_collection(&mut self, name: &str) {
        match collections::create(&self.context.library.root, name) {
            Ok(collection) => {
                self.reload_collections(Some(&collection.id));
                self.message = Some(format!("Created collection {}. Press e to add fonts.", collection.name));
            }
            Err(error) => self.message = Some(format!("{error:#}")),
        }
    }

    fn rename_collection(&mut self, id: &str, name: &str) {
        let Some(collection) = self.context.collections.get(id).cloned() else { return };
        match collections::rename(&self.context.library.root, &collection, name) {
            Ok(renamed) => {
                self.reload_collections(Some(id));
                self.message = Some(format!("Renamed collection {} to {}.", collection.name, renamed.name));
            }
            Err(error) => self.message = Some(format!("{error:#}")),
        }
    }

    fn delete_collection(&mut self, id: &str) {
        let Some(collection) = self.context.collections.get(id).cloned() else { return };
        let root = self.context.library.root.clone();
        let id = id.to_string();
        self.change(&format!("Deleted collection {}", collection.name), |app| {
            collections::delete(&root, &collection)?;
            app.reload_collections(None);
            if app.context.settings.enabled_collections.contains(&id) {
                app.context.settings.enabled_collections.retain(|c| *c != id);
                app.config.save_settings(&app.context.settings)?;
            }
            Ok(())
        });
    }

    fn toggle_collection_row(&mut self) {
        let Some(collection) = self.selected_collection() else { return };
        let (id, name) = (collection.id.clone(), collection.name.clone());
        let on = !self.context.settings.enabled_collections.contains(&id);
        let label = format!("{} collection {name}", if on { "Enabled" } else { "Disabled" });
        self.update_settings(&label, |s| {
            if on {
                s.enabled_collections.push(id);
            } else {
                s.enabled_collections.retain(|c| *c != id);
            }
        });
    }

    /// Changes and saves the settings, then registers or unregisters only the fonts that
    /// the change affects.
    fn update_settings(&mut self, label: &str, change: impl FnOnce(&mut Settings)) {
        self.change(label, |app| {
            change(&mut app.context.settings);
            app.config.save_settings(&app.context.settings)
        });
    }

    /// Runs a change to the settings or the collections, then registers or unregisters only
    /// the fonts that the change affects. If the change fails, the settings and collections
    /// are read again from disk.
    fn change(&mut self, label: &str, change: impl FnOnce(&mut Self) -> anyhow::Result<()>) {
        let before = self.context.desired_paths();
        if let Err(error) = change(self) {
            if let Ok(settings) = self.config.load_settings() {
                self.context.settings = settings;
            }
            self.reload_collections(self.editing.clone().as_deref());
            self.refresh_marks();
            self.message = Some(format!("{error:#}"));
            return;
        }
        let after = self.context.desired_paths();
        let report = sync::apply_changes(&before, &after, &mut self.context.state, self.activator.as_mut());
        let saved = self.config.save_state(&self.context.state);
        self.refresh_marks();

        let mut message =
            format!("{label}. Registered {}, unregistered {}.", report.registered.len(), report.unregistered.len());
        if let Some((path, error)) = report.errors.first() {
            message += &format!(" {} failed (first: {}: {error}).", report.errors.len(), path.display());
        }
        if let Err(error) = saved {
            message += &format!(" Could not save the state: {error:#}.");
        }
        self.message = Some(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activator::FakeActivator;
    use crate::app;
    use crate::testing::{write_file, write_font};

    pub(crate) struct Fixture {
        _dir: tempfile::TempDir,
        pub config: ConfigDir,
    }

    pub(crate) fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Fonts");
        write_font(&root, "Foundry A/Sans/Sans-Bold.otf", "Sans-Bold");
        write_font(&root, "Foundry A/Sans/Sans-Regular.otf", "Sans-Regular");
        write_font(&root, "Foundry A/Serif/Serif-Regular.otf", "Serif-Regular");
        write_font(&root, "Foundry B/Antiqua.otf", "Antiqua-Regular");
        write_file(&root, "Foundry B/Broken.ttf", b"not a font");
        write_file(
            &root,
            ".sorts/collections/brand.json",
            br#"{"id": "c1", "name": "Brand", "fonts": ["Foundry B/Antiqua.otf"]}"#,
        );
        write_file(&root, ".sorts/collections/editorial.json", br#"{"id": "c2", "name": "Editorial", "fonts": []}"#);
        let config = ConfigDir::new(dir.path().join("config"));
        app::init(&config, &root).unwrap();
        Fixture { _dir: dir, config }
    }

    pub(crate) fn open(f: &Fixture) -> App {
        App::new(Context::load(&f.config).unwrap(), f.config.clone(), Box::new(FakeActivator::default()))
    }

    pub(crate) fn press(app: &mut App, code: KeyCode) -> Action {
        app.handle_key(KeyEvent::from(code))
    }

    pub(crate) fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn selected_path(app: &App) -> String {
        app.tree.nodes[app.selected_node().unwrap()].path.clone()
    }

    fn select(app: &mut App, path: &str) {
        let node = app.tree.find(path).unwrap();
        let index = app.rows.iter().position(|r| r.node == node).expect("row is visible");
        app.font_list.select(Some(index));
    }

    fn saved_settings(f: &Fixture) -> Settings {
        f.config.load_settings().unwrap()
    }

    fn registered(app: &App) -> Vec<String> {
        app.context
            .state
            .registered
            .iter()
            .map(|p| p.strip_prefix(&app.context.library.root).unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn starts_with_collapsed_top_level_folders_and_the_first_row_selected() {
        let f = fixture();
        let app = open(&f);
        assert_eq!(app.rows.len(), 2);
        assert_eq!(selected_path(&app), "Foundry A/");
        assert_eq!(app.tab, Tab::Fonts);
    }

    #[test]
    fn arrow_keys_move_expand_and_collapse() {
        let f = fixture();
        let mut app = open(&f);

        press(&mut app, KeyCode::Right);
        assert_eq!(app.rows.len(), 4);
        press(&mut app, KeyCode::Down);
        assert_eq!(selected_path(&app), "Foundry A/Sans/");
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down); // stops at the last row
        assert_eq!(selected_path(&app), "Foundry B/");
        press(&mut app, KeyCode::Up);
        assert_eq!(selected_path(&app), "Foundry A/Serif/");

        // Left on a row inside a folder moves to the folder, a second Left collapses it.
        press(&mut app, KeyCode::Left);
        assert_eq!(selected_path(&app), "Foundry A/");
        press(&mut app, KeyCode::Left);
        assert_eq!(app.rows.len(), 2);
        press(&mut app, KeyCode::Up); // stops at the first row
        assert_eq!(selected_path(&app), "Foundry A/");
    }

    #[test]
    fn space_on_a_folder_enables_it_saves_the_settings_and_registers_the_fonts() {
        let f = fixture();
        let mut app = open(&f);

        press(&mut app, KeyCode::Char(' '));

        assert_eq!(saved_settings(&f).enabled, ["Foundry A/"]);
        assert_eq!(
            registered(&app),
            ["Foundry A/Sans/Sans-Bold.otf", "Foundry A/Sans/Sans-Regular.otf", "Foundry A/Serif/Serif-Regular.otf",]
        );
        assert_eq!(f.config.load_state().unwrap(), app.context.state);
        assert_eq!(app.mark(app.tree.find("Foundry A/").unwrap()), Mark::On);
        assert!(app.message.as_deref().unwrap().contains("3"), "{:?}", app.message);
    }

    #[test]
    fn space_on_a_file_in_an_enabled_folder_disables_only_that_file() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        select(&mut app, "Foundry A/Sans/Sans-Bold.otf");

        press(&mut app, KeyCode::Char(' '));

        assert_eq!(saved_settings(&f).enabled, ["Foundry A/Sans/Sans-Regular.otf", "Foundry A/Serif/"]);
        assert_eq!(app.mark(app.tree.find("Foundry A/").unwrap()), Mark::Partial);
        assert_eq!(app.counts(app.tree.find("Foundry A/").unwrap()), (2, 3));
        assert!(!registered(&app).contains(&"Foundry A/Sans/Sans-Bold.otf".to_string()));
    }

    #[test]
    fn space_on_a_partial_folder_enables_all_of_it() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Right);
        select(&mut app, "Foundry A/Serif/");
        press(&mut app, KeyCode::Char(' '));
        select(&mut app, "Foundry A/");
        assert_eq!(app.mark(app.selected_node().unwrap()), Mark::Partial);

        press(&mut app, KeyCode::Char(' '));

        assert_eq!(saved_settings(&f).enabled, ["Foundry A/"]);
    }

    #[test]
    fn fonts_in_an_enabled_collection_show_the_collection_mark() {
        let f = fixture();
        let mut settings = f.config.load_settings().unwrap();
        settings.enabled_collections = vec!["c1".into()];
        f.config.save_settings(&settings).unwrap();
        let app = open(&f);

        assert_eq!(app.mark(app.tree.find("Foundry B/Antiqua.otf").unwrap()), Mark::Collection);
        assert_eq!(app.mark(app.tree.find("Foundry B/").unwrap()), Mark::Collection);
        assert_eq!(app.mark(app.tree.find("Foundry A/").unwrap()), Mark::Off);
    }

    #[test]
    fn slash_starts_a_search_that_filters_the_rows() {
        let f = fixture();
        let mut app = open(&f);

        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "ant");

        assert!(app.searching);
        assert_eq!(app.search, "ant");
        let names: Vec<_> = app.rows.iter().map(|r| app.tree.nodes[r.node].name.as_str()).collect();
        assert_eq!(names, ["Foundry B", "Antiqua.otf"]);
        assert_eq!(selected_path(&app), "Foundry B/");

        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.search, "an");
    }

    #[test]
    fn enter_ends_typing_and_keeps_the_filter_and_esc_clears_it() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "ant");

        press(&mut app, KeyCode::Enter);
        assert!(!app.searching);
        assert_eq!(app.rows.len(), 2);
        // Keys work as commands again.
        press(&mut app, KeyCode::Down);
        assert_eq!(selected_path(&app), "Foundry B/Antiqua.otf");

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.search, "");
        assert_eq!(app.rows.len(), 2);
        assert_eq!(press(&mut app, KeyCode::Esc), Action::Quit);
    }

    #[test]
    fn q_does_not_quit() {
        let f = fixture();
        let mut app = open(&f);
        assert_eq!(press(&mut app, KeyCode::Char('q')), Action::Continue);
        press(&mut app, KeyCode::Tab);
        assert_eq!(press(&mut app, KeyCode::Char('q')), Action::Continue);
    }

    #[test]
    fn a_letter_jumps_to_the_next_visible_row_that_starts_with_it() {
        let f = fixture();
        let mut app = open(&f);

        press(&mut app, KeyCode::Char('f')); // the next match after the selected row
        assert_eq!(selected_path(&app), "Foundry B/");
        press(&mut app, KeyCode::Char('F')); // case-insensitive, wraps to the top
        assert_eq!(selected_path(&app), "Foundry A/");

        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(selected_path(&app), "Foundry A/Sans/");
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(selected_path(&app), "Foundry A/Serif/");
        press(&mut app, KeyCode::Char('s')); // wraps to the first match
        assert_eq!(selected_path(&app), "Foundry A/Sans/");

        press(&mut app, KeyCode::Char('x')); // no match: the selection stays
        assert_eq!(selected_path(&app), "Foundry A/Sans/");
    }

    #[test]
    fn ctrl_c_always_quits() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Char('/'));
        let action = app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(action, Action::Quit);
    }

    #[test]
    fn tab_switches_to_collections_where_space_toggles_a_collection() {
        let f = fixture();
        let mut app = open(&f);

        press(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Collections);
        press(&mut app, KeyCode::Char(' '));

        assert_eq!(saved_settings(&f).enabled_collections, ["c1"]);
        assert_eq!(registered(&app), ["Foundry B/Antiqua.otf"]);
        assert_eq!(app.mark(app.tree.find("Foundry B/Antiqua.otf").unwrap()), Mark::Collection);

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down); // stops at the last collection
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(saved_settings(&f).enabled_collections, ["c1", "c2"]);

        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(saved_settings(&f).enabled_collections, ["c2"]);
        assert!(registered(&app).is_empty());

        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.tab, Tab::Fonts);
    }

    #[test]
    fn slash_in_the_collections_tab_goes_to_the_fonts_search() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('/'));
        assert_eq!(app.tab, Tab::Fonts);
        assert!(app.searching);
    }

    #[test]
    fn a_toggle_does_not_register_fonts_that_wait_for_sync() {
        let f = fixture();
        let mut settings = f.config.load_settings().unwrap();
        settings.enabled = vec!["Foundry A/Serif/".into()];
        f.config.save_settings(&settings).unwrap();
        // The Serif font is enabled but not registered: the user has not run `sync`.
        let mut app = open(&f);

        select(&mut app, "Foundry B/");
        press(&mut app, KeyCode::Char(' '));

        assert_eq!(registered(&app), ["Foundry B/Antiqua.otf"]);
    }

    #[test]
    fn root_is_not_a_row() {
        let f = fixture();
        let app = open(&f);
        assert!(app.rows.iter().all(|r| r.node != ROOT));
        assert!(app.rows.iter().all(|r| app.tree.nodes[r.node].kind == Kind::Folder));
        assert!(entry::is_folder(&app.tree.nodes[app.rows[0].node].path));
    }

    fn collection_names(app: &App) -> Vec<&str> {
        app.context.collections.items.iter().map(|c| c.name.as_str()).collect()
    }

    fn collection_files(f: &Fixture) -> Vec<String> {
        let root = f.config.load_settings().unwrap().root.unwrap();
        crate::collections::load(&root).items.into_iter().map(|c| c.file).collect()
    }

    fn selected_collection(app: &App) -> &str {
        &app.context.collections.items[app.collection_list.selected().unwrap()].name
    }

    fn enable_collection(f: &Fixture, id: &str) {
        let mut settings = f.config.load_settings().unwrap();
        settings.enabled_collections.push(id.into());
        f.config.save_settings(&settings).unwrap();
    }

    #[test]
    fn n_creates_a_collection_and_selects_it() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);

        press(&mut app, KeyCode::Char('n'));
        assert!(matches!(app.input, Some(Input { prompt: Prompt::NewCollection, .. })));
        type_text(&mut app, "Studio Work");
        press(&mut app, KeyCode::Enter);

        assert!(app.input.is_none());
        assert_eq!(collection_files(&f), ["brand.json", "editorial.json", "studio-work.json"]);
        assert_eq!(collection_names(&app), ["Brand", "Editorial", "Studio Work"]);
        assert_eq!(selected_collection(&app), "Studio Work");
        assert!(app.message.as_deref().unwrap().contains("Studio Work"));
    }

    #[test]
    fn esc_cancels_a_prompt() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "Nope");

        assert_eq!(press(&mut app, KeyCode::Esc), Action::Continue);

        assert!(app.input.is_none());
        assert_eq!(collection_files(&f), ["brand.json", "editorial.json"]);
    }

    #[test]
    fn keys_in_a_prompt_are_text() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(press(&mut app, KeyCode::Char('q')), Action::Continue);
        type_text(&mut app, "x/d");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.input.as_ref().unwrap().text, "qx/");
    }

    #[test]
    fn a_failed_create_shows_the_error() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "BRAND");
        press(&mut app, KeyCode::Enter);

        assert!(app.message.as_deref().unwrap().contains("already exists"), "{:?}", app.message);
        assert_eq!(collection_files(&f), ["brand.json", "editorial.json"]);
    }

    #[test]
    fn r_renames_the_selected_collection_and_keeps_it_enabled() {
        let f = fixture();
        enable_collection(&f, "c1");
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);

        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.input.as_ref().unwrap().text, "Brand");
        for _ in 0..5 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "Studio");
        press(&mut app, KeyCode::Enter);

        assert_eq!(collection_files(&f), ["editorial.json", "studio.json"]);
        assert_eq!(selected_collection(&app), "Studio");
        assert_eq!(app.context.collections.get("c1").unwrap().name, "Studio");
        assert_eq!(f.config.load_settings().unwrap().enabled_collections, ["c1"]);
        assert_eq!(registered(&app), Vec::<String>::new(), "a rename changes no fonts");
    }

    #[test]
    fn d_then_y_deletes_and_disables_the_collection() {
        let f = fixture();
        enable_collection(&f, "c1");
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char(' ')); // disable
        press(&mut app, KeyCode::Char(' ')); // enable: registers Antiqua
        assert_eq!(registered(&app), ["Foundry B/Antiqua.otf"]);

        press(&mut app, KeyCode::Char('d'));
        assert!(matches!(app.input, Some(Input { prompt: Prompt::ConfirmDelete { .. }, .. })));
        press(&mut app, KeyCode::Char('y'));

        assert_eq!(collection_files(&f), ["editorial.json"]);
        assert!(f.config.load_settings().unwrap().enabled_collections.is_empty());
        assert!(registered(&app).is_empty());
        assert_eq!(selected_collection(&app), "Editorial");
    }

    #[test]
    fn d_then_another_key_keeps_the_collection() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('n'));

        assert!(app.input.is_none());
        assert_eq!(collection_files(&f), ["brand.json", "editorial.json"]);
    }

    #[test]
    fn e_edits_the_collection_in_the_font_tree() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Down);

        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.tab, Tab::Fonts);
        assert_eq!(app.editing.as_deref(), Some("c2"));
        // Marks show the collection, not the personal settings.
        assert_eq!(app.mark(app.tree.find("Foundry B/").unwrap()), Mark::Off);

        select(&mut app, "Foundry A/");
        press(&mut app, KeyCode::Char(' '));

        assert_eq!(app.context.collections.get("c2").unwrap().fonts, ["Foundry A/"]);
        let root = f.config.load_settings().unwrap().root.unwrap();
        assert_eq!(crate::collections::load(&root).get("c2").unwrap().fonts, ["Foundry A/"]);
        assert!(f.config.load_settings().unwrap().enabled.is_empty());
        assert!(registered(&app).is_empty(), "the collection is not enabled");
        assert_eq!(app.mark(app.tree.find("Foundry A/").unwrap()), Mark::On);

        assert_eq!(press(&mut app, KeyCode::Esc), Action::Continue);
        assert_eq!(app.editing, None);
        assert_eq!(app.tab, Tab::Collections);
        assert_eq!(app.mark(app.tree.find("Foundry A/").unwrap()), Mark::Off);
    }

    #[test]
    fn editing_an_enabled_collection_registers_the_change() {
        let f = fixture();
        enable_collection(&f, "c2");
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);

        press(&mut app, KeyCode::Char(' '));

        assert_eq!(registered(&app).len(), 3);
    }

    #[test]
    fn esc_in_edit_mode_clears_the_search_first() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "ant");
        press(&mut app, KeyCode::Enter);

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.search, "");
        assert_eq!(app.editing.as_deref(), Some("c1"));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.editing, None);
        assert_eq!(app.tab, Tab::Collections);
    }
}
