//! Draws the interactive list.

use super::{App, Input, Mark, Prompt, Tab};
use crate::tree::Kind;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};

pub fn render(app: &mut App, frame: &mut Frame) {
    let [header, body, message, help] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1), Constraint::Length(1)])
            .areas(frame.area());

    frame.render_widget(header_line(app), header);
    match app.tab {
        Tab::Fonts => {
            let items = font_items(app);
            frame.render_stateful_widget(list(items), body, &mut app.font_list);
        }
        Tab::Collections if app.context.collections.items.is_empty() => {
            frame.render_widget(Paragraph::new(" No collections. Press n to create one.").dim(), body);
        }
        Tab::Collections => {
            let items = collection_items(app);
            frame.render_stateful_widget(list(items), body, &mut app.collection_list);
        }
    }
    frame.render_widget(message_line(app), message);
    frame.render_widget(help_line(app), help);
}

fn header_line(app: &App) -> Line<'static> {
    let tab = |title: &'static str, tab: Tab| {
        let span = Span::from(format!(" {title} "));
        if app.tab == tab { span.reversed() } else { span }
    };
    let mut spans = vec![tab("Fonts", Tab::Fonts), Span::from(" "), tab("Collections", Tab::Collections)];
    if let Some(collection) = app.edited_collection() {
        spans.push(Span::from(format!("  Editing: {}", collection.name)).bold().magenta());
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
    }
}

fn help_line(app: &App) -> Line<'static> {
    let text = match &app.input {
        Some(Input { prompt: Prompt::ConfirmDelete { .. }, .. }) => " y delete  any other key cancels",
        Some(_) => " enter save  esc cancel",
        None if app.searching => " type to search  enter done  esc clear",
        None if app.editing.is_some() => " space add/remove  ←→ close/open  a-z jump  / search  esc done",
        None if app.tab == Tab::Fonts => {
            " space toggle  ←→ close/open  a-z jump  / search  tab collections  ctrl-c quit"
        }
        None => " space on/off  e edit  n new  r rename  d delete  ctrl-c quit",
    };
    Line::from(text).dim()
}

fn list(items: Vec<ListItem<'static>>) -> List<'static> {
    List::new(items).highlight_style(Style::new().reversed())
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
            let count = collection.fonts.len();
            ListItem::new(Line::from(vec![
                Span::from(format!(" {} {}", if on { "[x]" } else { "[ ]" }, collection.name)),
                Span::from(format!("  {count} {}", if count == 1 { "entry" } else { "entries" })).dim(),
            ]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, open, press, type_text};
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    fn draw(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|frame| render(app, frame)).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn fonts_tab() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
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
        press(&mut app, KeyCode::Char(' '));
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
    fn collections_tab() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char(' '));
        insta::assert_snapshot!(draw(&mut app));
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
    fn edit_mode() {
        let f = fixture();
        let mut app = open(&f);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        insta::assert_snapshot!(draw(&mut app));
    }
}
