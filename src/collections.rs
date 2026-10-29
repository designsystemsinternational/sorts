//! Collections: one JSON file per collection in `<root>/.sorts/collections/`.

use crate::settings::write_json;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    /// Fixed ID. Never changes, also not on rename.
    pub id: String,
    pub name: String,
    /// Folder entries and font entries (see `entry`).
    #[serde(default)]
    pub fonts: Vec<String>,
    /// The file name in the collections folder, e.g. `studio-work.json`. Not stored in the file.
    #[serde(skip)]
    pub file: String,
}

#[derive(Debug, Clone, Default)]
pub struct Collections {
    /// Sorted by file name.
    pub items: Vec<Collection>,
    /// Files that could not be read, and duplicate IDs.
    pub problems: Vec<String>,
}

impl Collections {
    pub fn get(&self, id: &str) -> Option<&Collection> {
        self.items.iter().find(|c| c.id == id)
    }
}

pub fn dir(root: &Path) -> PathBuf {
    root.join(".sorts").join("collections")
}

/// Reads all collection files. A missing folder means no collections.
///
/// If two files have the same `id`, the first file in alphabetical order wins.
pub fn load(root: &Path) -> Collections {
    let mut collections = Collections::default();
    let Ok(entries) = fs::read_dir(dir(root)) else { return collections };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter(|p| !p.file_name().unwrap_or_default().to_string_lossy().starts_with('.'))
        .collect();
    files.sort();

    for path in files {
        let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let parsed = fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|data| serde_json::from_slice::<Collection>(&data).map_err(|e| e.to_string()));
        match parsed {
            Err(error) => collections.problems.push(format!("Cannot read collection file {file}: {error}")),
            Ok(c) if collections.get(&c.id).is_some() => collections.problems.push(format!(
                "Collection file {file} has the same id as another collection ({}). It is ignored.",
                c.id
            )),
            Ok(c) => collections.items.push(Collection { file, ..c }),
        }
    }
    collections
}

/// The file name stem for a collection name: lowercase ASCII letters and digits, with one `-`
/// for each run of other characters. Accents are removed. Empty if the name has no letters or digits.
pub fn slug(name: &str) -> String {
    let ascii = deunicode::deunicode(name).to_lowercase();
    ascii.split(|c: char| !c.is_ascii_alphanumeric()).filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-")
}

/// The file name for a new name, or an error if the name has no letters or digits.
fn file_for(name: &str) -> anyhow::Result<String> {
    let slug = slug(name);
    anyhow::ensure!(!slug.is_empty(), "The collection name must contain letters or digits.");
    Ok(format!("{slug}.json"))
}

/// Creates a new collection file with a new fixed ID and no fonts.
/// Fails if the name gives an empty slug or if a file with the same slug exists.
pub fn create(root: &Path, name: &str) -> anyhow::Result<Collection> {
    let file = file_for(name)?;
    anyhow::ensure!(!dir(root).join(&file).exists(), "A collection file named {file} already exists.");
    let collection =
        Collection { id: uuid::Uuid::new_v4().to_string(), name: name.trim().to_string(), fonts: vec![], file };
    save(root, &collection)?;
    Ok(collection)
}

/// Renames a collection: writes the new name and moves the file to the new slug. Keeps the ID.
/// Fails if another file with the new slug exists.
pub fn rename(root: &Path, collection: &Collection, name: &str) -> anyhow::Result<Collection> {
    let file = file_for(name)?;
    let moves = file != collection.file;
    anyhow::ensure!(!moves || !dir(root).join(&file).exists(), "A collection file named {file} already exists.");
    let renamed = Collection { name: name.trim().to_string(), file, ..collection.clone() };
    save(root, &renamed)?;
    if moves {
        delete(root, collection)?;
    }
    Ok(renamed)
}

/// Writes the collection to its file.
pub fn save(root: &Path, collection: &Collection) -> anyhow::Result<()> {
    write_json(&dir(root).join(&collection.file), collection)
}

