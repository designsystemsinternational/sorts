//! Draws the interactive list.

use super::{App, Input, Mark, Prompt, Tab};
use crate::app::{MissingEntry, Owner};
use crate::tree::Kind;
use crate::typefaces::typefaces;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

pub fn render(app: &mut App, frame: &mut Frame) {
    let [header, body, message, help] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1), Constraint::Length(1)])
            .areas(frame.area());

    frame.render_widget(header_line(app), header);
    app.tab_areas = tab_areas(header, &app.tabs());
    app.list_area = Rect::default();
    app.typeface_area = Rect::default();
    let scrolled = app.scrolled;
    match app.tab {
        Tab::Fonts => {
            let items = font_items(app);
            app.list_area = body;
            render_list(frame, body, items, &mut app.font_list, scrolled);
        }
        Tab::Collections if app.context.collections.items.is_empty() => {
            frame.render_widget(Paragraph::new(" No collections. Press n to create one.").dim(), body);
        }
        Tab::Collections => {
            let [left, right] = Layout::horizontal([Constraint::Percentage(40), Constraint::Fill(1)]).areas(body);
            let items = collection_items(app);
            app.list_area = left;
            render_list(frame, left, items, &mut app.collection_list, scrolled);
            app.typeface_area = right;
            let lines = typeface_lines(app, right.height as usize);
            frame.render_widget(Paragraph::new(lines).block(Block::new().borders(Borders::LEFT)), right);
        }
        Tab::Settings => {
            let [intro, formats] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(body);
            let lines = vec![
                Line::from(" Format order").bold(),
                Line::from(" When files have the same PostScript name, sorts activates the first format.").dim(),
                Line::from(""),
            ];
            frame.render_widget(Paragraph::new(lines), intro);
            let items = format_items(app);
            app.list_area = formats;
            render_list(frame, formats, items, &mut app.settings_list, scrolled);
        }
        Tab::Missing => {
            let [intro, entries] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(body);
            let lines = vec![
                Line::from(" Missing fonts").bold(),
                Line::from(" These entries point to files or folders that do not exist.").dim(),
                Line::from(""),
            ];
            frame.render_widget(Paragraph::new(lines), intro);
            if app.missing.is_empty() {
                frame.render_widget(Paragraph::new(" All entries point to fonts.").dim(), entries);
            } else {
                let items = missing_items(app);
                app.list_area = entries;
                render_list(frame, entries, items, &mut app.missing_list, scrolled);
            }
        }
    }
    if app.picker.is_some() {
        render_picker(app, frame, body);
    }
    frame.render_widget(message_line(app), message);
    frame.render_widget(help_line(app), help);
}

/// The collection picker: a box over the font tree with one row per collection.
fn render_picker(app: &mut App, frame: &mut Frame, body: Rect) {
    let Some(picker) = &app.picker else { return };
    let node = picker.node;
    let items: Vec<ListItem<'static>> = app
        .context
        .collections
        .items
        .iter()
        .map(|c| ListItem::new(format!(" {} {}", checkbox(app.collection_mark(c, node)), c.name)))
        .collect();
    let title = format!(" Collections for {} ", app.tree.nodes[node].path);
    let width = (title.chars().count() as u16 + 4).max(40).min(body.width);
    let height = (items.len() as u16 + 2).min(body.height);
    let area = Rect::new(body.x + (body.width - width) / 2, body.y + (body.height - height) / 2, width, height);
    frame.render_widget(Clear, area);
    let block = Block::bordered().title(title.bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let picker = app.picker.as_mut().expect("picker is open");
    frame.render_stateful_widget(list(items), inner, &mut picker.list);
}

fn title(tab: Tab) -> &'static str {
    match tab {
        Tab::Fonts => "Fonts",
        Tab::Collections => "Collections",
        Tab::Settings => "Settings",
        Tab::Missing => "Missing",
    }
}

/// The area of each tab title in the header: " Title ", with one space between titles.
fn tab_areas(header: Rect, tabs: &[Tab]) -> Vec<(Tab, Rect)> {
    let mut x = header.x;
    tabs.iter()
        .map(|tab| {
            let width = title(*tab).len() as u16 + 2;
            let area = Rect::new(x, header.y, width, 1).intersection(header);
            x += width + 1;
            (*tab, area)
        })
        .collect()
}

