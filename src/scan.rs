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
    /// The number of files that were online only before the scan, so the scan downloaded them.
    pub downloaded: usize,
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

/// Env var for end-to-end tests: font IDs that contain this text count as online only.
pub const FAKE_DATALESS_ENV: &str = "SORTS_FAKE_DATALESS";

/// `SF_DATALESS` from `<sys/stat.h>`: the file is a placeholder, and its contents are only online.
/// A read downloads the file (for example from Google Drive or Dropbox) before it returns.
const SF_DATALESS: u32 = 0x4000_0000;

fn has_dataless_flag(flags: u32) -> bool {
    flags & SF_DATALESS != 0
}

/// True if the file is online only. Reading the metadata does not download the file.
pub fn is_dataless(_id: &str, metadata: &fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    has_dataless_flag(metadata.st_flags())
}

/// What the scan does, for a progress display.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScanEvent {
    /// The walk found this many font files so far.
    Found(usize),
    /// The walk is complete, and this many online-only files will now be downloaded.
    Downloading { files: usize, bytes: u64 },
    /// One online-only file was downloaded and read. Sent from several threads.
    Downloaded { bytes: u64 },
}

pub struct ScanOptions<'a> {
    pub is_dataless: &'a (dyn Fn(&str, &fs::Metadata) -> bool + Sync),
    pub progress: &'a (dyn Fn(ScanEvent) + Sync),
}

impl Default for ScanOptions<'_> {
    fn default() -> Self {
        Self { is_dataless: &is_dataless, progress: &|_| {} }
    }
}

/// Scans `root` for font files and reads their names.
///
/// Skips hidden files and folders (names that start with `.`) and `*.rightfontlibrary` folders.
pub fn scan(root: &Path) -> std::io::Result<Library> {
    scan_with(root, &ScanOptions::default())
}

/// Like [`scan`], with a progress callback. Online-only files are downloaded by the read.
pub fn scan_with(root: &Path, options: &ScanOptions) -> std::io::Result<Library> {
    // (id, size in bytes if the file is online only)
    let mut files: Vec<(String, Option<u64>)> = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_entry(|e| !is_hidden(e) && !is_rightfont_library(e)) {
        let entry = entry.map_err(std::io::Error::from)?;
        if entry.file_type().is_file() && has_font_extension(entry.path()) {
            let rel = entry.path().strip_prefix(root).expect("walkdir yields paths under root");
            let id = rel.to_string_lossy().into_owned();
            let metadata = entry.metadata().map_err(std::io::Error::from)?;
            let online_only = (options.is_dataless)(&id, &metadata).then_some(metadata.len());
            files.push((id, online_only));
            (options.progress)(ScanEvent::Found(files.len()));
        }
    }
    files.sort();

    let downloads: Vec<u64> = files.iter().filter_map(|(_, size)| *size).collect();
    if !downloads.is_empty() {
        (options.progress)(ScanEvent::Downloading { files: downloads.len(), bytes: downloads.iter().sum() });
    }

    let fonts = files
        .into_par_iter()
        .map(|(id, online_only)| {
            let result = fs::read(root.join(&id)).map_err(|e| e.to_string());
            if let Some(bytes) = online_only {
                (options.progress)(ScanEvent::Downloaded { bytes });
            }
            match result.and_then(|d| read_faces(&d)) {
                Ok(faces) => FontFile { id, faces, error: None },
                Err(error) => FontFile { id, faces: vec![], error: Some(error) },
            }
        })
        .collect();
    Ok(Library { root: root.to_path_buf(), fonts, downloaded: downloads.len() })
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

    fn scan_events(root: &Path, online_only: &str) -> (Library, Vec<ScanEvent>) {
        let events = std::sync::Mutex::new(Vec::new());
        let is_dataless = |id: &str, _: &fs::Metadata| id.contains(online_only);
        let progress = |event| events.lock().unwrap().push(event);
        let library = scan_with(root, &ScanOptions { is_dataless: &is_dataless, progress: &progress }).unwrap();
        (library, events.into_inner().unwrap())
    }

    #[test]
    fn reports_the_found_files_and_the_downloads() {
        let root = tempfile::tempdir().unwrap();
        write_font(root.path(), "A/Local.otf", "Local-Regular");
        write_font(root.path(), "B/Cloud-One.otf", "Cloud-One");
        write_font(root.path(), "B/Cloud-Two.otf", "Cloud-Two");
        let size = |id: &str| fs::metadata(root.path().join(id)).unwrap().len();
        let (one, two) = (size("B/Cloud-One.otf"), size("B/Cloud-Two.otf"));

        let (library, events) = scan_events(root.path(), "Cloud");

        assert_eq!(events[..3], [ScanEvent::Found(1), ScanEvent::Found(2), ScanEvent::Found(3)]);
        assert_eq!(events[3], ScanEvent::Downloading { files: 2, bytes: one + two });
        // The files are read in parallel, so the downloads can end in any order.
        let mut downloaded = events[4..].to_vec();
        downloaded.sort();
        let mut expected = vec![ScanEvent::Downloaded { bytes: one }, ScanEvent::Downloaded { bytes: two }];
        expected.sort();
        assert_eq!(downloaded, expected);
        assert_eq!(library.downloaded, 2);
        assert_eq!(library.get("B/Cloud-One.otf").unwrap().faces[0].postscript, "Cloud-One");
    }

    #[test]
    fn reports_no_downloads_when_all_files_are_on_disk() {
        let root = tempfile::tempdir().unwrap();
        write_font(root.path(), "A/Local.otf", "Local-Regular");

        let (library, events) = scan_events(root.path(), "Cloud");

        assert_eq!(events, [ScanEvent::Found(1)]);
        assert_eq!(library.downloaded, 0);
    }

    #[test]
    fn the_dataless_flag_marks_online_only_files() {
        assert!(has_dataless_flag(0x4000_0000));
        assert!(has_dataless_flag(0x4000_0020));
        assert!(!has_dataless_flag(0x20));
        let root = tempfile::tempdir().unwrap();
        write_font(root.path(), "A.otf", "A");
        assert!(!is_dataless("A.otf", &fs::metadata(root.path().join("A.otf")).unwrap()));
    }

    #[test]
    fn extension_is_lowercase() {
        let font = FontFile { id: "A/B.OTF".into(), faces: vec![], error: None };
        assert_eq!(font.extension(), "otf");
    }
}
