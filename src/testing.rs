//! Test support: generates minimal font files so tests do not need licensed fixtures.
//!
//! The generated fonts are minimal but complete TrueType fonts with one empty glyph.
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

/// A single-face font file (`.otf` / `.ttf`).
pub fn font_bytes(names: FaceNames) -> Vec<u8> {
    sfnt(names, 0)
}

/// A font collection file (`.ttc` / `.otc`) with one face per entry.
pub fn collection_bytes(faces: &[FaceNames]) -> Vec<u8> {
    let header_len = 12 + 4 * faces.len();
    let mut fonts = Vec::new();
    let mut offsets = Vec::new();
    for names in faces {
        let offset = header_len + fonts.len();
        offsets.push(offset as u32);
        fonts.extend(sfnt(*names, offset));
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
fn sfnt(names: FaceNames, base: usize) -> Vec<u8> {
    // Sorted by tag, as the OpenType spec requires.
    let tables: [(&[u8; 4], Vec<u8>); 10] = [
        (b"OS/2", os2()),
        (b"cmap", cmap()),
        (b"glyf", vec![]),
        (b"head", head()),
        (b"hhea", hhea()),
        (b"hmtx", hmtx()),
        (b"loca", loca()),
        (b"maxp", maxp()),
        (b"name", name(names)),
        (b"post", post()),
    ];
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

fn hhea() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(0x0001_0000u32.to_be_bytes()); // version
    t.extend(800i16.to_be_bytes()); // ascender
    t.extend((-200i16).to_be_bytes()); // descender
    t.extend([0u8; 26]); // lineGap .. metricDataFormat
    t.extend(1u16.to_be_bytes()); // numberOfHMetrics
    t
}

fn maxp() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(0x0001_0000u32.to_be_bytes()); // version 1.0 (TrueType)
    t.extend(1u16.to_be_bytes()); // numGlyphs
    t.extend([0u8; 26]); // maxPoints .. maxComponentDepth
    t
}

fn hmtx() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(500u16.to_be_bytes()); // advanceWidth
    t.extend(0i16.to_be_bytes()); // lsb
    t
}

/// Short offsets: glyph 0 is empty.
fn loca() -> Vec<u8> {
    [0u16, 0].iter().flat_map(|o| o.to_be_bytes()).collect()
}

/// A format 4 subtable with only the required final segment (no characters mapped).
fn cmap() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(0u16.to_be_bytes()); // version
    t.extend(1u16.to_be_bytes()); // numTables
    t.extend(3u16.to_be_bytes()); // platform: Windows
    t.extend(1u16.to_be_bytes()); // encoding: Unicode BMP
    t.extend(12u32.to_be_bytes()); // subtable offset
    for value in [4u16, 24, 0, 2, 2, 0, 0, 0xFFFF, 0, 0xFFFF, 1, 0] {
        // format, length, language, segCountX2, searchRange, entrySelector, rangeShift,
        // endCode, reservedPad, startCode, idDelta, idRangeOffset
        t.extend(value.to_be_bytes());
    }
    t
}

fn post() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend(0x0003_0000u32.to_be_bytes()); // version 3: no glyph names
    t.extend([0u8; 28]); // italicAngle .. maxMemType1
    t
}

fn os2() -> Vec<u8> {
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
