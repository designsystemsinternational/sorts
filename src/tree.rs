//! The folder tree of the library, used by the interactive list.

use crate::entry;
use crate::scan::Library;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

pub const ROOT: usize = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    /// The last path component.
    pub name: String,
    /// The entry for this node: `A/B/` for a folder, the font ID for a file. Empty for the root.
    pub path: String,
    pub parent: Option<usize>,
    /// Folders first, then files, each sorted by name.
    pub children: Vec<usize>,
    /// Indices into `Library::fonts` of all fonts in this node. Contiguous because the
    /// library is sorted by ID and all IDs in a folder share the folder's prefix.
    pub fonts: Range<usize>,
}

#[derive(Debug, Clone)]
pub struct Tree {
    /// `nodes[ROOT]` is the root folder.
    pub nodes: Vec<Node>,
}

/// A row in the list: a node and its depth (0 = top level).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub node: usize,
    pub depth: usize,
}

impl Tree {
    pub fn build(library: &Library) -> Self {
        let mut nodes = vec![Node {
            kind: Kind::Folder,
            name: String::new(),
            path: String::new(),
            parent: None,
            children: vec![],
            fonts: 0..library.fonts.len(),
        }];
        let mut folders: HashMap<String, usize> = HashMap::from([(String::new(), ROOT)]);

        for (index, font) in library.fonts.iter().enumerate() {
            let (dirs, file) = font.id.rsplit_once('/').unwrap_or(("", &font.id));
            let mut parent = ROOT;
            let mut path = String::new();
            for name in dirs.split('/').filter(|d| !d.is_empty()) {
                path.push_str(name);
                path.push('/');
                parent = match folders.get(&path) {
                    Some(&folder) => {
                        nodes[folder].fonts.end = index + 1;
                        folder
                    }
                    None => {
                        let folder = push(&mut nodes, Kind::Folder, name, path.clone(), parent, index);
                        folders.insert(path.clone(), folder);
                        folder
                    }
                };
            }
            push(&mut nodes, Kind::File, file, font.id.clone(), parent, index);
        }

        let keys: Vec<(bool, String)> = nodes.iter().map(|n| (n.kind == Kind::File, n.name.to_lowercase())).collect();
        for node in &mut nodes {
            node.children.sort_by(|a, b| keys[*a].cmp(&keys[*b]));
        }
        Self { nodes }
    }

    /// The rows to show. Folders show their children when they are in `expanded`.
    ///
    /// With a search query, only nodes that match and their parent folders are shown, and
    /// those parent folders are expanded. A folder that matches itself is not expanded
    /// automatically, and shows all its children when the user expands it.
    ///
    /// A folder matches on its path. A file matches on its ID, family names, and PostScript names.
    pub fn rows(&self, library: &Library, expanded: &HashSet<usize>, query: &str) -> Vec<Row> {
        let filter = (!query.trim().is_empty()).then(|| self.matches(library, query));
        let mut rows = Vec::new();
        self.walk(ROOT, 0, expanded, filter.as_ref(), true, &mut rows);
        rows
    }

    /// Per node: (the node matches the query, a node below it matches the query).
    fn matches(&self, library: &Library, query: &str) -> Vec<(bool, bool)> {
        let mut result: Vec<(bool, bool)> = self
            .nodes
            .iter()
            .map(|node| {
                let text = match node.kind {
                    Kind::Folder => node.path.clone(),
                    Kind::File => {
                        let font = &library.fonts[node.fonts.start];
                        let names = font.faces.iter().map(|f| format!("{} {}", f.family, f.postscript));
                        std::iter::once(font.id.clone()).chain(names).collect::<Vec<_>>().join(" ")
                    }
                };
                (node.parent.is_some() && matches_query(&text, query), false)
            })
            .collect();
        // Children always come after their parent, so a reverse pass sees children first.
        for index in (1..self.nodes.len()).rev() {
            let (own, below) = result[index];
            if own || below {
                result[self.nodes[index].parent.expect("only the root has no parent")].1 = true;
            }
        }
        result
    }

    fn walk(
        &self,
        node: usize,
        depth: usize,
        expanded: &HashSet<usize>,
        filter: Option<&Vec<(bool, bool)>>,
        filtered: bool,
        rows: &mut Vec<Row>,
    ) {
        for &child in &self.nodes[node].children {
            let (own, below) = filter.map_or((true, false), |m| m[child]);
            if filtered && !own && !below {
                continue;
            }
            rows.push(Row { node: child, depth });
            if self.nodes[child].kind == Kind::Folder {
                if filter.is_some() && filtered && !own {
                    self.walk(child, depth + 1, expanded, filter, true, rows);
                } else if expanded.contains(&child) {
                    self.walk(child, depth + 1, expanded, filter, false, rows);
                }
            }
        }
    }

