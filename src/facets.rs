//! Facets: properties of a font file that the search can filter on, as `key:value` terms
//! (e.g. `class:serif spacing:mono variable:true`).
//!
//! Each facet has an [`Analyzer`]. The scan runs all analyzers on the first face of each file,
//! every time. Nothing is stored. To add a facet, write an analyzer and add it to [`ANALYZERS`];
//! the search and its error messages use that list.

/// Calculates one facet of a face.
pub trait Analyzer: Sync {
    /// The search key, e.g. `class`.
    fn key(&self) -> &'static str;
    /// All values, in the order that the help text shows them.
    fn values(&self) -> &'static [&'static str];
    /// One of `values()`.
    fn analyze(&self, face: &ttf_parser::Face) -> &'static str;
}

/// All facets, in a fixed order. `FontFile::facets` has one value per analyzer, in this order.
pub const ANALYZERS: &[&dyn Analyzer] = &[&Class, &Spacing, &Variable];

/// The value of each analyzer for a face, in the order of [`ANALYZERS`].
pub fn analyze(face: &ttf_parser::Face) -> Vec<&'static str> {
    ANALYZERS.iter().map(|a| a.analyze(face)).collect()
}

/// The index in [`ANALYZERS`] of the analyzer with this key.
pub fn position(key: &str) -> Option<usize> {
    ANALYZERS.iter().position(|a| a.key() == key)
}

/// `class`: serif, sans, or other.
///
/// The shape of `H` (or `I`) decides first: a serif makes the ink near the baseline or the cap
/// height wider than the stems. If the font has neither letter, the PANOSE serif style in the
/// OS/2 table decides. Fonts that fit neither rule are `other`.
pub struct Class;

/// The ink near the end of a stem must be this many times wider than the stem for a serif.
const SERIF_RATIO: f32 = 1.35;

/// The heights (as a part of the glyph height) where a serif can be, from the baseline and from
/// the top. Hairline serifs are thin, slab serifs are thick.
const SERIF_HEIGHTS: [f32; 4] = [0.005, 0.015, 0.03, 0.05];

impl Analyzer for Class {
    fn key(&self) -> &'static str {
        "class"
    }

    fn values(&self) -> &'static [&'static str] {
        &["serif", "sans", "other"]
    }

    fn analyze(&self, face: &ttf_parser::Face) -> &'static str {
        [('H', 2), ('I', 1)]
            .into_iter()
            .find_map(|(c, stems)| shape_class(face, c, stems))
            .or_else(|| panose_class(face))
            .unwrap_or("other")
    }
}

/// The class from the stems of one glyph. `None` if the font has no outline for `c`.
fn shape_class(face: &ttf_parser::Face, c: char, stems: usize) -> Option<&'static str> {
    let mut outline = Outline::default();
    let rect = face.outline_glyph(face.glyph_index(c)?, &mut outline)?;
    let (bottom, top) = (f32::from(rect.y_min), f32::from(rect.y_max));
    let height = top - bottom;
    if height <= 0.0 {
        return None;
    }
    let mut ratio: f32 = 0.0;
    for (stem_y, ends) in [(bottom + 0.25 * height, bottom), (top - 0.25 * height, top)] {
        let section = outline.cross_section(stem_y);
        if section.len() != stems {
            return Some("other");
        }
        let stem = ink(&section);
        for part in SERIF_HEIGHTS {
            let y = if ends == bottom { bottom + part * height } else { top - part * height };
            ratio = ratio.max(ink(&outline.cross_section(y)) / stem);
        }
    }
    Some(if ratio >= SERIF_RATIO { "serif" } else { "sans" })
}

/// The class from the PANOSE values in the OS/2 table, if they give one.
fn panose_class(face: &ttf_parser::Face) -> Option<&'static str> {
    let os2 = face.raw_face().table(ttf_parser::Tag::from_bytes(b"OS/2"))?;
    let (family, serif_style) = (*os2.get(32)?, *os2.get(33)?);
    match (family, serif_style) {
        (2, 2..=10) => Some("serif"),
        (2, 11..=15) => Some("sans"),
        (3..=5, _) => Some("other"),
        _ => None,
    }
}

/// The total width of the intervals.
fn ink(section: &[(f32, f32)]) -> f32 {
    section.iter().map(|(start, end)| end - start).sum()
}

