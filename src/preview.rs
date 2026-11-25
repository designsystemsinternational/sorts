//! The font preview page: the fonts of the current list in the browser, grouped by folder.
//!
//! `page` builds the model (pure), `html` draws it (pure). The page loads each font from the
//! preview server (see `preview_server`) only when its row comes into view.

use crate::active::pick_formats;
use crate::scan::{FontFile, Library};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

pub const DEFAULT_TEXT: &str = "The quick brown fox jumps over the lazy dog";
/// The sample size in pixels when the page opens.
pub const DEFAULT_SIZE: u32 = 48;
pub const MIN_SIZE: u32 = 8;
pub const MAX_SIZE: u32 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    pub sections: Vec<Section>,
    /// The section to scroll to when the page opens.
    pub start: Option<usize>,
}

/// The fonts of one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The folder ID, e.g. `Foundry A/Sans/`. Empty for the top level of the Fonts folder.
    pub folder: String,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// The position of the file in `Page::files`. The server serves the file under this number.
    pub number: usize,
    pub id: String,
    /// The family and style of the first face, or the file name if there are no names.
    pub name: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Font,
    Unreadable,
    /// A `.ttc` or `.otc` file. Browsers often cannot load font collections.
    Collection,
}

impl Page {
    /// The font IDs of all samples, in `Sample::number` order.
    pub fn files(&self) -> Vec<&str> {
        self.samples().map(|s| s.id.as_str()).collect()
    }

    /// The number of samples.
    pub fn len(&self) -> usize {
        self.sections.iter().map(|s| s.samples.len()).sum()
    }

    fn samples(&self) -> impl Iterator<Item = &Sample> {
        self.sections.iter().flat_map(|s| &s.samples)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The preview of `fonts` (indices into `library.fonts`, in the order to show). When several
/// files share a PostScript name, only the file that `format_order` prefers is shown.
/// Consecutive fonts of the same folder form a section. `start` is a folder ID: the page
/// scrolls to the first section in it.
pub fn page(library: &Library, title: &str, fonts: &[usize], format_order: &[String], start: Option<&str>) -> Page {
    let picked: HashSet<&str> = pick_formats(fonts.iter().map(|&i| &library.fonts[i]), format_order)
        .into_iter()
        .map(|f| f.id.as_str())
        .collect();
    let mut sections: Vec<Section> = Vec::new();
    let shown = fonts.iter().map(|&i| &library.fonts[i]).filter(|f| picked.contains(f.id.as_str()));
    for (number, font) in shown.enumerate() {
        let (folder, file) = match font.id.rsplit_once('/') {
            Some((folder, file)) => (format!("{folder}/"), file),
            None => (String::new(), font.id.as_str()),
        };
        let sample = Sample { number, id: font.id.clone(), name: name(font, file), kind: kind(font) };
        match sections.last_mut() {
            Some(section) if section.folder == folder => section.samples.push(sample),
            _ => sections.push(Section { folder, samples: vec![sample] }),
        }
    }
    let start = start.filter(|s| !s.is_empty()).and_then(|s| sections.iter().position(|x| x.folder.starts_with(s)));
    Page { title: title.to_string(), sections, start }
}

fn name(font: &FontFile, file: &str) -> String {
    match font.faces.first() {
        Some(face) if !face.family.is_empty() => format!("{} {}", face.family, face.style).trim().to_string(),
        _ => file.to_string(),
    }
}

fn kind(font: &FontFile) -> Kind {
    if font.error.is_some() {
        Kind::Unreadable
    } else if matches!(font.extension().as_str(), "ttc" | "otc") {
        Kind::Collection
    } else {
        Kind::Font
    }
}

fn escape(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#39;"),
            c => result.push(c),
        }
    }
    result
}

