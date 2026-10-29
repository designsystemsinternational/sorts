//! Scans the Fonts folder and reads the names of each font file.

use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// The file extensions that the scanner includes (lowercase).
pub const FONT_EXTENSIONS: &[&str] = &["otf", "ttf", "ttc", "otc"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    pub family: String,
    pub style: String,
    pub postscript: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFile {
    /// Path relative to the Fonts root, with `/` separators. This is the font ID.
    pub id: String,
    pub faces: Vec<Face>,
    /// Set when the file could not be read as a font.
    pub error: Option<String>,
}

impl FontFile {
    /// The lowercase file extension, e.g. `otf`.
    pub fn extension(&self) -> String {
        Path::new(&self.id).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct Library {
    pub root: PathBuf,
    /// Sorted by ID.
    pub fonts: Vec<FontFile>,
}

impl Library {
    pub fn path_of(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn get(&self, id: &str) -> Option<&FontFile> {
        self.fonts.binary_search_by(|f| f.id.as_str().cmp(id)).ok().map(|i| &self.fonts[i])
    }
}

/// Reads the faces of a font or font collection.
pub fn read_faces(data: &[u8]) -> Result<Vec<Face>, String> {
    use ttf_parser::name_id::*;
    let count = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    (0..count)
        .map(|index| {
            let face = ttf_parser::Face::parse(data, index).map_err(|e| e.to_string())?;
            let name = |ids: &[u16]| {
                ids.iter()
                    .find_map(|id| face.names().into_iter().filter(|n| n.name_id == *id).find_map(|n| n.to_string()))
                    .unwrap_or_default()
            };
            Ok(Face {
                family: name(&[TYPOGRAPHIC_FAMILY, FAMILY]),
                style: name(&[TYPOGRAPHIC_SUBFAMILY, SUBFAMILY]),
                postscript: name(&[POST_SCRIPT_NAME]),
            })
        })
        .collect()
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0 && entry.file_name().to_string_lossy().starts_with('.')
}

fn is_rightfont_library(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_dir() && entry.file_name().to_string_lossy().ends_with(".rightfontlibrary")
}

fn has_font_extension(path: &Path) -> bool {
    path.extension().is_some_and(|e| FONT_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str()))
}

/// Scans `root` for font files and reads their names.
///
/// Skips hidden files and folders (names that start with `.`) and `*.rightfontlibrary` folders.
pub fn scan(root: &Path) -> std::io::Result<Library> {
    let mut ids = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_entry(|e| !is_hidden(e) && !is_rightfont_library(e)) {
        let entry = entry.map_err(std::io::Error::from)?;
        if entry.file_type().is_file() && has_font_extension(entry.path()) {
            let rel = entry.path().strip_prefix(root).expect("walkdir yields paths under root");
            ids.push(rel.to_string_lossy().into_owned());
        }
    }
    ids.sort();

    let fonts = ids
        .into_par_iter()
        .map(|id| match fs::read(root.join(&id)).map_err(|e| e.to_string()).and_then(|d| read_faces(&d)) {
            Ok(faces) => FontFile { id, faces, error: None },
            Err(error) => FontFile { id, faces: vec![], error: Some(error) },
        })
        .collect();
    Ok(Library { root: root.to_path_buf(), fonts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    fn ids(library: &Library) -> Vec<&str> {
        library.fonts.iter().map(|f| f.id.as_str()).collect()
    }

    #[test]
    fn reads_the_names_of_a_font() {
        let faces =
            read_faces(&font_bytes(FaceNames { family: "Inter", style: "Bold", postscript: "Inter-Bold" })).unwrap();
        assert_eq!(faces, vec![Face { family: "Inter".into(), style: "Bold".into(), postscript: "Inter-Bold".into() }]);
    }

    #[test]
    fn reads_all_faces_of_a_collection() {
        let faces = read_faces(&collection_bytes(&[
            FaceNames::from_postscript("Helvetica-Regular"),
            FaceNames::from_postscript("Helvetica-Bold"),
        ]))
        .unwrap();
        let names: Vec<_> = faces.iter().map(|f| f.postscript.as_str()).collect();
        assert_eq!(names, ["Helvetica-Regular", "Helvetica-Bold"]);
    }

    #[test]
    fn returns_an_error_for_a_file_that_is_not_a_font() {
        assert!(read_faces(b"not a font").is_err());
    }

    #[test]
    fn finds_font_files_in_subfolders_sorted_by_id() {
        let root = tempfile::tempdir().unwrap();
        write_font(root.path(), "Foundry B/Family/B-Regular.otf", "B-Regular");
        write_font(root.path(), "Foundry A/A-Regular.ttf", "A-Regular");
        write_font(root.path(), "Foundry A/Deep/Er/A-Bold.OTF", "A-Bold");
        write_collection(root.path(), "Foundry C/C.ttc", &["C-Regular", "C-Bold"]);
        write_collection(root.path(), "Foundry C/D.otc", &["D-Regular"]);

        let library = scan(root.path()).unwrap();

        assert_eq!(
            ids(&library),
            [
                "Foundry A/A-Regular.ttf",
                "Foundry A/Deep/Er/A-Bold.OTF",
                "Foundry B/Family/B-Regular.otf",
                "Foundry C/C.ttc",
                "Foundry C/D.otc",
            ]
        );
        assert_eq!(library.get("Foundry C/C.ttc").unwrap().faces.len(), 2);
    }

    #[test]
    fn skips_other_files_hidden_files_and_rightfont_libraries() {
        let root = tempfile::tempdir().unwrap();
        write_font(root.path(), "Foundry/Keep.otf", "Keep-Regular");
        write_font(root.path(), "Foundry/Web.woff", "Web-Regular");
        write_font(root.path(), "Foundry/Web.woff2", "Web2-Regular");
        write_file(root.path(), "Foundry/Fonts.zip", b"zip");
        write_font(root.path(), "Foundry/._Keep.otf", "AppleDouble");
        write_font(root.path(), ".sorts/Hidden.otf", "Hidden-Regular");
        write_font(root.path(), "Team.rightfontlibrary/fonts/A/A.otf", "Library-Regular");

        let library = scan(root.path()).unwrap();

        assert_eq!(ids(&library), ["Foundry/Keep.otf"]);
    }

    #[test]
    fn keeps_unreadable_files_with_an_error() {
        let root = tempfile::tempdir().unwrap();
        write_file(root.path(), "Foundry/Broken.ttf", b"not a font");

        let library = scan(root.path()).unwrap();

        let font = library.get("Foundry/Broken.ttf").unwrap();
        assert!(font.faces.is_empty());
        assert!(font.error.is_some());
    }

    #[test]
    fn extension_is_lowercase() {
        let font = FontFile { id: "A/B.OTF".into(), faces: vec![], error: None };
        assert_eq!(font.extension(), "otf");
    }
}
