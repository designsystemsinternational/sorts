//! The interactive list. `App` holds all state and handles keys; `view` draws it;
//! `terminal` runs the event loop.

pub mod terminal;
pub mod view;

use crate::activator::FontActivator;
use crate::app::{Context, MissingEntry, Owner};
use crate::collections::{self, Collection};
use crate::entry;
use crate::settings::{ConfigDir, Settings};
use crate::sync;
use crate::tree::{Kind, ROOT, Row, Tree};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::ListState;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Keys typed within this time of the previous key add to the jump text, as in macOS lists.
const JUMP_TIMEOUT: Duration = Duration::from_millis(500);

/// Rows per mouse wheel event.
const SCROLL_LINES: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Fonts,
    Collections,
    Settings,
    /// Entries that match no font. Shown only if some were missing.
    Missing,
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
    Rename {
        id: String,
    },
    ConfirmDelete {
        id: String,
    },
    /// Replace the entry in `App::replacing` with `target`.
    ConfirmReplace {
        target: String,
    },
    ConfirmRemove {
        missing: MissingEntry,
    },
}

/// The list of collections that the selected folder or font can be added to or removed from.
/// While it is open, it gets all keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    /// The font tree node that the picker adds or removes.
    pub node: usize,
    pub list: ListState,
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
    pub picker: Option<Picker>,
    /// The ID of the collection that the font tree edits. `None`: the tree edits the personal settings.
    pub editing: Option<String>,
    /// The missing entry that the font tree picks a new location for.
    pub replacing: Option<MissingEntry>,
    /// The entries that match no font, from the personal settings and the collections.
    pub missing: Vec<MissingEntry>,
    /// True if the Missing tab is shown: some entries were missing when the app opened, or later.
    show_missing: bool,
    pub font_list: ListState,
    pub collection_list: ListState,
    /// The selected row in `Settings::full_format_order`.
    pub settings_list: ListState,
    pub missing_list: ListState,
    /// True while the selected format is picked up: the arrow keys move it.
    pub moving: bool,
    /// The area of the list in the current tab, from the last draw. Mouse events outside it
    /// are ignored.
    pub list_area: Rect,
    /// The area of the typeface column in the Collections tab, from the last draw. The mouse
    /// wheel scrolls it.
    pub typeface_area: Rect,
    /// The first typeface row in view, and the ID of the collection it belongs to. The view sets
    /// the offset back to 0 when another collection is selected.
    pub typeface_offset: usize,
    pub typeface_collection: Option<String>,
    /// The areas of the tab titles in the header, from the last draw.
    pub tab_areas: Vec<(Tab, Rect)>,
    /// True after a mouse scroll, until the next key. Then the view can show rows without the
    /// selected one.
    pub scrolled: bool,
    /// The text typed to jump to a row, and the time of its last key.
    jump: Option<(String, Instant)>,
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
            picker: None,
            editing: None,
            replacing: None,
            missing: vec![],
            show_missing: false,
            font_list: ListState::default(),
            collection_list: ListState::default(),
            settings_list: ListState::default().with_selected(Some(0)),
            missing_list: ListState::default(),
            moving: false,
            list_area: Rect::default(),
            typeface_area: Rect::default(),
            typeface_offset: 0,
            typeface_collection: None,
            tab_areas: vec![],
            scrolled: false,
            jump: None,
            explicit: vec![],
            from_collection: vec![],
        };
        app.refresh_marks();
        app.refresh_rows();
        app.refresh_missing();
        if !app.context.collections.items.is_empty() {
            app.collection_list.select(Some(0));
        }
        app.message = match app.missing.len() {
            0 => None,
            1 => Some("1 entry points to fonts that do not exist. See the Missing tab.".into()),
            n => Some(format!("{n} entries point to fonts that do not exist. See the Missing tab.")),
        };
        app
    }

    /// The tabs in the header, in `tab` order.
    pub fn tabs(&self) -> Vec<Tab> {
        let mut tabs = vec![Tab::Fonts, Tab::Collections, Tab::Settings];
        if self.show_missing {
            tabs.push(Tab::Missing);
        }
        tabs
    }

    /// The tab `delta` places after the current one, wrapping.
    fn other_tab(&self, delta: isize) -> Tab {
        let tabs = self.tabs();
        let index = tabs.iter().position(|t| *t == self.tab).unwrap_or(0) as isize;
        tabs[(index + delta).rem_euclid(tabs.len() as isize) as usize]
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        self.handle_key_at(key, Instant::now())
    }

    /// Handles a key pressed at `now`. Tests pass the time to check the jump timeout.
    pub fn handle_key_at(&mut self, key: KeyEvent, now: Instant) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        self.scrolled = false;
        // Any key other than a jump key ends the jump text.
        let jump = self.jump.take().filter(|(_, time)| now.saturating_duration_since(*time) < JUMP_TIMEOUT);
        if self.input.is_some() {
            self.handle_input_key(key.code);
            return Action::Continue;
        }
        if self.picker.is_some() {
            self.handle_picker_key(key.code);
            return Action::Continue;
        }
        if self.searching {
            self.handle_search_key(key.code);
            return Action::Continue;
        }
        if self.moving {
            match key.code {
                KeyCode::Down => return self.move_format(1),
                KeyCode::Up => return self.move_format(-1),
                KeyCode::Enter | KeyCode::Esc => {
                    self.moving = false;
                    return Action::Continue;
                }
                // Any other key puts the format down and does its usual action.
                _ => self.moving = false,
            }
        }
        match (self.tab, key.code) {
            (_, KeyCode::Esc) if !self.search.is_empty() => self.set_search(String::new()),
            (_, KeyCode::Esc) if self.editing.is_some() => self.stop_editing(),
            (_, KeyCode::Esc) if self.replacing.is_some() => self.stop_replacing(),
            (_, KeyCode::Esc) => return Action::Quit,
            (_, KeyCode::Char('/')) => {
                self.tab = Tab::Fonts;
                self.searching = true;
            }
            (_, KeyCode::Tab | KeyCode::BackTab) if self.editing.is_some() => self.stop_editing(),
            (_, KeyCode::Tab | KeyCode::BackTab) if self.replacing.is_some() => self.stop_replacing(),
            (_, KeyCode::Tab) => self.tab = self.other_tab(1),
            (_, KeyCode::BackTab) => self.tab = self.other_tab(-1),
            (_, KeyCode::Down) => self.move_selection(1),
            (_, KeyCode::Up) => self.move_selection(-1),
            (Tab::Fonts, KeyCode::Right) => self.expand(),
            (Tab::Fonts, KeyCode::Left) => self.collapse(),
            (Tab::Fonts, KeyCode::Enter) if self.replacing.is_some() => self.ask_replace(),
            (Tab::Fonts, KeyCode::Enter) => self.toggle_font_row(),
            (Tab::Fonts, KeyCode::Char('+'))
                if jump.is_none() && self.editing.is_none() && self.replacing.is_none() =>
            {
                self.open_picker()
            }
            (Tab::Fonts, KeyCode::Char(c)) if c.is_alphanumeric() || jump.is_some() => self.jump_to(c, jump, now),
            (Tab::Collections, KeyCode::Enter) => self.toggle_collection_row(),
            (Tab::Collections, KeyCode::Char('e')) => self.start_editing(),
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
            (Tab::Settings, KeyCode::Enter) => self.moving = true,
            (Tab::Missing, KeyCode::Enter) => self.start_replacing(),
            (Tab::Missing, KeyCode::Char('d')) => {
                if let Some(missing) = self.selected_missing() {
                    let prompt = Prompt::ConfirmRemove { missing: missing.clone() };
                    self.input = Some(Input { prompt, text: String::new() })
                }
            }
            _ => {}
        }
        Action::Continue
    }

    /// The mouse wheel scrolls the list under the pointer without a change to the selection. In the
    /// Collections tab, it also scrolls the typeface column.
    /// A left click selects the row under the pointer, or switches to the tab whose title it hits.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        self.jump = None;
        let position = Position::new(mouse.column, mouse.row);
        if self.input.is_some() || self.picker.is_some() {
            return;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some((tab, _)) = self.tab_areas.iter().find(|(_, area)| area.contains(position))
        {
            self.click_tab(*tab);
            return;
        }
        if self.typeface_area.contains(position) {
            // The view stops the offset when the last row is at the bottom.
            match mouse.kind {
                MouseEventKind::ScrollDown => self.typeface_offset += SCROLL_LINES,
                MouseEventKind::ScrollUp => self.typeface_offset = self.typeface_offset.saturating_sub(SCROLL_LINES),
                _ => {}
            }
            return;
        }
        let area = self.list_area;
        if !area.contains(position) {
            return;
        }
        let (list, len) = self.current_list();
        let max_offset = len.saturating_sub(area.height as usize);
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                *list.offset_mut() = (list.offset() + SCROLL_LINES).min(max_offset);
                self.scrolled = true;
            }
            MouseEventKind::ScrollUp => {
                *list.offset_mut() = list.offset().saturating_sub(SCROLL_LINES);
                self.scrolled = true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let index = list.offset() + (mouse.row - area.y) as usize;
                if index < len {
                    list.select(Some(index));
                    self.moving = false;
                }
            }
            _ => {}
        }
    }

    /// Switches to another tab. Ends edit mode, replace mode, search typing (the filter stays),
    /// and moving.
    fn click_tab(&mut self, tab: Tab) {
        if tab == self.tab {
            return;
        }
        if self.editing.is_some() {
            self.stop_editing();
        }
        if self.replacing.is_some() {
            self.stop_replacing();
        }
        self.searching = false;
        self.moving = false;
        self.tab = tab;
    }

    fn handle_input_key(&mut self, code: KeyCode) {
        let Some(input) = self.input.as_mut() else { return };
        if !matches!(input.prompt, Prompt::NewCollection | Prompt::Rename { .. }) {
            // A y/n question: only y confirms.
            let prompt = self.input.take().expect("input is open").prompt;
            if code == KeyCode::Char('y') {
                match prompt {
                    Prompt::ConfirmDelete { id } => self.delete_collection(&id),
                    Prompt::ConfirmReplace { target } => self.replace_missing(&target),
                    Prompt::ConfirmRemove { missing } => self.remove_missing(&missing),
                    Prompt::NewCollection | Prompt::Rename { .. } => unreachable!("handled below"),
                }
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
                    _ => unreachable!("handled above"),
                }
            }
            _ => {}
        }
    }

    fn handle_picker_key(&mut self, code: KeyCode) {
        let Some(picker) = self.picker.as_mut() else { return };
        let len = self.context.collections.items.len();
        let current = picker.list.selected().unwrap_or(0);
        match code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Down => picker.list.select(Some((current + 1).min(len.saturating_sub(1)))),
            KeyCode::Up => picker.list.select(Some(current.saturating_sub(1))),
            KeyCode::Enter => {
                let node = picker.node;
                if let Some(collection) = self.context.collections.items.get(current) {
                    let owner = Owner::Collection { id: collection.id.clone(), name: collection.name.clone() };
                    self.toggle_node(node, &owner);
                }
            }
            _ => {}
        }
    }

    fn open_picker(&mut self) {
        let Some(node) = self.selected_node() else { return };
        if self.context.collections.items.is_empty() {
            self.message = Some("No collections. Press n in the Collections tab to create one.".into());
            return;
        }
        self.picker = Some(Picker { node, list: ListState::default().with_selected(Some(0)) });
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

    /// The mark of a node for the entries of a collection: all, some, or none of its fonts.
    pub fn collection_mark(&self, collection: &Collection, node: usize) -> Mark {
        let fonts = &self.context.library.fonts[self.tree.nodes[node].fonts.clone()];
        let count = fonts.iter().filter(|f| entry::matches_any(&collection.fonts, &f.id)).count();
        match count {
            0 => Mark::Off,
            n if n == fonts.len() => Mark::On,
            _ => Mark::Partial,
        }
    }

    /// (fonts enabled in the personal settings, all fonts) in a node.
    pub fn counts(&self, node: usize) -> (usize, usize) {
        let fonts = self.tree.nodes[node].fonts.clone();
        (self.explicit[fonts.clone()].iter().filter(|on| **on).count(), fonts.len())
    }

    /// The list state of the current tab and its number of rows.
    fn current_list(&mut self) -> (&mut ListState, usize) {
        match self.tab {
            Tab::Fonts => (&mut self.font_list, self.rows.len()),
            Tab::Collections => (&mut self.collection_list, self.context.collections.items.len()),
            Tab::Settings => (&mut self.settings_list, self.context.settings.full_format_order().len()),
            Tab::Missing => (&mut self.missing_list, self.missing.len()),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let (list, len) = self.current_list();
        if len == 0 {
            return;
        }
        let current = list.selected().unwrap_or(0) as isize;
        list.select(Some((current + delta).clamp(0, len as isize - 1) as usize));
    }

    /// Adds `c` to the jump text that is still open (`previous`), or starts a new one. A new
    /// text selects the next visible row after the selected one (wrapping) whose name starts
    /// with it; a longer text can also keep the selected row. Case-insensitive.
    fn jump_to(&mut self, c: char, previous: Option<(String, Instant)>, now: Instant) {
        let selected = self.font_list.selected();
        let (text, start) = match previous {
            Some((mut text, _)) => {
                text.push(c);
                (text, selected.unwrap_or(0))
            }
            None => (c.to_string(), selected.map_or(0, |i| i + 1)),
        };
        let prefix = text.to_lowercase();
        let len = self.rows.len();
        let found = (0..len)
            .map(|offset| (start + offset) % len)
            .find(|&i| self.tree.nodes[self.rows[i].node].name.to_lowercase().starts_with(&prefix));
        if found.is_some() {
            self.font_list.select(found);
        }
        self.jump = Some((text, now));
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
        let owner = match self.edited_collection() {
            Some(c) => Owner::Collection { id: c.id.clone(), name: c.name.clone() },
            None => Owner::Settings,
        };
        self.toggle_node(node, &owner);
    }

    /// Disables a node whose fonts are all enabled in the settings or the collection, and
    /// enables any other node.
    fn toggle_node(&mut self, node: usize, owner: &Owner) {
        let path = &self.tree.nodes[node].path;
        let (on, label, entries) = match owner {
            Owner::Collection { id, .. } => {
                let Some(c) = self.context.collections.get(id) else { return };
                let on = self.collection_mark(c, node) != Mark::On;
                let label = format!(
                    "{} {path} {} {}",
                    if on { "Added" } else { "Removed" },
                    if on { "to" } else { "from" },
                    c.name
                );
                (on, label, &c.fonts)
            }
            Owner::Settings => {
                let on = self.mark(node) != Mark::On;
                (on, format!("{} {path}", if on { "Enabled" } else { "Disabled" }), &self.context.settings.enabled)
            }
        };
        let entries = self.tree.set_enabled(&self.context.library, entries, node, on);
        self.change_entries(&label, owner, |_| entries);
    }

    /// Changes the entries in the personal settings or in a collection file, then registers
    /// or unregisters only the fonts that the change affects.
    fn change_entries(&mut self, label: &str, owner: &Owner, change: impl FnOnce(&[String]) -> Vec<String>) {
        match owner {
            Owner::Settings => {
                let entries = change(&self.context.settings.enabled);
                self.update_settings(label, |s| s.enabled = entries);
            }
            Owner::Collection { id, name } => {
                let Some(collection) = self.context.collections.get(id) else {
                    self.message = Some(format!("The collection {name} does not exist anymore."));
                    return;
                };
                let mut collection = collection.clone();
                collection.fonts = change(&collection.fonts);
                let root = self.context.library.root.clone();
                self.change(label, |app| {
                    collections::save(&root, &collection)?;
                    app.reload_collections(Some(&collection.id));
                    Ok(())
                });
            }
        }
    }

    pub fn selected_missing(&self) -> Option<&MissingEntry> {
        self.missing_list.selected().and_then(|i| self.missing.get(i))
    }

    /// Reads the missing entries again. Keeps the selection at the same index.
    fn refresh_missing(&mut self) {
        self.missing = self.context.missing_entries();
        self.show_missing |= !self.missing.is_empty();
        let last = self.missing.len().checked_sub(1);
        self.missing_list.select(last.map(|last| self.missing_list.selected().unwrap_or(0).min(last)));
    }

    fn start_replacing(&mut self) {
        let Some(missing) = self.selected_missing().cloned() else { return };
        self.message = Some(format!("Select the new location of {}. Enter chooses it. Esc cancels.", missing.entry));
        self.replacing = Some(missing);
        self.tab = Tab::Fonts;
    }

    fn stop_replacing(&mut self) {
        self.replacing = None;
        self.tab = Tab::Missing;
        self.message = None;
    }

    /// Asks to replace the missing entry with the selected folder or file.
    fn ask_replace(&mut self) {
        let Some(node) = self.selected_node() else { return };
        let target = self.tree.nodes[node].path.clone();
        self.input = Some(Input { prompt: Prompt::ConfirmReplace { target }, text: String::new() });
    }

    fn replace_missing(&mut self, target: &str) {
        let Some(missing) = self.replacing.take() else { return };
        self.tab = Tab::Missing;
        let label = format!("Replaced {} with {target}", missing.entry);
        self.change_entries(&label, &missing.owner, |entries| entry::replace(entries, &missing.entry, target));
    }

    fn remove_missing(&mut self, missing: &MissingEntry) {
        let label = format!("Removed {}", missing.entry);
        self.change_entries(&label, &missing.owner, |entries| {
            entries.iter().filter(|e| **e != missing.entry).cloned().collect()
        });
    }

    fn start_editing(&mut self) {
        let Some(collection) = self.selected_collection() else { return };
        let (id, name) = (collection.id.clone(), collection.name.clone());
        self.message = Some(format!("Editing {name}. Enter adds or removes fonts. Esc when done."));
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
        self.refresh_missing();
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

    /// Moves the selected format up (`delta` -1) or down (1) in the format order, and
    /// activates the files that the new order prefers.
    fn move_format(&mut self, delta: isize) -> Action {
        let mut order = self.context.settings.full_format_order();
        let from = self.settings_list.selected().unwrap_or(0);
        let to = (from as isize + delta).clamp(0, order.len() as isize - 1) as usize;
        if to == from {
            return Action::Continue;
        }
        order.swap(from, to);
        let label = format!("Moved {} to position {}", order[to], to + 1);
        self.settings_list.select(Some(to));
        self.update_settings(&label, |s| s.format_order = order);
        Action::Continue
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
        self.refresh_missing();

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
    use std::time::{Duration, Instant};

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
    fn enter_on_a_folder_enables_it_saves_the_settings_and_registers_the_fonts() {
        let f = fixture();
        let mut app = open(&f);

        press(&mut app, KeyCode::Enter);

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
    fn enter_on_a_file_in_an_enabled_folder_disables_only_that_file() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        select(&mut app, "Foundry A/Sans/Sans-Bold.otf");

        press(&mut app, KeyCode::Enter);

        assert_eq!(saved_settings(&f).enabled, ["Foundry A/Sans/Sans-Regular.otf", "Foundry A/Serif/"]);
        assert_eq!(app.mark(app.tree.find("Foundry A/").unwrap()), Mark::Partial);
        assert_eq!(app.counts(app.tree.find("Foundry A/").unwrap()), (2, 3));
        assert!(!registered(&app).contains(&"Foundry A/Sans/Sans-Bold.otf".to_string()));
    }

    #[test]
    fn enter_on_a_partial_folder_enables_all_of_it() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Right);
        select(&mut app, "Foundry A/Serif/");
        press(&mut app, KeyCode::Enter);
        select(&mut app, "Foundry A/");
        assert_eq!(app.mark(app.selected_node().unwrap()), Mark::Partial);

        press(&mut app, KeyCode::Enter);

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

    /// Presses a key at a time in milliseconds after `start`.
    fn press_at(app: &mut App, code: KeyCode, start: Instant, ms: u64) {
        app.handle_key_at(KeyEvent::from(code), start + Duration::from_millis(ms));
    }

    #[test]
    fn a_letter_jumps_to_the_next_visible_row_that_starts_with_it() {
        let f = fixture();
        let mut app = open(&f);
        let start = Instant::now();

        press_at(&mut app, KeyCode::Char('f'), start, 0); // the next match after the selected row
        assert_eq!(selected_path(&app), "Foundry B/");
        press_at(&mut app, KeyCode::Char('F'), start, 2000); // case-insensitive, wraps to the top
        assert_eq!(selected_path(&app), "Foundry A/");

        press(&mut app, KeyCode::Right);
        press_at(&mut app, KeyCode::Char('s'), start, 4000);
        assert_eq!(selected_path(&app), "Foundry A/Sans/");
        press_at(&mut app, KeyCode::Char('s'), start, 6000);
        assert_eq!(selected_path(&app), "Foundry A/Serif/");
        press_at(&mut app, KeyCode::Char('s'), start, 8000); // wraps to the first match
        assert_eq!(selected_path(&app), "Foundry A/Sans/");

        press_at(&mut app, KeyCode::Char('x'), start, 10000); // no match: the selection stays
        assert_eq!(selected_path(&app), "Foundry A/Sans/");
    }

    #[test]
    fn letters_typed_quickly_jump_to_the_row_that_starts_with_all_of_them() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Right);
        let start = Instant::now();

        press_at(&mut app, KeyCode::Char('s'), start, 0);
        assert_eq!(selected_path(&app), "Foundry A/Sans/");
        press_at(&mut app, KeyCode::Char('e'), start, 300); // "se"
        assert_eq!(selected_path(&app), "Foundry A/Serif/");
        press_at(&mut app, KeyCode::Char('r'), start, 700); // "ser": the pause is from the last key
        assert_eq!(selected_path(&app), "Foundry A/Serif/");

        press_at(&mut app, KeyCode::Char('s'), start, 3000);
        assert_eq!(selected_path(&app), "Foundry A/Sans/");
        press_at(&mut app, KeyCode::Char('e'), start, 3600); // too late: a new "e" has no match
        assert_eq!(selected_path(&app), "Foundry A/Sans/");
    }

    #[test]
    fn typed_text_can_contain_spaces_and_punctuation() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Right);
        select(&mut app, "Foundry A/Serif/");
        let start = Instant::now();

        for (i, c) in "foundry a".chars().enumerate() {
            press_at(&mut app, KeyCode::Char(c), start, i as u64 * 100);
        }
        assert_eq!(selected_path(&app), "Foundry A/");

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right); // opens Sans
        for (i, c) in "sans-r".chars().enumerate() {
            press_at(&mut app, KeyCode::Char(c), start, 5000 + i as u64 * 100);
        }
        assert_eq!(selected_path(&app), "Foundry A/Sans/Sans-Regular.otf");
    }

    #[test]
    fn a_space_alone_does_not_jump() {
        let f = fixture();
        let mut app = open(&f);
        let start = Instant::now();
        press_at(&mut app, KeyCode::Char(' '), start, 0);
        press_at(&mut app, KeyCode::Char('f'), start, 100); // a new jump, not " f"
        assert_eq!(selected_path(&app), "Foundry B/");
    }

    #[test]
    fn another_key_ends_the_typed_text() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Right);
        let start = Instant::now();

        press_at(&mut app, KeyCode::Char('s'), start, 0);
        press_at(&mut app, KeyCode::Down, start, 100);
        press_at(&mut app, KeyCode::Char('a'), start, 200); // a new jump, not "sa"
        assert_eq!(selected_path(&app), "Foundry A/Serif/"); // "sa" would select Sans
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
    fn tab_switches_to_collections_where_enter_toggles_a_collection() {
        let f = fixture();
        let mut app = open(&f);

        press(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Collections);
        press(&mut app, KeyCode::Enter);

        assert_eq!(saved_settings(&f).enabled_collections, ["c1"]);
        assert_eq!(registered(&app), ["Foundry B/Antiqua.otf"]);
        assert_eq!(app.mark(app.tree.find("Foundry B/Antiqua.otf").unwrap()), Mark::Collection);

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down); // stops at the last collection
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved_settings(&f).enabled_collections, ["c1", "c2"]);

        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved_settings(&f).enabled_collections, ["c2"]);
        assert!(registered(&app).is_empty());

        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.tab, Tab::Fonts);
    }

    #[test]
    fn tab_goes_from_collections_to_settings_and_back_to_fonts() {
        let f = fixture();
        let mut app = open(&f);

        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Settings);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Fonts);

        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.tab, Tab::Settings);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.tab, Tab::Collections);
    }

    /// Opens the Settings tab with an .otf + .ttf pair that shares a PostScript name in an
    /// enabled folder.
    fn open_settings_with_a_format_pair(f: &Fixture) -> App {
        let root = Context::load(&f.config).unwrap().library.root;
        write_font(&root, "Foundry C/Mono.otf", "Mono-Regular");
        write_font(&root, "Foundry C/Mono.ttf", "Mono-Regular");
        let mut settings = saved_settings(f);
        settings.enabled = vec!["Foundry C/".into()];
        f.config.save_settings(&settings).unwrap();
        let mut app = open(f);
        app::sync(&f.config, &Default::default(), app.activator.as_mut()).unwrap();
        app.context.state = f.config.load_state().unwrap();
        press(&mut app, KeyCode::BackTab);
        app
    }

    #[test]
    fn enter_picks_up_a_format_and_arrows_move_it() {
        let f = fixture();
        let mut app = open_settings_with_a_format_pair(&f);
        assert_eq!(registered(&app), ["Foundry C/Mono.otf"]);

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert!(app.moving);
        press(&mut app, KeyCode::Up);

        assert_eq!(saved_settings(&f).format_order, ["ttf", "otf", "ttc", "otc"]);
        assert_eq!(app.settings_list.selected(), Some(0));
        assert_eq!(registered(&app), ["Foundry C/Mono.ttf"]);
        assert_eq!(app.message.as_deref(), Some("Moved ttf to position 1. Registered 1, unregistered 1."));

        press(&mut app, KeyCode::Up); // already at the top
        assert_eq!(saved_settings(&f).format_order, ["ttf", "otf", "ttc", "otc"]);

        press(&mut app, KeyCode::Enter);
        assert!(!app.moving);
        press(&mut app, KeyCode::Down); // only moves the selection now
        assert_eq!(app.settings_list.selected(), Some(1));
        assert_eq!(saved_settings(&f).format_order, ["ttf", "otf", "ttc", "otc"]);
    }

    #[test]
    fn space_does_nothing_in_any_tab() {
        let f = fixture();
        let mut app = open(&f);

        press(&mut app, KeyCode::Char(' '));
        assert!(saved_settings(&f).enabled.is_empty());

        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Char(' '));
        assert!(!app.moving);

        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Char(' '));
        assert!(saved_settings(&f).enabled_collections.is_empty());
        assert_eq!(app.tab, Tab::Collections);
    }

    #[test]
    fn esc_puts_a_format_down_and_does_not_quit() {
        let f = fixture();
        let mut app = open_settings_with_a_format_pair(&f);
        press(&mut app, KeyCode::Enter);
        assert!(app.moving);
        assert_eq!(press(&mut app, KeyCode::Esc), Action::Continue);
        assert!(!app.moving);
    }

    #[test]
    fn tab_puts_a_format_down() {
        let f = fixture();
        let mut app = open_settings_with_a_format_pair(&f);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Fonts);
        assert!(!app.moving);
    }

    /// Draws the app in a terminal with a list area of 3 rows (from y 1 to 3), as the event
    /// loop does before it reads an event.
    fn draw(app: &mut App) {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 6)).unwrap();
        terminal.draw(|frame| view::render(app, frame)).unwrap();
    }

    fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
        app.handle_mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE });
        draw(app);
    }

    fn click(app: &mut App, column: u16, row: u16) {
        mouse(app, MouseEventKind::Down(MouseButton::Left), column, row);
    }

    /// Opens the Fonts tab with 6 rows and "Foundry A/Sans/" (row 1) selected.
    fn open_six_rows(f: &Fixture) -> App {
        let mut app = open(f);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        draw(&mut app);
        assert_eq!(app.rows.len(), 6);
        app
    }

    #[test]
    fn the_mouse_wheel_scrolls_the_view_and_not_the_selection() {
        let f = fixture();
        let mut app = open_six_rows(&f);

        for _ in 0..5 {
            mouse(&mut app, MouseEventKind::ScrollDown, 0, 2);
        }
        assert_eq!(app.font_list.offset(), 3); // stops when the last row is at the bottom
        assert_eq!(selected_path(&app), "Foundry A/Sans/");

        mouse(&mut app, MouseEventKind::ScrollUp, 0, 2);
        assert_eq!(app.font_list.offset(), 2);
        assert_eq!(selected_path(&app), "Foundry A/Sans/");
    }

    #[test]
    fn a_key_after_a_scroll_brings_the_selection_back_into_view() {
        let f = fixture();
        let mut app = open_six_rows(&f);
        for _ in 0..3 {
            mouse(&mut app, MouseEventKind::ScrollDown, 0, 2);
        }

        press(&mut app, KeyCode::Down);
        draw(&mut app);

        assert_eq!(selected_path(&app), "Foundry A/Sans/Sans-Bold.otf");
        assert_eq!(app.font_list.offset(), 2);
    }

    #[test]
    fn a_click_selects_the_row_under_the_pointer() {
        let f = fixture();
        let mut app = open_six_rows(&f);
        for _ in 0..3 {
            mouse(&mut app, MouseEventKind::ScrollDown, 0, 2);
        }

        click(&mut app, 10, 2);

        assert_eq!(selected_path(&app), "Foundry A/Serif/");
        assert_eq!(app.font_list.offset(), 3);

        click(&mut app, 10, 4); // below the list
        assert_eq!(selected_path(&app), "Foundry A/Serif/");
    }

    #[test]
    fn the_mouse_in_the_collections_tab_only_acts_on_the_collection_list() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        draw(&mut app);

        click(&mut app, 70, 2); // the typeface column
        assert_eq!(selected_collection(&app), "Brand");

        click(&mut app, 5, 2);
        assert_eq!(selected_collection(&app), "Editorial");
    }

    #[test]
    fn the_mouse_wheel_over_the_typefaces_scrolls_them() {
        let f = fixture();
        let mut app = open_with_all_collection(&f);
        press(&mut app, KeyCode::Tab);
        draw(&mut app);

        for _ in 0..5 {
            mouse(&mut app, MouseEventKind::ScrollDown, 70, 2);
        }
        assert_eq!(app.typeface_offset, 2); // stops when the last row is at the bottom
        assert_eq!(selected_collection(&app), "All");
        assert_eq!(app.collection_list.offset(), 0);

        mouse(&mut app, MouseEventKind::ScrollUp, 70, 2);
        assert_eq!(app.typeface_offset, 1);
    }

    #[test]
    fn the_mouse_wheel_over_the_collection_list_does_not_scroll_the_typefaces() {
        let f = fixture();
        let mut app = open_with_all_collection(&f);
        press(&mut app, KeyCode::Tab);
        draw(&mut app);

        mouse(&mut app, MouseEventKind::ScrollDown, 5, 2);

        assert_eq!(app.typeface_offset, 0);
    }

    #[test]
    fn another_collection_shows_its_typefaces_from_the_top() {
        let f = fixture();
        let mut app = open_with_all_collection(&f);
        press(&mut app, KeyCode::Tab);
        draw(&mut app);
        mouse(&mut app, MouseEventKind::ScrollDown, 70, 2);

        press(&mut app, KeyCode::Down);
        draw(&mut app);
        assert_eq!(app.typeface_offset, 0);

        press(&mut app, KeyCode::Up); // back to the scrolled collection
        draw(&mut app);
        assert_eq!(app.typeface_offset, 0);

        mouse(&mut app, MouseEventKind::ScrollDown, 70, 2);
        click(&mut app, 5, 2);
        assert_eq!(selected_collection(&app), "Brand");
        assert_eq!(app.typeface_offset, 0);
    }

    #[test]
    fn a_click_on_a_tab_title_switches_to_that_tab() {
        let f = fixture();
        let mut app = open(&f);
        draw(&mut app);

        // The header is " Fonts   Collections   Settings ".
        click(&mut app, 12, 0);
        assert_eq!(app.tab, Tab::Collections);
        click(&mut app, 25, 0);
        assert_eq!(app.tab, Tab::Settings);
        click(&mut app, 7, 0); // the gap between two titles
        assert_eq!(app.tab, Tab::Settings);
        click(&mut app, 2, 0);
        assert_eq!(app.tab, Tab::Fonts);
    }

    #[test]
    fn a_click_on_a_tab_title_ends_edit_mode_typing_and_moving() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('e'));
        draw(&mut app);

        click(&mut app, 2, 0); // Fonts is already the tab in edit mode
        assert_eq!(app.editing.as_deref(), Some("c1"));

        click(&mut app, 25, 0);
        assert_eq!(app.tab, Tab::Settings);
        assert_eq!(app.editing, None);

        press(&mut app, KeyCode::Enter);
        click(&mut app, 12, 0);
        assert!(!app.moving);

        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "sans");
        click(&mut app, 12, 0);
        assert_eq!(app.tab, Tab::Collections);
        assert!(!app.searching);
        assert_eq!(app.search, "sans");
    }

    #[test]
    fn the_mouse_does_nothing_while_a_prompt_is_open() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('n'));
        draw(&mut app);

        click(&mut app, 5, 2);

        assert_eq!(selected_collection(&app), "Brand");
    }

    #[test]
    fn a_click_in_the_settings_tab_selects_a_format_and_puts_the_picked_up_one_down() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Enter);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 12)).unwrap();
        terminal.draw(|frame| view::render(&mut app, frame)).unwrap();

        // The format list starts below the 3 intro lines: y 4 is "otf", y 6 is "ttc".
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 6,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.settings_list.selected(), Some(2));
        assert!(!app.moving);
        assert_eq!(saved_settings(&f).format_order, ["otf", "ttf", "ttc", "otc"]);
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
        press(&mut app, KeyCode::Enter);

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
        press(&mut app, KeyCode::Enter); // disable
        press(&mut app, KeyCode::Enter); // enable: registers Antiqua
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
        press(&mut app, KeyCode::Enter);

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
        press(&mut app, KeyCode::Char('e'));

        press(&mut app, KeyCode::Enter);

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

    /// Opens the fixture with two missing entries: "Old/Sans/" in the personal settings and
    /// "Gone.otf" in the collection Brand.
    /// Opens the fixture with an extra collection "All" that includes both foundries.
    pub(crate) fn open_with_all_collection(f: &Fixture) -> App {
        let root = app::Context::load(&f.config).unwrap().library.root;
        let json = br#"{"id": "c0", "name": "All", "fonts": ["Foundry A/", "Foundry B/"]}"#;
        write_file(&root, ".sorts/collections/all.json", json);
        open(f)
    }

    pub(crate) fn open_with_missing(f: &Fixture) -> App {
        let mut settings = saved_settings(f);
        settings.enabled = vec!["Old/Sans/".into(), "Foundry B/Antiqua.otf".into()];
        f.config.save_settings(&settings).unwrap();
        let json = br#"{"id": "c1", "name": "Brand", "fonts": ["Gone.otf", "Foundry B/Antiqua.otf"]}"#;
        write_file(&settings.root.unwrap(), ".sorts/collections/brand.json", json);
        open(f)
    }

    #[test]
    fn the_missing_tab_is_shown_only_when_entries_are_missing() {
        let f = fixture();
        let app = open(&f);
        assert_eq!(app.tabs(), [Tab::Fonts, Tab::Collections, Tab::Settings]);
        assert_eq!(app.message, None);

        let app = open_with_missing(&f);
        assert_eq!(app.tabs(), [Tab::Fonts, Tab::Collections, Tab::Settings, Tab::Missing]);
        assert_eq!(app.missing.len(), 2);
        assert_eq!(app.message.as_deref(), Some("2 entries point to fonts that do not exist. See the Missing tab."));
    }

    #[test]
    fn enter_on_a_missing_entry_starts_replace_mode_and_y_replaces_it() {
        let f = fixture();
        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.tab, Tab::Missing);

        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tab, Tab::Fonts);
        assert_eq!(app.replacing.as_ref().unwrap().entry, "Old/Sans/");
        press(&mut app, KeyCode::Right);
        select(&mut app, "Foundry A/Sans/");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.input, Some(Input { prompt: Prompt::ConfirmReplace { .. }, .. })));
        assert_eq!(saved_settings(&f).enabled, ["Old/Sans/", "Foundry B/Antiqua.otf"], "nothing changes before y");
        press(&mut app, KeyCode::Char('y'));

        assert_eq!(saved_settings(&f).enabled, ["Foundry A/Sans/", "Foundry B/Antiqua.otf"]);
        assert_eq!(registered(&app), ["Foundry A/Sans/Sans-Bold.otf", "Foundry A/Sans/Sans-Regular.otf"]);
        assert_eq!(app.replacing, None);
        assert_eq!(app.tab, Tab::Missing);
        assert_eq!(app.missing.len(), 1);
        assert_eq!(app.missing_list.selected(), Some(0));
        assert_eq!(
            app.message.as_deref(),
            Some("Replaced Old/Sans/ with Foundry A/Sans/. Registered 2, unregistered 0.")
        );
    }

    #[test]
    fn replacing_an_entry_of_a_collection_saves_the_collection_file() {
        let f = fixture();
        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        select(&mut app, "Foundry A/");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('y'));

        let root = saved_settings(&f).root.unwrap();
        assert_eq!(crate::collections::load(&root).get("c1").unwrap().fonts, ["Foundry A/", "Foundry B/Antiqua.otf"]);
        assert_eq!(saved_settings(&f).enabled, ["Old/Sans/", "Foundry B/Antiqua.otf"]);
        assert!(registered(&app).is_empty(), "the collection is not enabled");
        assert_eq!(app.missing.iter().map(|m| m.entry.as_str()).collect::<Vec<_>>(), ["Old/Sans/"]);
    }

    #[test]
    fn another_key_than_y_keeps_replace_mode_and_esc_or_tab_cancels_it() {
        let f = fixture();
        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('n'));

        assert!(app.input.is_none());
        assert!(app.replacing.is_some());
        assert_eq!(saved_settings(&f).enabled, ["Old/Sans/", "Foundry B/Antiqua.otf"]);

        assert_eq!(press(&mut app, KeyCode::Esc), Action::Continue);
        assert_eq!(app.replacing, None);
        assert_eq!(app.tab, Tab::Missing);

        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.replacing, None);
        assert_eq!(app.tab, Tab::Missing);
        assert_eq!(app.missing.len(), 2);
    }

    #[test]
    fn d_then_y_removes_a_missing_entry() {
        let f = fixture();
        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);

        press(&mut app, KeyCode::Char('d'));
        assert!(matches!(app.input, Some(Input { prompt: Prompt::ConfirmRemove { .. }, .. })));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.missing.len(), 2);

        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(saved_settings(&f).enabled, ["Foundry B/Antiqua.otf"]);
        assert_eq!(app.missing.len(), 1);

        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('y'));
        let root = saved_settings(&f).root.unwrap();
        assert_eq!(crate::collections::load(&root).get("c1").unwrap().fonts, ["Foundry B/Antiqua.otf"]);

        // The tab stays, with an empty list.
        assert!(app.missing.is_empty());
        assert_eq!(app.tab, Tab::Missing);
        assert!(app.tabs().contains(&Tab::Missing));
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_none());
        assert_eq!(app.replacing, None);
    }

    #[test]
    fn j_and_k_do_not_move_in_the_missing_tab() {
        let f = fixture();
        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.missing_list.selected(), Some(0));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.missing_list.selected(), Some(1));
    }

    fn picker_marks(app: &App) -> Vec<(&str, Mark)> {
        let node = app.picker.as_ref().unwrap().node;
        app.context.collections.items.iter().map(|c| (c.name.as_str(), app.collection_mark(c, node))).collect()
    }

    fn saved_collection(f: &Fixture, id: &str) -> Vec<String> {
        let root = f.config.load_settings().unwrap().root.unwrap();
        crate::collections::load(&root).get(id).unwrap().fonts.clone()
    }

    #[test]
    fn plus_opens_the_collection_picker_for_the_selected_node_and_esc_closes_it() {
        let f = fixture();
        let mut app = open(&f);
        select(&mut app, "Foundry B/");

        press(&mut app, KeyCode::Char('+'));
        let picker = app.picker.as_ref().expect("picker is open");
        assert_eq!(picker.node, app.tree.find("Foundry B/").unwrap());
        assert_eq!(picker.list.selected(), Some(0));
        assert_eq!(picker_marks(&app), [("Brand", Mark::Partial), ("Editorial", Mark::Off)]);

        assert_eq!(press(&mut app, KeyCode::Esc), Action::Continue);
        assert!(app.picker.is_none());
        assert_eq!(app.tab, Tab::Fonts);
    }

    #[test]
    fn enter_in_the_picker_adds_the_node_to_the_collection_and_keeps_the_picker_open() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Char('+'));
        press(&mut app, KeyCode::Down);

        press(&mut app, KeyCode::Enter);

        assert_eq!(saved_collection(&f, "c2"), ["Foundry A/"]);
        assert!(saved_settings(&f).enabled.is_empty());
        assert!(registered(&app).is_empty(), "the collection is not enabled");
        assert_eq!(app.message.as_deref(), Some("Added Foundry A/ to Editorial. Registered 0, unregistered 0."));
        assert_eq!(picker_marks(&app), [("Brand", Mark::Off), ("Editorial", Mark::On)]);
        assert_eq!(app.picker.as_ref().unwrap().list.selected(), Some(1));
        assert_eq!(app.mark(app.tree.find("Foundry A/").unwrap()), Mark::Off, "marks show the personal settings");

        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved_collection(&f, "c1"), ["Foundry A/", "Foundry B/Antiqua.otf"]);
    }

    #[test]
    fn enter_in_the_picker_on_a_full_collection_removes_the_node() {
        let f = fixture();
        let mut app = open_with_all_collection(&f);
        select(&mut app, "Foundry A/");
        press(&mut app, KeyCode::Right);
        select(&mut app, "Foundry A/Sans/");
        press(&mut app, KeyCode::Char('+'));
        assert_eq!(picker_marks(&app)[0], ("All", Mark::On));

        press(&mut app, KeyCode::Enter);

        let fonts = saved_collection(&f, "c0");
        assert!(!fonts.iter().any(|e| e.starts_with("Foundry A/Sans/") || e == "Foundry A/"), "{fonts:?}");
        assert!(fonts.contains(&"Foundry A/Serif/".to_string()), "{fonts:?}");
        assert!(fonts.contains(&"Foundry B/".to_string()), "{fonts:?}");
        assert_eq!(picker_marks(&app)[0], ("All", Mark::Off));
        assert!(app.message.as_deref().unwrap().starts_with("Removed Foundry A/Sans/ from All."));
    }

    #[test]
    fn a_picker_change_to_an_enabled_collection_registers_the_fonts() {
        let f = fixture();
        enable_collection(&f, "c2");
        let mut app = open(&f);
        press(&mut app, KeyCode::Char('+'));
        press(&mut app, KeyCode::Down);

        press(&mut app, KeyCode::Enter);

        assert_eq!(registered(&app).len(), 3);
        assert_eq!(app.mark(app.tree.find("Foundry A/").unwrap()), Mark::Collection);
    }

    #[test]
    fn plus_without_collections_shows_a_message() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        for _ in 0..2 {
            press(&mut app, KeyCode::Char('d'));
            press(&mut app, KeyCode::Char('y'));
        }
        press(&mut app, KeyCode::BackTab);

        press(&mut app, KeyCode::Char('+'));

        assert!(app.picker.is_none());
        assert_eq!(app.message.as_deref(), Some("No collections. Press n in the Collections tab to create one."));
    }

    #[test]
    fn plus_during_a_jump_is_jump_text() {
        let f = fixture();
        let mut app = open(&f);
        let start = Instant::now();
        press_at(&mut app, KeyCode::Char('f'), start, 0);
        press_at(&mut app, KeyCode::Char('+'), start, 100);
        assert!(app.picker.is_none());

        press_at(&mut app, KeyCode::Char('+'), start, 1000);
        assert!(app.picker.is_some());
    }

    #[test]
    fn plus_does_nothing_in_edit_mode_and_in_replace_mode() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('+'));
        assert!(app.picker.is_none());

        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('+'));
        assert!(app.picker.is_none());
    }

    #[test]
    fn the_picker_gets_all_keys_but_ctrl_c_still_quits() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Char('+'));
        for code in [KeyCode::Tab, KeyCode::Char('/'), KeyCode::Char('b'), KeyCode::Right] {
            press(&mut app, code);
        }
        assert!(app.picker.is_some());
        assert_eq!(app.tab, Tab::Fonts);
        assert!(!app.searching);
        assert!(app.expanded.is_empty());
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.handle_key(ctrl_c), Action::Quit);
    }

    #[test]
    fn the_mouse_does_nothing_while_the_picker_is_open() {
        let f = fixture();
        let mut app = open(&f);
        draw(&mut app);
        press(&mut app, KeyCode::Char('+'));
        click(&mut app, 3, 2);
        click(&mut app, 15, 0);
        assert_eq!(selected_path(&app), "Foundry A/");
        assert_eq!(app.tab, Tab::Fonts);
    }
}