/// The HTML of the page. Font URLs are relative (`font/<number>`), so the page URL must end with `/`.
pub fn html(page: &Page) -> String {
    use std::fmt::Write;
    let count = match page.len() {
        1 => "1 font".to_string(),
        n => format!("{n} fonts"),
    };
    let mut body = String::new();
    for (index, section) in page.sections.iter().enumerate() {
        let folder = match section.folder.trim_end_matches('/') {
            "" => "Top level of the Fonts folder",
            folder => folder,
        };
        let _ = writeln!(body, "<section id=\"s{index}\">\n<h2>{}</h2>", escape(folder));
        for sample in &section.samples {
            let file = sample.id.rsplit('/').next().unwrap_or(&sample.id);
            let label = format!(
                "<div class=\"label\"><span class=\"name\">{}</span><span class=\"file\">{}</span></div>",
                escape(&sample.name),
                escape(file)
            );
            let _ = match sample.kind {
                Kind::Font => writeln!(
                    body,
                    "<div class=\"font\" data-n=\"{}\">{label}<div class=\"sample\">{}</div></div>",
                    sample.number,
                    escape(DEFAULT_TEXT)
                ),
                Kind::Unreadable => {
                    writeln!(body, "<div class=\"font\">{label}<div class=\"note\">Unreadable file.</div></div>")
                }
                Kind::Collection => writeln!(
                    body,
                    "<div class=\"font\">{label}<div class=\"note\">No preview for font collections (.ttc, .otc).</div></div>"
                ),
            };
        }
        body.push_str("</section>\n");
    }
    let start = page.start.map_or("null".to_string(), |s| format!("\"s{s}\""));
    TEMPLATE
        .replace("{{title}}", &escape(&page.title))
        .replace("{{count}}", &count)
        .replace("{{text}}", &escape(DEFAULT_TEXT))
        .replace("{{size}}", &DEFAULT_SIZE.to_string())
        .replace("{{min}}", &MIN_SIZE.to_string())
        .replace("{{max}}", &MAX_SIZE.to_string())
        .replace("{{start}}", &start)
        .replace("{{body}}", &body)
}

const TEMPLATE: &str = include_str!("preview.html");

/// Shows a preview page, e.g. in the browser.
pub trait Previewer {
    /// `files` holds the full path of each file in `Page::files` order.
    fn show(&mut self, page: &Page, files: Vec<PathBuf>) -> anyhow::Result<()>;
}

/// Keeps the pages in memory, for tests.
#[derive(Default, Clone)]
pub struct FakePreviewer {
    pub pages: Rc<RefCell<Vec<Page>>>,
}