pub fn delete(root: &Path, collection: &Collection) -> anyhow::Result<()> {
    let path = dir(root).join(&collection.file);
    fs::remove_file(&path).with_context(|| format!("Cannot delete {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::write_file;

    fn write(root: &Path, file: &str, json: &str) {
        write_file(root, &format!(".sorts/collections/{file}"), json.as_bytes());
    }

    #[test]
    fn collections_live_in_a_hidden_sorts_folder() {
        assert_eq!(dir(Path::new("/fonts")), Path::new("/fonts/.sorts/collections"));
    }

    #[test]
    fn no_folder_means_no_collections() {
        let root = tempfile::tempdir().unwrap();
        let collections = load(root.path());
        assert!(collections.items.is_empty());
        assert!(collections.problems.is_empty());
    }

    #[test]
    fn reads_collection_files_sorted_by_file_name() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "studio-work.json", r#"{"id": "r1", "name": "Studio Work", "fonts": ["A/", "B/One.otf"]}"#);
        write(root.path(), "brand-x.json", r#"{"id": "b1", "name": "Brand X"}"#);

        let collections = load(root.path());

        assert_eq!(
            collections.items,
            vec![
                Collection { id: "b1".into(), name: "Brand X".into(), fonts: vec![], file: "brand-x.json".into() },
                Collection {
                    id: "r1".into(),
                    name: "Studio Work".into(),
                    fonts: vec!["A/".into(), "B/One.otf".into()],
                    file: "studio-work.json".into(),
                },
            ]
        );
        assert_eq!(collections.get("r1").unwrap().name, "Studio Work");
    }

    #[test]
    fn ignores_files_that_are_not_json_and_hidden_files() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "notes.txt", "hello");
        write(root.path(), ".hidden.json", r#"{"id": "h", "name": "Hidden"}"#);

        let collections = load(root.path());

        assert!(collections.items.is_empty());
        assert!(collections.problems.is_empty());
    }

    #[test]
    fn reports_invalid_files() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "broken.json", "{ nope");
        write(root.path(), "ok.json", r#"{"id": "ok", "name": "OK"}"#);

        let collections = load(root.path());

        assert_eq!(collections.items.len(), 1);
        assert_eq!(collections.problems.len(), 1);
        assert!(collections.problems[0].contains("broken.json"));
    }

    #[test]
    fn duplicate_ids_keep_the_first_file_and_report_a_problem() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "a.json", r#"{"id": "same", "name": "A"}"#);
        write(root.path(), "b.json", r#"{"id": "same", "name": "B"}"#);

        let collections = load(root.path());

        assert_eq!(collections.items.len(), 1);
        assert_eq!(collections.items[0].name, "A");
        assert_eq!(collections.problems.len(), 1);
        assert!(collections.problems[0].contains("b.json"));
    }

    fn files(root: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(dir(root)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn slug_lowercases_and_joins_words_with_dashes() {
        assert_eq!(slug("Studio Work"), "studio-work");
        assert_eq!(slug("  --Hello__World--  "), "hello-world");
        assert_eq!(slug("Sans Display 2026"), "sans-display-2026");
    }

    #[test]
    fn slug_removes_accents_and_symbols() {
        assert_eq!(slug("Söhne & Café"), "sohne-cafe");
        assert_eq!(slug("PP®F Fonts"), "pp-r-f-fonts");
        assert_eq!(slug("!!!"), "");
    }

    #[test]
    fn create_writes_a_file_with_a_new_id() {
        let root = tempfile::tempdir().unwrap();

        let a = create(root.path(), "  Studio Work ").unwrap();
        let b = create(root.path(), "Brand").unwrap();

        assert_eq!(a.name, "Studio Work");
        assert_eq!(a.file, "studio-work.json");
        assert!(a.fonts.is_empty());
        assert_eq!(a.id.len(), 36, "a UUID: {}", a.id);
        assert_ne!(a.id, b.id);
        assert_eq!(load(root.path()).items, vec![b, a]);
    }

    #[test]
    fn create_refuses_an_existing_slug_or_an_empty_name() {
        let root = tempfile::tempdir().unwrap();
        create(root.path(), "Brand X").unwrap();

        let error = create(root.path(), "brand x").unwrap_err().to_string();
        assert!(error.contains("brand-x.json"), "{error}");
        assert!(create(root.path(), "  ").is_err());
        assert!(create(root.path(), "!!!").is_err());
        assert_eq!(files(root.path()), ["brand-x.json"]);
    }

    #[test]
    fn rename_moves_the_file_and_keeps_the_id_and_fonts() {
        let root = tempfile::tempdir().unwrap();
        let mut collection = create(root.path(), "Brand").unwrap();
        collection.fonts = vec!["A/".into()];
        save(root.path(), &collection).unwrap();

        let renamed = rename(root.path(), &collection, "Studio Work").unwrap();

        assert_eq!(renamed.id, collection.id);
        assert_eq!(renamed.fonts, ["A/"]);
        assert_eq!(renamed.file, "studio-work.json");
        assert_eq!(files(root.path()), ["studio-work.json"]);
        assert_eq!(load(root.path()).items, vec![renamed]);
    }

    #[test]
    fn rename_to_the_same_slug_keeps_the_file() {
        let root = tempfile::tempdir().unwrap();
        let collection = create(root.path(), "brand").unwrap();

        let renamed = rename(root.path(), &collection, "Brand").unwrap();

        assert_eq!(files(root.path()), ["brand.json"]);
        assert_eq!(load(root.path()).items[0].name, "Brand");
        assert_eq!(renamed.file, "brand.json");
    }

    #[test]
    fn rename_refuses_a_slug_that_another_file_uses() {
        let root = tempfile::tempdir().unwrap();
        let collection = create(root.path(), "Brand").unwrap();
        create(root.path(), "Other").unwrap();

        assert!(rename(root.path(), &collection, "other").is_err());
        assert!(rename(root.path(), &collection, "").is_err());
        assert_eq!(files(root.path()), ["brand.json", "other.json"]);
    }

    #[test]
    fn save_keeps_unknown_fields_out_and_writes_the_fonts() {
        let root = tempfile::tempdir().unwrap();
        let mut collection = create(root.path(), "Brand").unwrap();
        collection.fonts = vec!["A/".into(), "B/One.otf".into()];

        save(root.path(), &collection).unwrap();

        let json = fs::read_to_string(dir(root.path()).join("brand.json")).unwrap();
        assert!(!json.contains("\"file\""), "{json}");
        assert_eq!(load(root.path()).items, vec![collection]);
    }

    #[test]
    fn delete_removes_the_file() {
        let root = tempfile::tempdir().unwrap();
        let collection = create(root.path(), "Brand").unwrap();

        delete(root.path(), &collection).unwrap();

        assert!(files(root.path()).is_empty());
    }
}
