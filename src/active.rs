//! Calculates the active set: the font files that should be registered.

use crate::collections::Collections;
use crate::entry;
use crate::scan::{FontFile, Library};
use crate::settings::Settings;
use std::collections::HashSet;

/// The IDs of the fonts to register, sorted.
///
/// active fonts = expand(enabled) + expand(each enabled collection)
///
/// Unreadable files are left out. When several files share a PostScript name, only the
/// file whose extension comes first in `format_order` is kept (ties: first ID in
/// alphabetical order).
pub fn active_fonts(library: &Library, settings: &Settings, collections: &Collections) -> Vec<String> {
    let mut entries: Vec<&str> = settings.enabled.iter().map(String::as_str).collect();
    for id in &settings.enabled_collections {
        if let Some(collection) = collections.get(id) {
            entries.extend(collection.fonts.iter().map(String::as_str));
        }
    }

    let rank = |font: &FontFile| {
        let ext = font.extension();
        settings.format_order.iter().position(|f| f.eq_ignore_ascii_case(&ext)).unwrap_or(usize::MAX)
    };
    let mut candidates: Vec<&FontFile> =
        library.fonts.iter().filter(|f| f.error.is_none() && entry::matches_any(&entries, &f.id)).collect();
    candidates.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.id.cmp(&b.id)));

    let mut taken: HashSet<&str> = HashSet::new();
    let mut active = Vec::new();
    for font in candidates {
        let names: Vec<&str> = font.faces.iter().map(|f| f.postscript.as_str()).filter(|ps| !ps.is_empty()).collect();
        if names.iter().any(|ps| taken.contains(ps)) {
            continue;
        }
        taken.extend(names);
        active.push(font.id.clone());
    }
    active.sort();
    active
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::Collection;
    use crate::scan::Face;

    fn font(id: &str, postscripts: &[&str]) -> FontFile {
        FontFile {
            id: id.into(),
            faces: postscripts
                .iter()
                .map(|ps| Face { family: String::new(), style: String::new(), postscript: ps.to_string() })
                .collect(),
            error: None,
        }
    }

    fn library(mut fonts: Vec<FontFile>) -> Library {
        fonts.sort_by(|a, b| a.id.cmp(&b.id));
        Library { root: "/fonts".into(), fonts, downloaded: 0 }
    }

    fn enabled(entries: &[&str]) -> Settings {
        Settings { enabled: entries.iter().map(|e| e.to_string()).collect(), ..Settings::default() }
    }

    fn none() -> Collections {
        Collections::default()
    }

    #[test]
    fn nothing_enabled_means_nothing_active() {
        let library = library(vec![font("A/A.otf", &["A"])]);
        assert!(active_fonts(&library, &Settings::default(), &none()).is_empty());
    }

    #[test]
    fn enabled_file_and_folder_entries_are_expanded() {
        let library = library(vec![
            font("A/One.otf", &["A1"]),
            font("A/Deep/Two.otf", &["A2"]),
            font("B/One.otf", &["B1"]),
            font("B/Two.otf", &["B2"]),
        ]);

        let active = active_fonts(&library, &enabled(&["A/", "B/Two.otf"]), &none());

        assert_eq!(active, ["A/Deep/Two.otf", "A/One.otf", "B/Two.otf"]);
    }

    #[test]
    fn enabled_collections_add_their_fonts() {
        let library = library(vec![font("A/One.otf", &["A1"]), font("B/One.otf", &["B1"]), font("C/One.otf", &["C1"])]);
        let collections = Collections {
            items: vec![
                Collection { id: "on".into(), name: "On".into(), fonts: vec!["A/".into()], ..Default::default() },
                Collection { id: "off".into(), name: "Off".into(), fonts: vec!["B/".into()], ..Default::default() },
            ],
            problems: vec![],
        };
        let settings = Settings {
            enabled: vec!["C/One.otf".into()],
            enabled_collections: vec!["on".into(), "deleted".into()],
            ..Settings::default()
        };

        let active = active_fonts(&library, &settings, &collections);

        assert_eq!(active, ["A/One.otf", "C/One.otf"]);
    }

    #[test]
    fn unreadable_files_are_left_out() {
        let mut broken = font("A/Broken.ttf", &[]);
        broken.error = Some("unknown magic".into());
        let library = library(vec![broken, font("A/Good.otf", &["Good"])]);

        assert_eq!(active_fonts(&library, &enabled(&["A/"]), &none()), ["A/Good.otf"]);
    }

    #[test]
    fn same_postscript_name_keeps_the_preferred_format() {
        let library = library(vec![font("A/X.ttf", &["X"]), font("A/X.otf", &["X"]), font("A/Y.ttf", &["Y"])]);

        assert_eq!(active_fonts(&library, &enabled(&["A/"]), &none()), ["A/X.otf", "A/Y.ttf"]);

        let ttf_first = Settings { format_order: vec!["ttf".into(), "otf".into()], ..enabled(&["A/"]) };
        assert_eq!(active_fonts(&library, &ttf_first, &none()), ["A/X.ttf", "A/Y.ttf"]);
    }

    #[test]
    fn same_postscript_name_and_format_keeps_the_first_id() {
        let library = library(vec![font("B/X.otf", &["X"]), font("A/X.otf", &["X"])]);
        assert_eq!(active_fonts(&library, &enabled(&["A/", "B/"]), &none()), ["A/X.otf"]);
    }

    #[test]
    fn formats_that_are_not_in_the_order_come_last() {
        let library = library(vec![font("A/X.dfont", &["X"]), font("A/X.ttf", &["X"])]);
        let settings = Settings { format_order: vec!["ttf".into()], ..enabled(&["A/"]) };
        assert_eq!(active_fonts(&library, &settings, &none()), ["A/X.ttf"]);
    }

    #[test]
    fn a_collection_file_is_left_out_if_one_of_its_faces_is_taken() {
        let library = library(vec![font("A/Family.ttc", &["Regular", "Bold"]), font("A/Bold.otf", &["Bold"])]);
        assert_eq!(active_fonts(&library, &enabled(&["A/"]), &none()), ["A/Bold.otf"]);
    }
}