impl Previewer for FakePreviewer {
    fn show(&mut self, page: &Page, _files: Vec<PathBuf>) -> anyhow::Result<()> {
        self.pages.borrow_mut().push(page.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::Face;

    fn font(id: &str, family: &str, style: &str, postscript: &str) -> FontFile {
        FontFile {
            id: id.into(),
            faces: vec![Face { family: family.into(), style: style.into(), postscript: postscript.into() }],
            facets: vec![],
            error: None,
        }
    }

    fn library(fonts: Vec<FontFile>) -> Library {
        Library { root: "/fonts".into(), fonts, downloaded: 0 }
    }

    fn order() -> Vec<String> {
        vec!["otf".into(), "ttf".into(), "ttc".into(), "otc".into()]
    }

    fn all(library: &Library) -> Vec<usize> {
        (0..library.fonts.len()).collect()
    }

    fn ids(section: &Section) -> Vec<&str> {
        section.samples.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn consecutive_fonts_of_the_same_folder_form_a_section() {
        let library = library(vec![
            font("A/Sans/Sans-Bold.otf", "Sans", "Bold", "Sans-Bold"),
            font("A/Sans/Sans-Regular.otf", "Sans", "Regular", "Sans-Regular"),
            font("A/Serif.otf", "Serif", "Regular", "Serif-Regular"),
            font("Top.otf", "Top", "Regular", "Top-Regular"),
        ]);
        let page = page(&library, "All fonts", &all(&library), &order(), None);
        let folders: Vec<&str> = page.sections.iter().map(|s| s.folder.as_str()).collect();
        assert_eq!(folders, ["A/Sans/", "A/", ""]);
        assert_eq!(ids(&page.sections[0]), ["A/Sans/Sans-Bold.otf", "A/Sans/Sans-Regular.otf"]);
        assert_eq!(page.sections[0].samples[0].name, "Sans Bold");
        assert_eq!(page.len(), 4);
    }

    #[test]
    fn the_order_of_the_fonts_is_kept() {
        let library =
            library(vec![font("A/a.otf", "Alpha", "Regular", "Alpha"), font("A/b.otf", "Beta", "Regular", "Beta")]);
        let page = page(&library, "t", &[1, 0], &order(), None);
        assert_eq!(ids(&page.sections[0]), ["A/b.otf", "A/a.otf"]);
    }

    #[test]
    fn samples_are_numbered_in_page_order() {
        let library =
            library(vec![font("A/a.otf", "Alpha", "Regular", "Alpha"), font("B/b.otf", "Beta", "Regular", "Beta")]);
        let page = page(&library, "t", &all(&library), &order(), None);
        let numbers: Vec<usize> = page.sections.iter().flat_map(|s| s.samples.iter().map(|x| x.number)).collect();
        assert_eq!(numbers, [0, 1]);
        assert_eq!(page.files(), ["A/a.otf", "B/b.otf"]);
    }

    #[test]
    fn files_with_the_same_postscript_name_show_once_in_the_preferred_format() {
        let library = library(vec![
            font("A/Serif-Italic.otf", "Serif", "Italic", "Serif-Italic"),
            font("A/Serif-Italic.ttf", "Serif", "Italic", "Serif-Italic"),
        ]);
        let ttf_first = vec!["ttf".to_string(), "otf".to_string()];
        let page = page(&library, "t", &all(&library), &ttf_first, None);
        assert_eq!(page.files(), ["A/Serif-Italic.ttf"]);
    }

    #[test]
    fn unreadable_files_and_font_collections_have_their_own_kind() {
        let mut broken = font("A/Broken.ttf", "", "", "");
        broken.faces.clear();
        broken.error = Some("bad".into());
        let library = library(vec![
            broken,
            font("A/Family.ttc", "Family", "Regular", "Family-Regular"),
            font("A/Sans.otf", "Sans", "Regular", "Sans-Regular"),
        ]);
        let page = page(&library, "t", &all(&library), &order(), None);
        let kinds: Vec<(&str, Kind)> = page.sections[0].samples.iter().map(|s| (s.name.as_str(), s.kind)).collect();
        assert_eq!(
            kinds,
            [("Broken.ttf", Kind::Unreadable), ("Family Regular", Kind::Collection), ("Sans Regular", Kind::Font)]
        );
    }

    #[test]
    fn start_is_the_first_section_in_the_folder() {
        let library = library(vec![
            font("A/a.otf", "Alpha", "Regular", "Alpha"),
            font("B/Sans/b.otf", "Beta", "Regular", "Beta"),
            font("B/c.otf", "Gamma", "Regular", "Gamma"),
        ]);
        let fonts = all(&library);
        assert_eq!(page(&library, "t", &fonts, &order(), Some("B/")).start, Some(1));
        assert_eq!(page(&library, "t", &fonts, &order(), Some("A/")).start, Some(0));
        assert_eq!(page(&library, "t", &fonts, &order(), Some("C/")).start, None);
        assert_eq!(page(&library, "t", &fonts, &order(), Some("")).start, None);
        assert_eq!(page(&library, "t", &fonts, &order(), None).start, None);
    }

    #[test]
    fn html_escapes_names() {
        let library = library(vec![font("A/<b>&.otf", "<Fam>", "\"Bold\"", "X")]);
        let html = html(&page(&library, "Search: <x>", &all(&library), &order(), None));
        assert!(html.contains("&lt;Fam&gt; &quot;Bold&quot;"));
        assert!(html.contains("&lt;b&gt;&amp;.otf"));
        assert!(html.contains("Search: &lt;x&gt;"));
        assert!(!html.contains("<Fam>"));
    }

    #[test]
    fn html_of_a_page() {
        let mut broken = font("A/Broken.ttf", "", "", "");
        broken.faces.clear();
        broken.error = Some("bad".into());
        let library = library(vec![
            font("A/Sans/Sans-Bold.otf", "Sans", "Bold", "Sans-Bold"),
            broken,
            font("A/Family.ttc", "Family", "Regular", "Family-Regular"),
            font("Top.otf", "Top", "Regular", "Top-Regular"),
        ]);
        let page = page(&library, "All fonts", &all(&library), &order(), Some("A/"));
        insta::assert_snapshot!(html(&page));
    }
}