/// The outline of a glyph as straight lines. Curves are split into short lines.
#[derive(Default)]
struct Outline {
    lines: Vec<[f32; 4]>,
    start: (f32, f32),
    current: (f32, f32),
}

const CURVE_STEPS: usize = 8;

impl Outline {
    /// The filled intervals on the horizontal line at `y`, sorted, with the non-zero rule (so
    /// overlapping contours count once).
    fn cross_section(&self, y: f32) -> Vec<(f32, f32)> {
        let mut crossings: Vec<(f32, i32)> = self
            .lines
            .iter()
            .filter_map(|&[x0, y0, x1, y1]| {
                let direction = if y0 <= y && y < y1 {
                    1
                } else if y1 <= y && y < y0 {
                    -1
                } else {
                    return None;
                };
                Some((x0 + (y - y0) / (y1 - y0) * (x1 - x0), direction))
            })
            .collect();
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut intervals = Vec::new();
        let mut winding = 0;
        let mut start = 0.0;
        for (x, direction) in crossings {
            if winding == 0 {
                start = x;
            }
            winding += direction;
            if winding == 0 && x > start {
                intervals.push((start, x));
            }
        }
        intervals
    }

    fn push(&mut self, x: f32, y: f32) {
        self.lines.push([self.current.0, self.current.1, x, y]);
        self.current = (x, y);
    }
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = (x, y);
        self.current = (x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x0, y0) = self.current;
        for step in 1..=CURVE_STEPS {
            let t = step as f32 / CURVE_STEPS as f32;
            let u = 1.0 - t;
            self.push(u * u * x0 + 2.0 * u * t * x1 + t * t * x, u * u * y0 + 2.0 * u * t * y1 + t * t * y);
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x0, y0) = self.current;
        for step in 1..=CURVE_STEPS {
            let t = step as f32 / CURVE_STEPS as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.push(a * x0 + b * x1 + c * x2 + d * x, a * y0 + b * y1 + c * y2 + d * y);
        }
    }

    fn close(&mut self) {
        if self.current != self.start {
            let (x, y) = self.start;
            self.push(x, y);
        }
    }
}

/// `spacing`: mono or proportional.
///
/// The advance widths of narrow and wide letters decide. If the font has fewer than two of
/// them, the `isFixedPitch` flag in the `post` table decides.
pub struct Spacing;

impl Analyzer for Spacing {
    fn key(&self) -> &'static str {
        "spacing"
    }

    fn values(&self) -> &'static [&'static str] {
        &["mono", "proportional"]
    }

    fn analyze(&self, face: &ttf_parser::Face) -> &'static str {
        let advances: Vec<u16> = ['i', 'l', 'm', 'M', 'W']
            .into_iter()
            .filter_map(|c| face.glyph_hor_advance(face.glyph_index(c)?))
            .collect();
        let mono = if advances.len() >= 2 { advances.iter().all(|a| *a == advances[0]) } else { face.is_monospaced() };
        if mono { "mono" } else { "proportional" }
    }
}

/// `variable`: true if the font has variation axes (an `fvar` table).
pub struct Variable;