fn header_line(app: &App) -> Line<'static> {
    let mut spans = vec![];
    for (index, tab) in app.tabs().into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::from(" "));
        }
        let span = Span::from(format!(" {} ", title(tab)));
        spans.push(if app.tab == tab { span.reversed() } else { span });
    }
    if let Some(collection) = app.edited_collection() {
        spans.push(Span::from(format!("  Editing: {}", collection.name)).bold().magenta());
    }
    if let Some(missing) = &app.replacing {
        spans.push(Span::from(format!("  Replace: {}", missing.entry)).bold().magenta());
    }
    if app.searching || !app.search.is_empty() {
        spans.push(Span::from(format!("   / {}", app.search)));
        if app.searching {
            spans.push(Span::from("_").slow_blink());
        }
    }
    Line::from(spans)
}

fn message_line(app: &App) -> Line<'static> {
    let Some(input) = &app.input else {
        return Line::from(format!(" {}", app.message.as_deref().unwrap_or_default())).yellow();
    };
    let name = |id: &str| app.context.collections.get(id).map(|c| c.name.clone()).unwrap_or_default();
    match &input.prompt {
        Prompt::NewCollection => Line::from(format!(" New collection name: {}_", input.text)),
        Prompt::Rename { .. } => Line::from(format!(" Rename collection: {}_", input.text)),
        Prompt::ConfirmDelete { id } => {
            Line::from(format!(" Delete collection \"{}\" for the whole team? y/n", name(id))).red().bold()
        }
        Prompt::ConfirmReplace { target } => {
            let Some(missing) = &app.replacing else { return Line::default() };
            let question = format!(" Replace \"{}\" with \"{target}\"", missing.entry);
            confirmation(question, &missing.owner, "in")
        }
        Prompt::ConfirmRemove { missing } => {
            let MissingEntry { owner, entry } = missing;
            match owner {
                Owner::Settings => Line::from(format!(" Remove \"{entry}\" from your settings? y/n")).bold(),
                Owner::Collection { .. } => confirmation(format!(" Remove \"{entry}\""), owner, "from"),
            }
        }
    }
}

/// A y/n question. A change to a collection affects the whole team, so the question says so.
fn confirmation(question: String, owner: &Owner, preposition: &str) -> Line<'static> {
    match owner {
        Owner::Settings => Line::from(format!("{question}? y/n")).bold(),
        Owner::Collection { name, .. } => {
            Line::from(format!("{question} {preposition} \"{name}\" for the whole team? y/n")).red().bold()
        }
    }
}

fn help_line(app: &App) -> Line<'static> {
    // With a kept search filter, esc clears the filter first.
    let esc = match () {
        _ if !app.search.is_empty() => ("Esc", "Clear"),
        _ if app.editing.is_some() => ("Esc", "Done"),
        _ if app.replacing.is_some() => ("Esc", "Cancel"),
        _ => ("Esc", "Quit"),
    };
    let keys: Vec<(&str, &str)> = match &app.input {
        Some(Input { prompt: Prompt::ConfirmDelete { .. }, .. }) => vec![("Y", "Delete"), ("Other keys", "Cancel")],
        Some(Input { prompt: Prompt::ConfirmReplace { .. }, .. }) => vec![("Y", "Replace"), ("Other keys", "Cancel")],
        Some(Input { prompt: Prompt::ConfirmRemove { .. }, .. }) => vec![("Y", "Remove"), ("Other keys", "Cancel")],
        Some(_) => vec![("Enter", "Save"), ("Esc", "Cancel")],
        None if app.picker.is_some() => vec![("Enter", "Add/remove"), ("↑↓", "Select"), ("Esc", "Done")],
        None if app.searching => vec![("Type", "Search"), ("Enter", "Done"), ("Esc", "Clear")],
        None if app.editing.is_some() => {
            vec![("Enter", "Add/remove"), ("←→", "Close/open"), ("A-Z", "Jump"), ("/", "Search"), esc]
        }
        None if app.replacing.is_some() => {
            vec![("Enter", "Choose"), ("←→", "Close/open"), ("A-Z", "Jump"), ("/", "Search"), esc]
        }
        None if app.tab == Tab::Fonts => {
            vec![
                ("Enter", "Toggle"),
                ("←→", "Close/open"),
                ("A-Z", "Jump"),
                ("/", "Search"),
                ("+", "Collections"),
                ("Tab", "Collections"),
                esc,
            ]
        }
        None if app.moving => vec![("↑↓", "Move"), ("Enter", "Done")],
        None if app.tab == Tab::Settings && app.tabs().contains(&Tab::Missing) => {
            vec![("Enter", "Pick up and move"), ("↑↓", "Select"), ("Tab", "Missing"), esc]
        }
        None if app.tab == Tab::Settings => {
            vec![("Enter", "Pick up and move"), ("↑↓", "Select"), ("Tab", "Fonts"), esc]
        }
        None if app.tab == Tab::Missing && app.missing.is_empty() => vec![("Tab", "Fonts"), esc],
        None if app.tab == Tab::Missing => {
            vec![("Enter", "Replace"), ("D", "Remove"), ("↑↓", "Select"), ("Tab", "Fonts"), esc]
        }
        None => vec![("Enter", "Enable/disable"), ("E", "Edit"), ("N", "New"), ("R", "Rename"), ("D", "Delete"), esc],
    };
    let text = keys.iter().map(|(key, action)| format!("{key} : {action}")).collect::<Vec<_>>().join(" | ");
    Line::from(format!(" {text}")).dim()
}