    /// The new `enabled` entries after enabling or disabling all fonts in `node`.
    ///
    /// The entries are rebuilt from the set of enabled fonts: a folder whose fonts are all
    /// enabled becomes one folder entry, so new fonts in that folder are included later.
    /// Entries that match no font in the library are kept as they are.
    pub fn set_enabled(&self, library: &Library, entries: &[String], node: usize, on: bool) -> Vec<String> {
        let mut enabled: Vec<bool> = library.fonts.iter().map(|f| entry::matches_any(entries, &f.id)).collect();
        for index in self.nodes[node].fonts.clone() {
            enabled[index] = on;
        }
        let mut result = Vec::new();
        self.collect_entries(ROOT, &enabled, &mut result);
        result.extend(entries.iter().filter(|e| !entry::exists(e, library)).cloned());
        result
    }

    fn collect_entries(&self, node: usize, enabled: &[bool], result: &mut Vec<String>) {
        for &child in &self.nodes[node].children {
            let child_node = &self.nodes[child];
            if enabled[child_node.fonts.clone()].iter().all(|on| *on) {
                result.push(child_node.path.clone());
            } else if child_node.kind == Kind::Folder {
                self.collect_entries(child, enabled, result);
            }
        }
    }

    /// The node for an entry path, if it exists.
    pub fn find(&self, path: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.path == path)
    }
}

fn push(nodes: &mut Vec<Node>, kind: Kind, name: &str, path: String, parent: usize, font: usize) -> usize {
    let index = nodes.len();
    nodes.push(Node {
        kind,
        name: name.to_string(),
        path,
        parent: Some(parent),
        children: vec![],
        fonts: font..font + 1,
    });
    nodes[parent].children.push(index);
    index
}