impl Analyzer for Variable {
    fn key(&self) -> &'static str {
        "variable"
    }

    fn values(&self) -> &'static [&'static str] {
        &["true", "false"]
    }

    fn analyze(&self, face: &ttf_parser::Face) -> &'static str {
        if face.is_variable() { "true" } else { "false" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    const NAMES: FaceNames = FaceNames { family: "Test", style: "Regular", postscript: "Test-Regular" };

    /// PANOSE for a Latin text font with this serif style.
    fn panose(serif_style: u8) -> [u8; 10] {
        [2, serif_style, 0, 0, 0, 0, 0, 0, 0, 0]
    }

    fn facet(analyzer: &dyn Analyzer, spec: &FontSpec) -> &'static str {
        let data = spec_bytes(spec);
        analyzer.analyze(&ttf_parser::Face::parse(&data, 0).unwrap())
    }

    #[test]
    fn every_analyzer_has_a_unique_key_and_returns_one_of_its_values() {
        let mut keys: Vec<_> = ANALYZERS.iter().map(|a| a.key()).collect();
        keys.dedup();
        assert_eq!(keys, ["class", "spacing", "variable"]);
        for spec in [FontSpec::new(NAMES), FontSpec::sans(NAMES), FontSpec::serif(NAMES).variable()] {
            for analyzer in ANALYZERS {
                assert!(analyzer.values().contains(&facet(*analyzer, &spec)), "{}", analyzer.key());
            }
        }
    }

    #[test]
    fn class_is_sans_for_plain_stems() {
        assert_eq!(facet(&Class, &FontSpec::sans(NAMES)), "sans");
    }

    #[test]
    fn class_is_serif_for_stems_with_feet() {
        assert_eq!(facet(&Class, &FontSpec::serif(NAMES)), "serif");
    }

    #[test]
    fn class_uses_i_if_there_is_no_h() {
        let glyphs = |source: Vec<Glyph>| source.into_iter().filter(|g| g.char != 'H').collect();
        assert_eq!(facet(&Class, &FontSpec::new(NAMES).glyphs(glyphs(serif_glyphs()))), "serif");
        assert_eq!(facet(&Class, &FontSpec::new(NAMES).glyphs(glyphs(sans_glyphs()))), "sans");
    }

    #[test]
    fn class_is_serif_for_feet_only_at_the_top() {
        let h = Glyph::new('H', 700, &[[100, 0, 180, 700], [520, 0, 600, 700], [40, 670, 660, 700]]);
        assert_eq!(facet(&Class, &FontSpec::new(NAMES).glyphs(vec![h])), "serif");
    }

    #[test]
    fn class_is_other_for_an_h_without_two_stems() {
        // The analyzer counts the stems at a quarter of the height (175).
        let stencil = Glyph::new('H', 700, &[[100, 0, 180, 150], [100, 200, 180, 700], [520, 0, 600, 700]]);
        let blob = Glyph::new('H', 700, &[[100, 0, 600, 700]]);
        assert_eq!(facet(&Class, &FontSpec::new(NAMES).glyphs(vec![stencil])), "other");
        assert_eq!(facet(&Class, &FontSpec::new(NAMES).glyphs(vec![blob])), "other");
    }

    #[test]
    fn the_shape_wins_over_panose() {
        assert_eq!(facet(&Class, &FontSpec::sans(NAMES).panose(panose(2))), "sans");
        assert_eq!(facet(&Class, &FontSpec::serif(NAMES).panose(panose(11))), "serif");
    }

    #[test]
    fn class_uses_panose_without_h_and_i() {
        for style in 2..=10 {
            assert_eq!(facet(&Class, &FontSpec::new(NAMES).panose(panose(style))), "serif", "{style}");
        }
        for style in 11..=15 {
            assert_eq!(facet(&Class, &FontSpec::new(NAMES).panose(panose(style))), "sans", "{style}");
        }
    }

    #[test]
    fn class_is_other_without_glyphs_or_panose() {
        assert_eq!(facet(&Class, &FontSpec::new(NAMES)), "other");
        assert_eq!(facet(&Class, &FontSpec::new(NAMES).panose(panose(1))), "other");
        // Hand written, decorative, and symbol fonts.
        for family in 3..=5 {
            let spec = FontSpec::new(NAMES).panose([family, 11, 0, 0, 0, 0, 0, 0, 0, 0]);
            assert_eq!(facet(&Class, &spec), "other", "{family}");
        }
    }

    #[test]
    fn spacing_is_mono_if_the_letters_have_the_same_advance() {
        assert_eq!(facet(&Spacing, &FontSpec::sans(NAMES).monospaced()), "mono");
        assert_eq!(facet(&Spacing, &FontSpec::sans(NAMES)), "proportional");
    }

    #[test]
    fn spacing_trusts_the_letters_over_the_fixed_pitch_flag() {
        assert_eq!(facet(&Spacing, &FontSpec::sans(NAMES).fixed_pitch()), "proportional");
    }

    #[test]
    fn spacing_uses_the_fixed_pitch_flag_without_letters() {
        assert_eq!(facet(&Spacing, &FontSpec::new(NAMES).fixed_pitch()), "mono");
        assert_eq!(facet(&Spacing, &FontSpec::new(NAMES)), "proportional");
    }

    #[test]
    fn variable_is_true_with_variation_axes() {
        assert_eq!(facet(&Variable, &FontSpec::new(NAMES).variable()), "true");
        assert_eq!(facet(&Variable, &FontSpec::new(NAMES)), "false");
    }

    #[test]
    fn analyze_gives_one_value_per_analyzer() {
        let data = spec_bytes(&FontSpec::serif(NAMES).monospaced().variable());
        assert_eq!(analyze(&ttf_parser::Face::parse(&data, 0).unwrap()), ["serif", "mono", "true"]);
    }
}
