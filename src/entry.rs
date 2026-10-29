//! Entries in the personal settings and in collections.
//!
//! An entry is a path relative to the Fonts root. A folder entry ends with `/` and
//! includes all fonts in the folder and its subfolders. Any other entry is one font file.

use crate::scan::Library;

pub fn is_folder(entry: &str) -> bool {
    entry.ends_with('/')
}

/// True if the entry includes the font with this ID.
pub fn matches(entry: &str, font_id: &str) -> bool {
    if is_folder(entry) { font_id.starts_with(entry) } else { font_id == entry }
}

/// True if any of the entries includes the font with this ID.
pub fn matches_any<S: AsRef<str>>(entries: &[S], font_id: &str) -> bool {
    entries.iter().any(|e| matches(e.as_ref(), font_id))
}

/// True if the entry points to at least one font in the library.
pub fn exists(entry: &str, library: &Library) -> bool {
    library.fonts.iter().any(|f| matches(entry, &f.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::FontFile;

    fn library(ids: &[&str]) -> Library {
        Library {
            root: "/fonts".into(),
            fonts: ids.iter().map(|id| FontFile { id: id.to_string(), faces: vec![], error: None }).collect(),
        }
    }

    #[test]
    fn folder_entries_end_with_a_slash() {
        assert!(is_folder("Foundry A/"));
        assert!(!is_folder("Foundry A/Font.otf"));
    }

    #[test]
    fn a_file_entry_matches_only_that_file() {
        assert!(matches("A/B.otf", "A/B.otf"));
        assert!(!matches("A/B.otf", "A/B.otf.bak"));
        assert!(!matches("A/B.otf", "A/C.otf"));
    }

    #[test]
    fn a_folder_entry_matches_files_in_the_folder_and_its_subfolders() {
        assert!(matches("A/", "A/B.otf"));
        assert!(matches("A/", "A/Deep/C.otf"));
        assert!(!matches("A/", "AB/C.otf"));
        assert!(!matches("A/Deep/", "A/B.otf"));
    }

    #[test]
    fn matches_any_checks_all_entries() {
        let entries = ["A/", "B/One.otf"];
        assert!(matches_any(&entries, "A/X.otf"));
        assert!(matches_any(&entries, "B/One.otf"));
        assert!(!matches_any(&entries, "B/Two.otf"));
    }

    #[test]
    fn exists_is_true_if_the_entry_matches_a_font() {
        let library = library(&["A/Deep/X.otf", "B/One.otf"]);
        assert!(exists("A/", &library));
        assert!(exists("A/Deep/", &library));
        assert!(exists("B/One.otf", &library));
        assert!(!exists("B/Two.otf", &library));
        assert!(!exists("C/", &library));
    }
}
