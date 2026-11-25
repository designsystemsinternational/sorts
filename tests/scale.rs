//! Scale test: the scan calculates the facets of 50,000 fonts at every load, with no cache.
//!
//! Ignored by default because it writes 50,000 files. Run it in release mode, so the times
//! are the times of the real binary:
//!
//! ```sh
//! cargo test --release --test scale -- --ignored --nocapture
//! ```

use rayon::prelude::*;
use sorts::facets;
use sorts::testing::{FaceNames, FontSpec, spec_bytes, write_file};
use std::time::{Duration, Instant};

const FONTS: usize = 50_000;
const FONTS_PER_FOLDER: usize = 100;

/// The time budget for all facet analyzers on all fonts, on one thread, in release mode.
/// Measured on a real Fonts folder, the analyzers take about 2 µs per font (0.1 s for 50,000).
const ANALYSIS_BUDGET: Duration = Duration::from_secs(1);

/// The time budget for the full scan (walk, read, names, and facets) of 50,000 small files.
const SCAN_BUDGET: Duration = Duration::from_secs(5);

/// A debug build (`cargo test -- --ignored`) is about 15 times slower in the analyzers.
fn budget(release: Duration) -> Duration {
    if cfg!(debug_assertions) { release * 20 } else { release }
}

/// Font `i` of the test set: serif or sans, mono every third, variable every fifth.
fn spec(i: usize, postscript: &str) -> FontSpec<'_> {
    let names = FaceNames::from_postscript(postscript);
    let mut spec = if i.is_multiple_of(2) { FontSpec::serif(names) } else { FontSpec::sans(names) };
    if i.is_multiple_of(3) {
        spec = spec.monospaced();
    }
    if i.is_multiple_of(5) {
        spec = spec.variable();
    }
    spec
}

fn expected_facets(i: usize) -> Vec<&'static str> {
    vec![
        if i.is_multiple_of(2) { "serif" } else { "sans" },
        if i.is_multiple_of(3) { "mono" } else { "proportional" },
        if i.is_multiple_of(5) { "true" } else { "false" },
    ]
}

#[test]
#[ignore = "writes 50,000 files; run with `cargo test --release --test scale -- --ignored`"]
fn the_facets_of_50_000_fonts_are_calculated_at_every_load() {
    let root = tempfile::tempdir().unwrap();
    let fonts: Vec<(String, Vec<u8>)> = (0..FONTS)
        .into_par_iter()
        .map(|i| {
            let id = format!("Foundry {:03}/Family/Font{i:05}-Regular.otf", i / FONTS_PER_FOLDER);
            let bytes = spec_bytes(&spec(i, &format!("Font{i:05}-Regular")));
            write_file(root.path(), &id, &bytes);
            (id, bytes)
        })
        .collect();

    let start = Instant::now();
    for (_, bytes) in &fonts {
        let face = ttf_parser::Face::parse(bytes, 0).unwrap();
        std::hint::black_box(facets::analyze(&face));
    }
    let analysis = start.elapsed();

    let start = Instant::now();
    let library = sorts::scan::scan(root.path()).unwrap();
    let scan = start.elapsed();

    println!("{FONTS} fonts: facets {analysis:?}, full scan {scan:?}");
    assert_eq!(library.fonts.len(), FONTS);
    for font in &library.fonts {
        let i: usize = font.id[font.id.len() - 17..font.id.len() - 12].parse().unwrap();
        assert_eq!(font.facets, expected_facets(i), "{}", font.id);
    }
    assert!(analysis < budget(ANALYSIS_BUDGET), "the facets of {FONTS} fonts took {analysis:?}");
    assert!(scan < budget(SCAN_BUDGET), "the scan of {FONTS} fonts took {scan:?}");
}