fn list(items: Vec<ListItem<'static>>) -> List<'static> {
    List::new(items).highlight_style(Style::new().reversed())
}

/// After a mouse scroll, the selected row can be outside the view. Then the list is drawn
/// without the selection, because ratatui scrolls a list back to its selected row.
fn render_list(frame: &mut Frame, area: Rect, items: Vec<ListItem<'static>>, state: &mut ListState, scrolled: bool) {
    let in_view =
        state.selected().is_some_and(|i| (state.offset()..state.offset() + area.height as usize).contains(&i));
    if scrolled && !in_view {
        let mut unselected = ListState::default().with_offset(state.offset());
        frame.render_stateful_widget(list(items), area, &mut unselected);
        *state.offset_mut() = unselected.offset();
    } else {
        frame.render_stateful_widget(list(items), area, state);
    }
}

fn checkbox(mark: Mark) -> &'static str {
    match mark {
        Mark::On => "[x]",
        Mark::Partial => "[~]",
        Mark::Collection => "[c]",
        Mark::Off => "[ ]",
    }
}

fn font_items(app: &App) -> Vec<ListItem<'static>> {
    app.rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let node = &app.tree.nodes[row.node];
            // A folder is open when the next row is one of its children.
            let open = app.rows.get(index + 1).is_some_and(|next| next.depth > row.depth);
            let arrow = match (node.kind, open) {
                (Kind::File, _) => "  ",
                (Kind::Folder, true) => "▾ ",
                (Kind::Folder, false) => "▸ ",
            };
            let mut spans = vec![Span::from(format!(
                " {}{arrow}{} {}",
                "  ".repeat(row.depth),
                checkbox(app.mark(row.node)),
                node.name
            ))];
            match node.kind {
                Kind::Folder => {
                    let (enabled, total) = app.counts(row.node);
                    spans.push(Span::from(format!("  {enabled}/{total}")).dim());
                }
                Kind::File => {
                    let font = &app.context.library.fonts[node.fonts.start];
                    if font.error.is_some() {
                        spans.push(Span::from("  unreadable").red());
                    } else if let Some(face) = font.faces.first() {
                        spans.push(Span::from(format!("  {} {}", face.family, face.style)).dim());
                    }
                }
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

fn collection_items(app: &App) -> Vec<ListItem<'static>> {
    app.context
        .collections
        .items
        .iter()
        .map(|collection| {
            let on = app.context.settings.enabled_collections.contains(&collection.id);
            let count = typefaces(&app.context.library, &collection.fonts).items.len();
            ListItem::new(Line::from(vec![
                Span::from(format!(" {} {}", if on { "[x]" } else { "[ ]" }, collection.name)),
                Span::from(format!("  {}", plural(count, "typeface"))).dim(),
            ]))
        })
        .collect()
}

