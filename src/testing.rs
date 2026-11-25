//! Test support: generates minimal font files so tests do not need licensed fixtures.
//!
//! The generated fonts are minimal but complete TrueType fonts. By default they have one empty
//! glyph. A [`FontSpec`] adds glyphs made of rectangles, PANOSE values, the fixed-pitch flag,
//! and an `fvar` table, so tests can check the facet analyzers.
//! `ttf-parser` can read their names, and CoreText can register them.

use std::fs;
use std::path::{Path, PathBuf};

/// Names of one face in a generated font.
#[derive(Clone, Copy)]
pub struct FaceNames<'a> {
    pub family: &'a str,
    pub style: &'a str,
    pub postscript: &'a str,
}

impl<'a> FaceNames<'a> {
    /// Derives the family and style from a PostScript name like `Inter-Bold`.
    pub fn from_postscript(postscript: &'a str) -> Self {
        let (family, style) = postscript.split_once('-').unwrap_or((postscript, "Regular"));
        Self { family, style, postscript }
    }
}

/// A glyph made of rectangles. Each rectangle is `[x_min, y_min, x_max, y_max]` and becomes one
/// contour. Rectangles can overlap, as in many variable fonts.
#[derive(Clone, Debug)]
pub struct Glyph {
    pub char: char,
    pub advance: u16,
    pub rects: Vec<[i16; 4]>,
}

impl Glyph {
    pub fn new(char: char, advance: u16, rects: &[[i16; 4]]) -> Self {
        Self { char, advance, rects: rects.to_vec() }
    }
}

/// Everything about a generated face. `FontSpec::new(names)` gives the default minimal font.
#[derive(Clone)]
pub struct FontSpec<'a> {
    pub names: FaceNames<'a>,
    pub glyphs: Vec<Glyph>,
    /// The 10 PANOSE bytes in the OS/2 table.
    pub panose: [u8; 10],
    /// `post.isFixedPitch`.
    pub fixed_pitch: bool,
    /// Adds an `fvar` table with one `wght` axis.
    pub variable: bool,
}

impl<'a> FontSpec<'a> {
    pub fn new(names: FaceNames<'a>) -> Self {
        Self { names, glyphs: vec![], panose: [0; 10], fixed_pitch: false, variable: false }
    }

    /// A font with a sans `H` and `I` (plain stems) and proportional `i`, `m`, and `W`.
    pub fn sans(names: FaceNames<'a>) -> Self {
        Self { glyphs: sans_glyphs(), ..Self::new(names) }
    }

    /// A font with a serif `H` and `I` (wide feet and heads) and proportional `i`, `m`, and `W`.
    pub fn serif(names: FaceNames<'a>) -> Self {
        Self { glyphs: serif_glyphs(), ..Self::new(names) }
    }

    pub fn glyphs(mut self, glyphs: Vec<Glyph>) -> Self {
        self.glyphs = glyphs;
        self
    }

    pub fn panose(mut self, panose: [u8; 10]) -> Self {
        self.panose = panose;
        self
    }

    pub fn fixed_pitch(mut self) -> Self {
        self.fixed_pitch = true;
        self
    }

    pub fn variable(mut self) -> Self {
        self.variable = true;
        self
    }

    /// Gives all glyphs the same advance width.
    pub fn monospaced(mut self) -> Self {
        for glyph in &mut self.glyphs {
            glyph.advance = 600;
        }
        self
    }
}

/// Sans glyphs: stems with no serifs. Cap height 700, x-height 500.
pub fn sans_glyphs() -> Vec<Glyph> {
    vec![
        Glyph::new('H', 700, &[[100, 0, 180, 700], [520, 0, 600, 700], [100, 320, 600, 390]]),
        Glyph::new('I', 280, &[[100, 0, 180, 700]]),
        Glyph::new('i', 260, &[[90, 0, 170, 500], [90, 600, 170, 690]]),
        Glyph::new('m', 860, &[[80, 0, 150, 500], [380, 0, 450, 480], [680, 0, 750, 480], [80, 430, 750, 500]]),
        Glyph::new('W', 940, &[[40, 0, 120, 700], [430, 0, 510, 700], [820, 0, 900, 700], [40, 0, 900, 60]]),
    ]
}