/// True if every search term is in the text (case-insensitive).
pub fn matches_query(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query.to_lowercase().split_whitespace().all(|term| text.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{Face, FontFile};

    fn library(ids: &[&str]) -> Library {
        let mut fonts: Vec<FontFile> = ids
            .iter()
            .map(|id| {
                let ps = id.rsplit('/').next().unwrap().trim_end_matches(".otf").to_string();
                FontFile {
                    id: id.to_string(),
                    faces: vec![Face { family: format!("{ps} Family"), style: "Regular".into(), postscript: ps }],
                    error: None,
                }
            })
            .collect();
        fonts.sort_by(|a, b| a.id.cmp(&b.id));
        Library { root: "/fonts".into(), fonts, downloaded: 0 }
    }

    fn sample() -> Library {
        library(&[
            "Foundry A/Sans/Sans-Bold.otf",
            "Foundry A/Sans/Sans-Regular.otf",
            "Foundry A/Serif/Serif-Regular.otf",
            "Foundry A/Readme.otf",
            "Foundry B/Antiqua.otf",
            "Foundry B/Mono/Mono-Regular.otf",
        ])
    }

    fn paths(tree: &Tree, rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| format!("{}{}", "  ".repeat(r.depth), tree.nodes[r.node].name)).collect()
    }

    fn expanded(tree: &Tree, paths: &[&str]) -> HashSet<usize> {
        paths.iter().map(|p| tree.find(p).unwrap()).collect()
    }

    fn entries(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn builds_folders_before_files() {
        let library = sample();
        let tree = Tree::build(&library);

        let foundry = tree.find("Foundry A/").unwrap();
        let children: Vec<_> = tree.nodes[foundry].children.iter().map(|c| tree.nodes[*c].name.as_str()).collect();

        assert_eq!(children, ["Sans", "Serif", "Readme.otf"]);
        assert_eq!(tree.nodes[foundry].kind, Kind::Folder);
        let file = tree.find("Foundry B/Antiqua.otf").unwrap();
        assert_eq!(tree.nodes[file].kind, Kind::File);
        assert_eq!(tree.nodes[file].parent, tree.find("Foundry B/"));
    }

    #[test]
    fn folder_font_ranges_cover_the_whole_subtree() {
        let library = sample();
        let tree = Tree::build(&library);

        let ids = |path: &str| -> Vec<&str> {
            library.fonts[tree.nodes[tree.find(path).unwrap()].fonts.clone()].iter().map(|f| f.id.as_str()).collect()
        };

        assert_eq!(ids("Foundry A/Sans/").len(), 2);
        assert_eq!(ids("Foundry A/").len(), 4);
        assert_eq!(ids("Foundry B/Antiqua.otf"), ["Foundry B/Antiqua.otf"]);
        assert_eq!(tree.nodes[ROOT].fonts, 0..6);
    }

    #[test]
    fn rows_show_top_level_folders_and_expanded_children() {
        let library = sample();
        let tree = Tree::build(&library);

        assert_eq!(paths(&tree, &tree.rows(&library, &HashSet::new(), "")), ["Foundry A", "Foundry B"]);

        let rows = tree.rows(&library, &expanded(&tree, &["Foundry B/"]), "");
        assert_eq!(paths(&tree, &rows), ["Foundry A", "Foundry B", "  Mono", "  Antiqua.otf"]);
    }

    #[test]
    fn search_shows_matches_with_their_folders_expanded() {
        let library = sample();
        let tree = Tree::build(&library);

        let rows = tree.rows(&library, &HashSet::new(), "regular");

        assert_eq!(
            paths(&tree, &rows),
            [
                "Foundry A",
                "  Sans",
                "    Sans-Regular.otf",
                "  Serif",
                "    Serif-Regular.otf",
                "Foundry B",
                "  Mono",
                "    Mono-Regular.otf",
            ]
        );
    }

    #[test]
    fn search_matches_family_and_postscript_names_with_all_terms() {
        let library = sample();
        let tree = Tree::build(&library);

        let rows = tree.rows(&library, &HashSet::new(), "antiqua family");
        assert_eq!(paths(&tree, &rows), ["Foundry B", "  Antiqua.otf"]);

        assert!(tree.rows(&library, &HashSet::new(), "antiqua bold").is_empty());
    }

    #[test]
    fn a_folder_that_matches_shows_all_children_when_expanded() {
        let library = sample();
        let tree = Tree::build(&library);

        // The file in "Serif" matches too, but the matching folder stays collapsed.
        let collapsed = tree.rows(&library, &HashSet::new(), "serif");
        assert_eq!(paths(&tree, &collapsed), ["Foundry A", "  Serif"]);

        let rows = tree.rows(&library, &expanded(&tree, &["Foundry B/"]), "foundry");
        assert_eq!(paths(&tree, &rows), ["Foundry A", "Foundry B", "  Mono", "  Antiqua.otf"]);

        let rows = tree.rows(&library, &HashSet::new(), "foundry");
        assert_eq!(paths(&tree, &rows), ["Foundry A", "Foundry B"]);
    }

    #[test]
    fn enabling_a_folder_gives_one_folder_entry() {
        let library = sample();
        let tree = Tree::build(&library);
        let node = tree.find("Foundry A/Sans/").unwrap();

        assert_eq!(tree.set_enabled(&library, &[], node, true), ["Foundry A/Sans/"]);
    }

    #[test]
    fn disabling_one_file_in_an_enabled_folder_splits_the_folder_entry() {
        let library = sample();
        let tree = Tree::build(&library);
        let node = tree.find("Foundry A/Sans/Sans-Bold.otf").unwrap();

        let result = tree.set_enabled(&library, &entries(&["Foundry A/"]), node, false);

        assert_eq!(result, ["Foundry A/Sans/Sans-Regular.otf", "Foundry A/Serif/", "Foundry A/Readme.otf"]);
    }

    #[test]
    fn enabling_the_last_file_of_a_folder_collapses_to_a_folder_entry() {
        let library = sample();
        let tree = Tree::build(&library);
        let node = tree.find("Foundry A/Sans/Sans-Bold.otf").unwrap();
        let split = entries(&["Foundry A/Sans/Sans-Regular.otf", "Foundry A/Serif/", "Foundry A/Readme.otf"]);

        assert_eq!(tree.set_enabled(&library, &split, node, true), ["Foundry A/"]);
    }

    #[test]
    fn disabling_a_folder_removes_all_entries_inside_it() {
        let library = sample();
        let tree = Tree::build(&library);
        let node = tree.find("Foundry A/").unwrap();
        let current = entries(&["Foundry A/Sans/", "Foundry A/Readme.otf", "Foundry B/Antiqua.otf"]);

        assert_eq!(tree.set_enabled(&library, &current, node, false), ["Foundry B/Antiqua.otf"]);
    }

    #[test]
    fn entries_that_match_no_font_are_kept() {
        let library = sample();
        let tree = Tree::build(&library);
        let node = tree.find("Foundry B/Antiqua.otf").unwrap();

        let result = tree.set_enabled(&library, &entries(&["Gone/", "Old.otf"]), node, true);

        assert_eq!(result, ["Foundry B/Antiqua.otf", "Gone/", "Old.otf"]);
    }

    #[test]
    fn the_root_is_never_one_entry() {
        let library = sample();
        let tree = Tree::build(&library);

        assert_eq!(tree.set_enabled(&library, &[], ROOT, true), ["Foundry A/", "Foundry B/"]);
    }

    #[test]
    fn entry_helpers_agree_with_the_tree() {
        let library = sample();
        let tree = Tree::build(&library);
        for node in &tree.nodes[1..] {
            let count = library.fonts.iter().filter(|f| entry::matches(&node.path, &f.id)).count();
            assert_eq!(count, node.fonts.len(), "{}", node.path);
        }
    }
}