fn format_items(app: &App) -> Vec<ListItem<'static>> {
    let selected = app.settings_list.selected();
    app.context
        .settings
        .full_format_order()
        .into_iter()
        .enumerate()
        .map(|(index, format)| {
            let mut spans = vec![Span::from(format!(" {}  {format}", index + 1))];
            if app.moving && selected == Some(index) {
                spans.push(Span::from("  ⇅").bold());
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

fn missing_items(app: &App) -> Vec<ListItem<'static>> {
    app.missing
        .iter()
        .map(|missing| {
            let owner = match &missing.owner {
                Owner::Settings => "your settings".to_string(),
                Owner::Collection { name, .. } => format!("collection {name}"),
            };
            ListItem::new(Line::from(vec![
                Span::from(format!(" {}", missing.entry)),
                Span::from(format!("  {owner}")).dim(),
            ]))
        })
        .collect()
}

/// "1 style", "2 styles".
fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

/// The typefaces of the selected collection, at most `height` lines, from `App::typeface_offset`.
/// The offset starts at 0 for each newly selected collection and stops when the last row is at
/// the bottom. If rows are below the view, the last line says how many.
fn typeface_lines(app: &mut App, height: usize) -> Vec<Line<'static>> {
    let Some(collection) = app.selected_collection() else { return vec![] };
    let collection = collection.clone();
    if app.typeface_collection.as_deref() != Some(collection.id.as_str()) {
        app.typeface_collection = Some(collection.id.clone());
        app.typeface_offset = 0;
    }
    let mut lines = vec![Line::from(format!(" {}", collection.name)).bold()];
    let typefaces = typefaces(&app.context.library, &collection.fonts);
    if typefaces.items.is_empty() && typefaces.unreadable == 0 {
        lines.push(Line::from(" No fonts. Press e to add some.").dim());
        return lines;
    }
    let mut rows: Vec<Line<'static>> = typefaces
        .items
        .iter()
        .map(|t| {
            Line::from(vec![
                Span::from(format!(" {} ", t.name)),
                Span::from(format!("({})", plural(t.styles, "style"))).dim(),
            ])
        })
        .collect();
    if typefaces.unreadable > 0 {
        rows.push(Line::from(format!(" Unreadable ({})", plural(typefaces.unreadable, "file"))).red());
    }
    let room = height.saturating_sub(1);
    let offset = app.typeface_offset.min(rows.len().saturating_sub(room));
    app.typeface_offset = offset;
    let mut rows = rows.split_off(offset);
    if rows.len() > room {
        let more = rows.len() - room.saturating_sub(1);
        rows.truncate(room.saturating_sub(1));
        rows.push(Line::from(format!(" ↓ {more} more")).dim());
    }
    lines.extend(rows);
    lines
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, open, open_with_all_collection, open_with_missing, press, type_text};
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, MouseEvent, MouseEventKind};

    fn draw(app: &mut App) -> String {
        draw_sized(app, 80, 12)
    }

    fn draw_sized(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(app, frame)).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn fonts_tab() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Right);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn fonts_tab_with_unreadable_file_and_partial_folder() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn search() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "regular");
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn search_kept_after_enter() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "regular");
        press(&mut app, KeyCode::Enter);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn collections_tab() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn collections_tab_shows_the_typefaces_of_the_selected_collection() {
        let f = fixture();
        let mut app = open_with_all_collection(&f);
        press(&mut app, KeyCode::Tab);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn collections_tab_with_an_empty_collection() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Down);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn collections_tab_with_more_typefaces_than_rows() {
        let f = fixture();
        let mut app = open_with_all_collection(&f);
        press(&mut app, KeyCode::Tab);
        insta::assert_snapshot!(draw_sized(&mut app, 80, 7));
    }

    #[test]
    fn collections_tab_with_the_typefaces_scrolled() {
        let f = fixture();
        let mut app = open_with_all_collection(&f);
        press(&mut app, KeyCode::Tab);
        draw_sized(&mut app, 80, 7);
        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 70,
            row: 2,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(wheel);
        insta::assert_snapshot!(draw_sized(&mut app, 80, 7));
    }

    #[test]
    fn new_collection_prompt() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "Studio");
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn delete_confirmation() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('d'));
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn settings_tab() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Down);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn settings_tab_with_a_format_picked_up() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Up);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn fonts_tab_scrolled_away_from_the_selection() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Right);
        draw_sized(&mut app, 80, 5);
        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 1,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(wheel);
        app.handle_mouse(wheel);
        insta::assert_snapshot!(draw_sized(&mut app, 80, 5));
    }

    #[test]
    fn edit_mode() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn collection_picker() {
        let f = fixture();
        let mut app = open_with_all_collection(&f);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('+'));
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn missing_tab() {
        let f = fixture();
        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn missing_tab_with_nothing_left() {
        let f = fixture();
        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);
        for _ in 0..2 {
            press(&mut app, KeyCode::Char('d'));
            press(&mut app, KeyCode::Char('y'));
        }
        app.message = None;
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn replace_mode() {
        let f = fixture();
        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Right);
        insta::assert_snapshot!(draw(&mut app));
    }

    #[test]
    fn replace_confirmation_for_a_collection() {
        let f = fixture();
        let mut app = open_with_missing(&f);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        insta::assert_snapshot!(draw(&mut app));
    }
}