/// Serif glyphs: the same stems with feet and heads that overlap the stems.
pub fn serif_glyphs() -> Vec<Glyph> {
    let serifs = |x0: i16, x1: i16| [[x0 - 60, 0, x1 + 60, 30], [x0 - 60, 670, x1 + 60, 700]];
    let [h1, h2] = serifs(100, 180);
    let [h3, h4] = serifs(520, 600);
    let mut glyphs = sans_glyphs();
    glyphs[0].rects.extend([h1, h2, h3, h4]);
    glyphs[1].rects.extend([h1, h2]);
    glyphs
}

/// A single-face font file (`.otf` / `.ttf`).
pub fn font_bytes(names: FaceNames) -> Vec<u8> {
    spec_bytes(&FontSpec::new(names))
}

/// A single-face font file from a full spec.
pub fn spec_bytes(spec: &FontSpec) -> Vec<u8> {
    sfnt(spec, 0)
}

/// Writes a font from a full spec to `root/rel`.
pub fn write_spec(root: &Path, rel: &str, spec: &FontSpec) -> PathBuf {
    write_file(root, rel, &spec_bytes(spec))
}

/// A font collection file (`.ttc` / `.otc`) with one face per entry.
pub fn collection_bytes(faces: &[FaceNames]) -> Vec<u8> {
    let specs: Vec<_> = faces.iter().map(|names| FontSpec::new(*names)).collect();
    collection_spec_bytes(&specs)
}

/// A font collection file with one face per spec.
pub fn collection_spec_bytes(faces: &[FontSpec]) -> Vec<u8> {
    let header_len = 12 + 4 * faces.len();
    let mut fonts = Vec::new();
    let mut offsets = Vec::new();
    for spec in faces {
        let offset = header_len + fonts.len();
        offsets.push(offset as u32);
        fonts.extend(sfnt(spec, offset));
    }
    let mut out = Vec::new();
    out.extend(b"ttcf");
    out.extend(0x0001_0000u32.to_be_bytes());
    out.extend((faces.len() as u32).to_be_bytes());
    for offset in offsets {
        out.extend(offset.to_be_bytes());
    }
    out.extend(fonts);
    out
}

/// Writes a generated font to `root/rel`. The names come from the PostScript name.
pub fn write_font(root: &Path, rel: &str, postscript: &str) -> PathBuf {
    write_file(root, rel, &font_bytes(FaceNames::from_postscript(postscript)))
}

/// Writes a generated font collection to `root/rel`.
pub fn write_collection(root: &Path, rel: &str, postscripts: &[&str]) -> PathBuf {
    let faces: Vec<_> = postscripts.iter().map(|ps| FaceNames::from_postscript(ps)).collect();
    write_file(root, rel, &collection_bytes(&faces))
}

/// Writes any bytes to `root/rel` and creates the parent folders.
pub fn write_file(root: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    path
}

/// Builds one sfnt font. `base` is the offset of the font inside the file (non-zero in a collection).
fn sfnt(spec: &FontSpec, base: usize) -> Vec<u8> {
    let (glyf, loca) = glyf_and_loca(&spec.glyphs);
    let glyphs = 1 + spec.glyphs.len() as u16;
    // Sorted by tag, as the OpenType spec requires.
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2(&spec.panose)),
        (b"cmap", cmap(&spec.glyphs)),
        (b"glyf", glyf),
        (b"head", head()),
        (b"hhea", hhea(glyphs)),
        (b"hmtx", hmtx(&spec.glyphs)),
        (b"loca", loca),
        (b"maxp", maxp(glyphs)),
        (b"name", name(spec.names)),
        (b"post", post(spec.fixed_pitch)),
    ];
    if spec.variable {
        tables.insert(2, (b"fvar", fvar()));
    }
    let mut out = Vec::new();
    out.extend(0x0001_0000u32.to_be_bytes());
    out.extend((tables.len() as u16).to_be_bytes());
    out.extend([0u8; 6]); // searchRange, entrySelector, rangeShift (not checked)

    let mut offset = base + 12 + 16 * tables.len();
    let mut data = Vec::new();
    for (tag, table) in &tables {
        out.extend(*tag);
        out.extend(0u32.to_be_bytes()); // checksum (not checked)
        out.extend((offset as u32).to_be_bytes());
        out.extend((table.len() as u32).to_be_bytes());
        let padded = table.len().div_ceil(4) * 4;
        data.extend(table);
        data.resize(data.len() + padded - table.len(), 0);
        offset += padded;
    }
    out.extend(data);
    out
}

