//! Groups the fonts of a list of entries (e.g. a collection) by typeface.

use crate::entry;
use crate::scan::Library;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typeface {
    pub name: String,
    /// The number of different styles (PostScript names) in the typeface.
    pub styles: usize,
}

/// The typefaces of the fonts that the entries include, sorted by name.
pub fn typefaces<S: AsRef<str>>(library: &Library, entries: &[S]) -> Typefaces {
    let mut families: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut unreadable = 0;
    for font in library.fonts.iter().filter(|f| entry::matches_any(entries, &f.id)) {
        if font.error.is_some() {
            unreadable += 1;
        }
        for face in &font.faces {
            families.entry(&face.family).or_default().insert(&face.postscript);
        }
    }
    let mut items: Vec<_> =
        families.into_iter().map(|(name, styles)| Typeface { name: name.to_string(), styles: styles.len() }).collect();
    items.sort_by_key(|t| t.name.to_lowercase());
    Typefaces { items, unreadable }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Typefaces {
    pub items: Vec<Typeface>,
    /// The number of included files that could not be read.
    pub unreadable: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{Face, FontFile};

    fn face(family: &str, postscript: &str) -> Face {
        Face { family: family.into(), style: String::new(), postscript: postscript.into() }
    }

    fn font(id: &str, faces: Vec<Face>) -> FontFile {
        FontFile { id: id.into(), faces, error: None }
    }

    fn library(mut fonts: Vec<FontFile>) -> Library {
        fonts.sort_by(|a, b| a.id.cmp(&b.id));
        Library { root: "/fonts".into(), fonts, downloaded: 0 }
    }

    fn typeface(name: &str, styles: usize) -> Typeface {
        Typeface { name: name.into(), styles }
    }

    #[test]
    fn groups_the_included_fonts_by_family() {
        let library = library(vec![
            font("A/Sans/Sans-Regular.otf", vec![face("Sans", "Sans-Regular")]),
            font("A/Sans/Sans-Bold.otf", vec![face("Sans", "Sans-Bold")]),
            font("A/Serif/Serif-Regular.otf", vec![face("Serif", "Serif-Regular")]),
            font("B/Mono.otf", vec![face("Mono", "Mono-Regular")]),
        ]);
        let result = typefaces(&library, &["A/Sans/", "B/Mono.otf"]);
        assert_eq!(result.items, vec![typeface("Mono", 1), typeface("Sans", 2)]);
        assert_eq!(result.unreadable, 0);
    }

    #[test]
    fn files_with_the_same_postscript_name_count_as_one_style() {
        let library = library(vec![
            font("A/Serif-Italic.otf", vec![face("Serif", "Serif-Italic")]),
            font("A/Serif-Italic.ttf", vec![face("Serif", "Serif-Italic")]),
            font("A/Serif-Regular.ttf", vec![face("Serif", "Serif-Regular")]),
        ]);
        assert_eq!(typefaces(&library, &["A/"]).items, vec![typeface("Serif", 2)]);
    }

    #[test]
    fn each_face_of_a_font_collection_counts_in_its_own_family() {
        let library = library(vec![font(
            "A/Family.ttc",
            vec![face("Sans", "Sans-Regular"), face("Sans", "Sans-Bold"), face("Serif", "Serif-Regular")],
        )]);
        assert_eq!(typefaces(&library, &["A/Family.ttc"]).items, vec![typeface("Sans", 2), typeface("Serif", 1)]);
    }

    #[test]
    fn unreadable_files_are_counted_separately() {
        let mut broken = font("A/Broken.ttf", vec![]);
        broken.error = Some("bad".into());
        let library = library(vec![broken, font("A/Sans.otf", vec![face("Sans", "Sans-Regular")])]);
        let result = typefaces(&library, &["A/"]);
        assert_eq!(result.items, vec![typeface("Sans", 1)]);
        assert_eq!(result.unreadable, 1);
    }

    #[test]
    fn typefaces_are_sorted_case_insensitively() {
        let library = library(vec![
            font("A/a.otf", vec![face("beta", "beta-Regular")]),
            font("A/b.otf", vec![face("Alpha", "Alpha-Regular")]),
            font("A/c.otf", vec![face("Gamma", "Gamma-Regular")]),
        ]);
        let names: Vec<_> = typefaces(&library, &["A/"]).items.into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["Alpha", "beta", "Gamma"]);
    }

    #[test]
    fn no_entries_give_no_typefaces() {
        let library = library(vec![font("A/Sans.otf", vec![face("Sans", "Sans-Regular")])]);
        assert_eq!(typefaces(&library, &[] as &[&str]), Typefaces::default());
    }
}