fn head() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(0x0001_0000u32.to_be_bytes()); // version
    t.extend(0x0001_0000u32.to_be_bytes()); // fontRevision
    t.extend(0u32.to_be_bytes()); // checksumAdjustment
    t.extend(0x5F0F_3CF5u32.to_be_bytes()); // magicNumber
    t.extend(0u16.to_be_bytes()); // flags
    t.extend(1000u16.to_be_bytes()); // unitsPerEm
    t.extend([0u8; 16]); // created, modified
    t.extend([0u8; 8]); // xMin, yMin, xMax, yMax
    t.extend([0u8; 6]); // macStyle, lowestRecPPEM, fontDirectionHint
    t.extend(0u16.to_be_bytes()); // indexToLocFormat
    t.extend(0u16.to_be_bytes()); // glyphDataFormat
    t
}

fn hhea(glyphs: u16) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(0x0001_0000u32.to_be_bytes()); // version
    t.extend(800i16.to_be_bytes()); // ascender
    t.extend((-200i16).to_be_bytes()); // descender
    t.extend([0u8; 26]); // lineGap .. metricDataFormat
    t.extend(glyphs.to_be_bytes()); // numberOfHMetrics
    t
}

fn maxp(glyphs: u16) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(0x0001_0000u32.to_be_bytes()); // version 1.0 (TrueType)
    t.extend(glyphs.to_be_bytes()); // numGlyphs
    t.extend([0u8; 26]); // maxPoints .. maxComponentDepth
    t
}

/// Glyph 0 (.notdef) has an advance of 500, then one metric per glyph.
fn hmtx(glyphs: &[Glyph]) -> Vec<u8> {
    let mut t = Vec::new();
    for advance in std::iter::once(500).chain(glyphs.iter().map(|g| g.advance)) {
        t.extend(advance.to_be_bytes()); // advanceWidth
        t.extend(0i16.to_be_bytes()); // lsb
    }
    t
}

/// Short offsets (offset / 2). Glyph 0 is empty.
fn glyf_and_loca(glyphs: &[Glyph]) -> (Vec<u8>, Vec<u8>) {
    let mut glyf = Vec::new();
    let mut offsets = vec![0u16, 0];
    for glyph in glyphs {
        glyf.extend(simple_glyph(&glyph.rects));
        glyf.resize(glyf.len().div_ceil(2) * 2, 0);
        offsets.push((glyf.len() / 2) as u16);
    }
    (glyf, offsets.iter().flat_map(|o| o.to_be_bytes()).collect())
}

/// A simple TrueType glyph with one clockwise contour per rectangle. All points are on the
/// curve, and the coordinates are 16-bit deltas.
fn simple_glyph(rects: &[[i16; 4]]) -> Vec<u8> {
    if rects.is_empty() {
        return vec![];
    }
    let points: Vec<(i16, i16)> =
        rects.iter().flat_map(|&[x0, y0, x1, y1]| [(x0, y0), (x0, y1), (x1, y1), (x1, y0)]).collect();
    let mut t = Vec::new();
    t.extend((rects.len() as i16).to_be_bytes()); // numberOfContours
    for value in [
        rects.iter().map(|r| r[0]).min().unwrap(),
        rects.iter().map(|r| r[1]).min().unwrap(),
        rects.iter().map(|r| r[2]).max().unwrap(),
        rects.iter().map(|r| r[3]).max().unwrap(),
    ] {
        t.extend(value.to_be_bytes());
    }
    for contour in 0..rects.len() {
        t.extend((contour as u16 * 4 + 3).to_be_bytes()); // endPtsOfContours
    }
    t.extend(0u16.to_be_bytes()); // instructionLength
    t.extend(std::iter::repeat_n(0x01u8, points.len())); // flags: on curve
    let mut previous = (0i16, 0i16);
    let mut ys = Vec::new();
    for &(x, y) in &points {
        t.extend((x - previous.0).to_be_bytes());
        ys.extend((y - previous.1).to_be_bytes());
        previous = (x, y);
    }
    t.extend(ys);
    t
}

/// A format 4 subtable with one segment per glyph and the required final segment.
fn cmap(glyphs: &[Glyph]) -> Vec<u8> {
    let mut chars: Vec<(u16, u16)> =
        glyphs.iter().enumerate().map(|(index, g)| (g.char as u16, index as u16 + 1)).collect();
    chars.sort();
    let segments = chars.len() as u16 + 1;
    let mut t = Vec::new();
    t.extend(0u16.to_be_bytes()); // version
    t.extend(1u16.to_be_bytes()); // numTables
    t.extend(3u16.to_be_bytes()); // platform: Windows
    t.extend(1u16.to_be_bytes()); // encoding: Unicode BMP
    t.extend(12u32.to_be_bytes()); // subtable offset
    let ends = chars.iter().map(|(c, _)| *c).chain([0xFFFF]);
    let starts = ends.clone();
    let deltas = chars.iter().map(|(c, glyph)| glyph.wrapping_sub(*c)).chain([1]);
    // format, length, language, segCountX2, searchRange, entrySelector, rangeShift
    for value in [4u16, 16 + 8 * segments, 0, 2 * segments, 2, 0, 0] {
        t.extend(value.to_be_bytes());
    }
    let mut put = |values: &mut dyn Iterator<Item = u16>| values.for_each(|v| t.extend(v.to_be_bytes()));
    put(&mut ends.clone());
    put(&mut std::iter::once(0)); // reservedPad
    put(&mut starts.into_iter());
    put(&mut deltas.into_iter());
    put(&mut std::iter::repeat_n(0, segments as usize)); // idRangeOffset
    t
}

/// One `wght` axis from 100 to 900, no named instances.
fn fvar() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(0x0001_0000u32.to_be_bytes()); // version
    for value in [16u16, 2, 1, 20, 0, 8] {
        // axesArrayOffset, reserved, axisCount, axisSize, instanceCount, instanceSize
        t.extend(value.to_be_bytes());
    }
    t.extend(b"wght");
    for value in [100i32, 400, 900] {
        t.extend((value << 16).to_be_bytes()); // min, default, max (Fixed 16.16)
    }
    t.extend(0u16.to_be_bytes()); // flags
    t.extend(256u16.to_be_bytes()); // axisNameID
    t
}

fn post(fixed_pitch: bool) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(0x0003_0000u32.to_be_bytes()); // version 3: no glyph names
    t.extend([0u8; 8]); // italicAngle, underlinePosition, underlineThickness
    t.extend(u32::from(fixed_pitch).to_be_bytes()); // isFixedPitch
    t.extend([0u8; 16]); // minMemType42 .. maxMemType1
    t
}

fn os2(panose: &[u8; 10]) -> Vec<u8> {
    let mut t = vec![0u8; 96];
    let mut put = |offset: usize, value: u16| t[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    put(0, 4); // version
    put(2, 500); // xAvgCharWidth
    put(4, 400); // usWeightClass
    put(6, 5); // usWidthClass
    put(62, 0x40); // fsSelection: REGULAR
    put(66, 0xFFFF); // usLastCharIndex
    put(68, 800); // sTypoAscender
    put(70, (-200i16) as u16); // sTypoDescender
    put(74, 800); // usWinAscent
    put(76, 200); // usWinDescent
    t[32..42].copy_from_slice(panose);
    t[58..62].copy_from_slice(b"TEST"); // achVendID
    t
}

fn name(names: FaceNames) -> Vec<u8> {
    let records = [(1u16, names.family), (2, names.style), (6, names.postscript)];
    let strings: Vec<Vec<u8>> =
        records.iter().map(|(_, s)| s.encode_utf16().flat_map(u16::to_be_bytes).collect()).collect();
    let mut t = Vec::new();
    t.extend(0u16.to_be_bytes()); // format
    t.extend((records.len() as u16).to_be_bytes());
    t.extend((6 + 12 * records.len() as u16).to_be_bytes()); // stringOffset
    let mut offset = 0u16;
    for ((id, _), bytes) in records.iter().zip(&strings) {
        t.extend(3u16.to_be_bytes()); // platform: Windows
        t.extend(1u16.to_be_bytes()); // encoding: Unicode BMP
        t.extend(0x0409u16.to_be_bytes()); // language: en-US
        t.extend(id.to_be_bytes());
        t.extend((bytes.len() as u16).to_be_bytes());
        t.extend(offset.to_be_bytes());
        offset += bytes.len() as u16;
    }
    for bytes in strings {
        t.extend(bytes);
    }
    t
}
