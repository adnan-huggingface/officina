//! Real type faces, loaded from the system at startup.
//!
//! egui ships one weight of one font. A spreadsheet drawn with it has no bold —
//! and the usual workaround, drawing the glyphs twice half a pixel apart, is
//! visible as smearing at any size above about fourteen points and is simply
//! wrong at forty. Italic is faked the same way, by shearing, which turns a
//! serif face into a slanted serif face rather than into its italic.
//!
//! So the faces are loaded from the operating system's own font directory
//! instead. Nothing is redistributed — these are the files already on the
//! machine, and asking for Arial on a machine that has Arial is the whole point
//! of a document that names Arial. Whatever is missing falls back to egui's
//! built-in face, so a machine with no fonts at all still starts.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use eframe::egui;

/// The three families a workbook's font name is resolved into.
///
/// Not an attempt at a font-matching engine. A spreadsheet names Arial,
/// Calibri, Times New Roman, or Courier New nine times out of ten, and the
/// tenth is a name nobody has installed anyway — for which the answer is the
/// same as Excel's: substitute something of the same shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Sans,
    Serif,
    Mono,
}

impl Family {
    /// Which family a font name belongs to.
    pub fn of(name: &str) -> Family {
        let lower = name.to_ascii_lowercase();
        const SERIF: [&str; 8] = [
            "times",
            "georgia",
            "garamond",
            "book antiqua",
            "palatino",
            "cambria",
            "constantia",
            "serif",
        ];
        const MONO: [&str; 6] = [
            "courier",
            "consolas",
            "menlo",
            "monaco",
            "lucida console",
            "mono",
        ];
        if MONO.iter().any(|m| lower.contains(m)) {
            Family::Mono
        } else if SERIF.iter().any(|s| lower.contains(s)) {
            Family::Serif
        } else {
            Family::Sans
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Family::Sans => "sans",
            Family::Serif => "serif",
            Family::Mono => "mono",
        }
    }
}

/// The egui font family for one face of one family.
///
/// The name is built rather than matched so that a caller cannot ask for a
/// combination that was never registered.
pub fn face(family: Family, bold: bool, italic: bool) -> egui::FontFamily {
    let suffix = match (bold, italic) {
        (false, false) => "",
        (true, false) => "-bold",
        (false, true) => "-italic",
        (true, true) => "-bolditalic",
    };
    egui::FontFamily::Name(format!("{}{suffix}", family.slug()).into())
}

/// Faces a document may name that are not what the three families substitute.
///
/// A resume set in Verdana is a third wider than the same words in Arial; a
/// substitution that ignores the name gets every line break wrong and then
/// every page break after it. These are the names documents actually use, each
/// with the file Windows (or Office) ships for its four styles — an empty name
/// means the style was never shipped, and asking for it falls back to the
/// family's generic face so bold stays bold.
///
/// The three generic families remain the answer for a name not in this table.
const NAMED: &[(&str, [&str; 4])] = &[
    (
        "verdana",
        [
            "verdana.ttf",
            "verdanab.ttf",
            "verdanai.ttf",
            "verdanaz.ttf",
        ],
    ),
    ("tahoma", ["tahoma.ttf", "tahomabd.ttf", "", ""]),
    (
        "trebuchet ms",
        ["trebuc.ttf", "trebucbd.ttf", "trebucit.ttf", "trebucbi.ttf"],
    ),
    (
        "georgia",
        [
            "georgia.ttf",
            "georgiab.ttf",
            "georgiai.ttf",
            "georgiaz.ttf",
        ],
    ),
    ("impact", ["impact.ttf", "", "", ""]),
    (
        "comic sans ms",
        ["comic.ttf", "comicbd.ttf", "comici.ttf", "comicz.ttf"],
    ),
    (
        "palatino linotype",
        ["pala.ttf", "palab.ttf", "palai.ttf", "palabi.ttf"],
    ),
    ("lucida sans unicode", ["l_10646.ttf", "", "", ""]),
    ("lucida console", ["lucon.ttf", "", "", ""]),
    (
        "franklin gothic medium",
        ["framd.ttf", "", "framdit.ttf", ""],
    ),
    (
        "segoe ui",
        [
            "segoeui.ttf",
            "segoeuib.ttf",
            "segoeuii.ttf",
            "segoeuiz.ttf",
        ],
    ),
    (
        "calibri",
        [
            "calibri.ttf",
            "calibrib.ttf",
            "calibrii.ttf",
            "calibriz.ttf",
        ],
    ),
    (
        "candara",
        [
            "candara.ttf",
            "candarab.ttf",
            "candarai.ttf",
            "candaraz.ttf",
        ],
    ),
    (
        "corbel",
        ["corbel.ttf", "corbelb.ttf", "corbeli.ttf", "corbelz.ttf"],
    ),
    (
        "constantia",
        [
            "constan.ttf",
            "constanb.ttf",
            "constani.ttf",
            "constanz.ttf",
        ],
    ),
    (
        "bookman old style",
        ["bookos.ttf", "bookosb.ttf", "bookosi.ttf", "bookosbi.ttf"],
    ),
    (
        "century gothic",
        ["gothic.ttf", "gothicb.ttf", "gothici.ttf", "gothicbi.ttf"],
    ),
    ("garamond", ["gara.ttf", "garabd.ttf", "garait.ttf", ""]),
    (
        "book antiqua",
        ["bkant.ttf", "antquab.ttf", "antquai.ttf", "antquabi.ttf"],
    ),
    (
        "arial narrow",
        ["arialn.ttf", "arialnb.ttf", "arialni.ttf", "arialnbi.ttf"],
    ),
    (
        "courier new",
        ["cour.ttf", "courbd.ttf", "couri.ttf", "courbi.ttf"],
    ),
    // The symbol-encoded faces Word's list galleries name. Their glyphs sit
    // in the U+F0xx private-use range behind a (3,0) symbol cmap, which the
    // shaper resolves like HarfBuzz does; a bullet drawn from the real file
    // is the same dot Word draws, diameter and all.
    ("symbol", ["symbol.ttf", "", "", ""]),
    ("wingdings", ["wingding.ttf", "", "", ""]),
    // The faces LibreOffice documents name. Present on hardly any Windows
    // machine — but when they are, the exact face must win over the
    // substitution below, exactly as Word behaves.
    (
        "dejavu sans",
        [
            "DejaVuSans.ttf",
            "DejaVuSans-Bold.ttf",
            "DejaVuSans-Oblique.ttf",
            "DejaVuSans-BoldOblique.ttf",
        ],
    ),
    (
        "dejavu serif",
        [
            "DejaVuSerif.ttf",
            "DejaVuSerif-Bold.ttf",
            "DejaVuSerif-Italic.ttf",
            "DejaVuSerif-BoldItalic.ttf",
        ],
    ),
    (
        "open sans",
        [
            "OpenSans-Regular.ttf",
            "OpenSans-Bold.ttf",
            "OpenSans-Italic.ttf",
            "OpenSans-BoldItalic.ttf",
        ],
    ),
    (
        "liberation sans",
        [
            "LiberationSans-Regular.ttf",
            "LiberationSans-Bold.ttf",
            "LiberationSans-Italic.ttf",
            "LiberationSans-BoldItalic.ttf",
        ],
    ),
    (
        "liberation serif",
        [
            "LiberationSerif-Regular.ttf",
            "LiberationSerif-Bold.ttf",
            "LiberationSerif-Italic.ttf",
            "LiberationSerif-BoldItalic.ttf",
        ],
    ),
    (
        "liberation mono",
        [
            "LiberationMono-Regular.ttf",
            "LiberationMono-Bold.ttf",
            "LiberationMono-Italic.ttf",
            "LiberationMono-BoldItalic.ttf",
        ],
    ),
];

/// What Word stands in for a face the machine does not have.
///
/// Not guessed: measured against Word 16 laying out the LibreOffice sample
/// documents on this machine (2026-08-15). Open Sans rendered at Segoe UI's
/// exact widths and line height — eight lines with every break point matching
/// and a 13.96pt line at 10.5pt, which is Segoe UI's 1.33em — and DejaVu Sans
/// at Verdana's, the closest advance fingerprint of all 270 installed
/// families by a factor of arbitrariness over the runner-up, which was
/// Verdana's own metric clone. The Liberation faces are metric clones of the
/// classic trio by design, and their `w:altName` says the same.
///
/// Applied only when the exact face is not installed; keys and values are the
/// lowercase the tables above use.
const SUBSTITUTES: &[(&str, &str)] = &[
    ("dejavu sans", "Verdana"),
    ("open sans", "Segoe UI"),
    ("liberation sans", "Arial"),
    ("liberation serif", "Times New Roman"),
    ("liberation mono", "Courier New"),
];

/// Word's substitute for a missing face. Asked with a lowercase name;
/// answers in display case, for a caller that hands the name on to GDI.
pub fn substitute(name: &str) -> Option<&'static str> {
    let name = first_name(name);
    SUBSTITUTES
        .iter()
        .find(|(from, _)| *from == name)
        .map(|(_, to)| *to)
}

/// The first name of a `Liberation Sans;Arial` chain.
///
/// LibreOffice writes its own fallback list straight into `w:rFonts`, and the
/// first entry is the face being asked for — Word reads it the same way.
fn first_name(name: &str) -> &str {
    name.split(';').next().unwrap_or(name).trim()
}

/// The faces from [`NAMED`] that were actually found and registered.
///
/// A `FontFamily::Name` epaint has never been given *panics* when drawn with,
/// so the one source of truth for "may I ask for Verdana bold" is what
/// [`register`] managed to load. Set once; a second registration in the same
/// process keeps the first answer, which is also the one epaint kept.
static NAMED_FACES: OnceLock<BTreeMap<(String, bool, bool), egui::FontFamily>> = OnceLock::new();

/// What [`register`] built out of the machine's own fonts, kept so a document's
/// own faces can be laid over it without reading every file again.
static SYSTEM: OnceLock<egui::FontDefinitions> = OnceLock::new();

/// The faces the open document carries in its own package, which outrank
/// anything the machine has under the same name — the author embedded them
/// precisely so that this reader would use *these* and not a look-alike.
///
/// A lock rather than a `OnceLock` because documents open and close, and the
/// set has to be replaced each time rather than accumulate: a face left behind
/// from the last document would silently re-wrap the next one.
#[allow(clippy::type_complexity)]
static DOCUMENT_FACES: RwLock<
    Option<BTreeMap<(String, bool, bool), (egui::FontFamily, Arc<[u8]>)>>,
> = RwLock::new(None);

/// The registered face for a document's font name, if the machine has it —
/// the exact face first, Word's substitute for it second.
///
/// `None` says to fall back to [`face`] of [`Family::of`] the same name — the
/// substitution that was previously the only answer.
pub fn named_face(name: &str, bold: bool, italic: bool) -> Option<egui::FontFamily> {
    let lower = first_name(name).to_ascii_lowercase();
    if let Some(exact) = exact_face(&lower, bold, italic) {
        return Some(exact);
    }
    let faces = NAMED_FACES.get()?;
    faces
        .get(&(substitute(&lower)?.to_ascii_lowercase(), bold, italic))
        .cloned()
}

/// The registered face for exactly this name, substitution not applied.
///
/// This is how a caller keying its own tables — measured line pitches — asks
/// whether the document's face is real on this machine or standing in.
pub fn exact_face(name: &str, bold: bool, italic: bool) -> Option<egui::FontFamily> {
    let key = (first_name(name).to_ascii_lowercase(), bold, italic);
    if let Some(family) = document_face(&key) {
        return Some(family);
    }
    NAMED_FACES.get()?.get(&key).cloned()
}

/// The registered family for a face the open document embedded, if it did.
fn document_face(key: &(String, bool, bool)) -> Option<egui::FontFamily> {
    let held = DOCUMENT_FACES.read().ok()?;
    let faces = held.as_ref()?;
    // An embedded family that carries only its regular face still answers for
    // bold and italic: Word draws those by leaning and thickening the face it
    // has, which is closer than dropping to a different family altogether.
    faces
        .get(key)
        .or_else(|| faces.get(&(key.0.clone(), false, false)))
        .map(|(family, _)| family.clone())
}

/// The bytes of an embedded face, for a renderer that must carry the font
/// rather than draw with it. Answers only for faces the document itself
/// supplied, so a caller can try the machine's fonts when it says no.
pub fn document_face_file(name: &str, bold: bool, italic: bool) -> Option<Arc<[u8]>> {
    let key = (first_name(name).to_ascii_lowercase(), bold, italic);
    let held = DOCUMENT_FACES.read().ok()?;
    let faces = held.as_ref()?;
    faces
        .get(&key)
        .or_else(|| faces.get(&(key.0.clone(), false, false)))
        .map(|(_, bytes)| bytes.clone())
}

/// One candidate file per face, in preference order.
///
/// Windows first because that is where this is built and run; the others are
/// there so the same binary is not visibly worse elsewhere. Liberation and
/// DejaVu are the usual Linux answers, and Liberation Sans is metric-compatible
/// with Arial, which matters: a column width is stored as a count of digit
/// widths in the workbook's own font.
fn candidates(family: Family, bold: bool, italic: bool) -> Vec<&'static str> {
    let windows: &[&str] = match (family, bold, italic) {
        (Family::Sans, false, false) => &["arial.ttf", "calibri.ttf", "segoeui.ttf"],
        (Family::Sans, true, false) => &["arialbd.ttf", "calibrib.ttf", "segoeuib.ttf"],
        (Family::Sans, false, true) => &["ariali.ttf", "calibrii.ttf", "segoeuii.ttf"],
        (Family::Sans, true, true) => &["arialbi.ttf", "calibriz.ttf", "segoeuiz.ttf"],
        (Family::Serif, false, false) => &["times.ttf", "georgia.ttf", "cambria.ttc"],
        (Family::Serif, true, false) => &["timesbd.ttf", "georgiab.ttf"],
        (Family::Serif, false, true) => &["timesi.ttf", "georgiai.ttf"],
        (Family::Serif, true, true) => &["timesbi.ttf", "georgiaz.ttf"],
        (Family::Mono, false, false) => &["consola.ttf", "cour.ttf"],
        (Family::Mono, true, false) => &["consolab.ttf", "courbd.ttf"],
        (Family::Mono, false, true) => &["consolai.ttf", "couri.ttf"],
        (Family::Mono, true, true) => &["consolaz.ttf", "courbi.ttf"],
    };
    let unix: &[&str] = match (family, bold, italic) {
        (Family::Sans, false, false) => {
            &["LiberationSans-Regular.ttf", "DejaVuSans.ttf", "Arial.ttf"]
        }
        (Family::Sans, true, false) => &[
            "LiberationSans-Bold.ttf",
            "DejaVuSans-Bold.ttf",
            "Arial Bold.ttf",
        ],
        (Family::Sans, false, true) => &[
            "LiberationSans-Italic.ttf",
            "DejaVuSans-Oblique.ttf",
            "Arial Italic.ttf",
        ],
        (Family::Sans, true, true) => &[
            "LiberationSans-BoldItalic.ttf",
            "DejaVuSans-BoldOblique.ttf",
        ],
        (Family::Serif, false, false) => &[
            "LiberationSerif-Regular.ttf",
            "DejaVuSerif.ttf",
            "Times New Roman.ttf",
        ],
        (Family::Serif, true, false) => &["LiberationSerif-Bold.ttf", "DejaVuSerif-Bold.ttf"],
        (Family::Serif, false, true) => &["LiberationSerif-Italic.ttf", "DejaVuSerif-Italic.ttf"],
        (Family::Serif, true, true) => &["LiberationSerif-BoldItalic.ttf"],
        (Family::Mono, false, false) => &[
            "LiberationMono-Regular.ttf",
            "DejaVuSansMono.ttf",
            "Menlo.ttc",
        ],
        (Family::Mono, true, false) => &["LiberationMono-Bold.ttf", "DejaVuSansMono-Bold.ttf"],
        (Family::Mono, false, true) => &["LiberationMono-Italic.ttf", "DejaVuSansMono-Oblique.ttf"],
        (Family::Mono, true, true) => &["LiberationMono-BoldItalic.ttf"],
    };
    let mut all = windows.to_vec();
    all.extend_from_slice(unix);
    all
}

/// The faces Office downloaded rather than Windows installed.
///
/// Word's own default type since 2024 — Aptos — is not shipped with Windows
/// and is not installed into the font directory: Office fetches it on first
/// use into `FontCache\4\CloudFonts`, one directory per family, holding files
/// whose names are opaque numbers. A machine can therefore *have* the face
/// Word laid a document in while every lookup by file name says it does not,
/// and the document is then measured in a stand-in whose every line breaks
/// somewhere else. The directory name is the family; the style is read out of
/// the file, because the number tells nothing.
///
/// Returns `(lowercase family, bold, italic, path)`.
fn cloud_faces() -> Vec<(String, bool, bool, PathBuf)> {
    let Some(local) = std::env::var_os("LOCALAPPDATA") else {
        return Vec::new();
    };
    let root = PathBuf::from(local).join("Microsoft/FontCache/4/CloudFonts");
    let Ok(families) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for family in families.flatten() {
        let name = family.file_name().to_string_lossy().to_ascii_lowercase();
        let Ok(files) = std::fs::read_dir(family.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Some((bold, italic)) = mac_style(&bytes) else {
                continue;
            };
            found.push((name.clone(), bold, italic, path));
        }
    }
    found
}

/// The bold and italic bits of a TrueType file's `head` table.
///
/// `None` when the bytes are not a single TrueType face this can read — a
/// collection, or anything truncated.
fn mac_style(bytes: &[u8]) -> Option<(bool, bool)> {
    let at = |i: usize| -> Option<u16> {
        Some(u16::from_be_bytes([*bytes.get(i)?, *bytes.get(i + 1)?]))
    };
    let tables = at(4)?;
    for index in 0..usize::from(tables) {
        let entry = 12 + 16 * index;
        if bytes.get(entry..entry + 4)? != b"head" {
            continue;
        }
        let offset =
            u32::from_be_bytes(bytes.get(entry + 8..entry + 12)?.try_into().ok()?) as usize;
        let style = at(offset + 44)?;
        return Some((style & 1 != 0, style & 2 != 0));
    }
    None
}

/// Where to look for a bare font file name.
pub(crate) fn font_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windir) = std::env::var_os("SystemRoot") {
        dirs.push(PathBuf::from(windir).join("Fonts"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
    }
    for path in [
        "/System/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/Library/Fonts",
        "/usr/share/fonts/truetype/liberation",
        "/usr/share/fonts/truetype/dejavu",
        "/usr/share/fonts/liberation",
        "/usr/share/fonts/dejavu",
        "/usr/share/fonts/TTF",
        "/usr/share/fonts",
    ] {
        dirs.push(PathBuf::from(path));
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/fonts"));
    }
    dirs
}

fn load(family: Family, bold: bool, italic: bool, dirs: &[PathBuf]) -> Option<Vec<u8>> {
    candidates(family, bold, italic)
        .iter()
        .find_map(|name| read_face(name, dirs))
}

/// One font file by its bare name, from wherever it is.
fn read_face(name: &str, dirs: &[PathBuf]) -> Option<Vec<u8>> {
    if name.is_empty() {
        return None;
    }
    for dir in dirs {
        if let Ok(bytes) = std::fs::read(dir.join(name)) {
            // A collection needs an index to pick a face out of; anything
            // that is not a single face is skipped rather than guessed at.
            if bytes.len() > 4 && &bytes[..4] == b"ttcf" {
                continue;
            }
            return Some(bytes);
        }
    }
    None
}

/// Registers every face that could be found, and returns which ones are real.
///
/// The return value is what tells a painter whether it may ask for bold: a
/// family whose bold face is missing is registered *pointing at the regular
/// one*, so drawing never fails, and the caller can decide whether to fall back
/// to synthesising weight or to accept that this machine has no bold Arial.
pub fn install(ctx: &egui::Context) -> Loaded {
    register(ctx, &font_directories())
}

/// The same, over a given set of directories.
///
/// Separate so a test can register the family *names* without reading a
/// hundred megabytes of type off the disk: the names are what the grid asks
/// for, and epaint refuses to substitute for a family it has never heard of.
pub fn register(ctx: &egui::Context, dirs: &[PathBuf]) -> Loaded {
    let mut definitions = egui::FontDefinitions::default();
    let mut loaded = Loaded::default();

    for family in [Family::Sans, Family::Serif, Family::Mono] {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let key = format!("{}-{}{}", family.slug(), bold as u8, italic as u8);
            let name = face(family, bold, italic);

            let fallback = match family {
                Family::Mono => egui::FontFamily::Monospace,
                _ => egui::FontFamily::Proportional,
            };
            // The built-in face last in every list, so a glyph the system font
            // lacks is still drawn rather than coming out as a blank box.
            let mut chain: Vec<String> = definitions
                .families
                .get(&fallback)
                .cloned()
                .unwrap_or_default();

            if let Some(bytes) = load(family, bold, italic, dirs) {
                definitions
                    .font_data
                    .insert(key.clone(), Arc::new(egui::FontData::from_owned(bytes)));
                chain.insert(0, key);
                loaded.faces.insert((family, bold, italic), true);
            } else {
                loaded.faces.insert((family, bold, italic), false);
            }
            definitions.families.insert(name, chain);
        }
    }

    // The exact names, on top of the generic families they would otherwise
    // fall into. Each named face's chain continues with its shape's generic
    // chain, so a glyph Verdana lacks is drawn by Arial rather than as tofu.
    let mut named = BTreeMap::new();
    for (name, files) in NAMED {
        let shape = Family::of(name);
        for (index, (bold, italic)) in [(false, false), (true, false), (false, true), (true, true)]
            .into_iter()
            .enumerate()
        {
            let Some(bytes) = read_face(files[index], dirs) else {
                continue;
            };
            let key = format!("named-{}-{}{}", name, bold as u8, italic as u8);
            let family =
                egui::FontFamily::Name(format!("{name}-{}{}", bold as u8, italic as u8).into());
            let mut chain: Vec<String> = definitions
                .families
                .get(&face(shape, bold, italic))
                .cloned()
                .unwrap_or_default();
            definitions
                .font_data
                .insert(key.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            chain.insert(0, key);
            definitions.families.insert(family.clone(), chain);
            named.insert((name.to_string(), bold, italic), family);
        }
    }
    // The faces Office downloaded, under the family name the document asks
    // for. After the shipped table, so a name that is both installed and
    // cached keeps the file Windows has.
    for (name, bold, italic, path) in cloud_faces() {
        if named.contains_key(&(name.clone(), bold, italic)) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let key = format!("cloud-{name}-{}{}", bold as u8, italic as u8);
        let family =
            egui::FontFamily::Name(format!("{name}-{}{}", bold as u8, italic as u8).into());
        let mut chain: Vec<String> = definitions
            .families
            .get(&face(Family::of(&name), bold, italic))
            .cloned()
            .unwrap_or_default();
        definitions
            .font_data
            .insert(key.clone(), Arc::new(egui::FontData::from_owned(bytes)));
        chain.insert(0, key);
        definitions.families.insert(family.clone(), chain);
        named.insert((name, bold, italic), family);
    }
    // A second registration keeps the first process-wide answer — which is
    // fine, because it is also the set of families epaint was actually given.
    let _ = NAMED_FACES.set(named);
    let _ = SYSTEM.set(definitions.clone());

    ctx.set_fonts(definitions);
    loaded
}

/// Lays the faces a document carries over the machine's own, and drops
/// whatever the last document left.
///
/// Called on every open, including with an empty list — a document with no
/// embedded type has to *clear* the previous one's, or its Ubuntu is drawn in
/// the Ubuntu of a file that is no longer on screen.
///
/// Each face is given the same `Name` family an installed face of that name
/// would have had, so nothing downstream needs to know where the type came
/// from; the generic chain still follows it, so a glyph the embedded subset
/// lacks is drawn from a system face rather than coming out as tofu.
pub fn embed_document(
    ctx: &egui::Context,
    faces: &[(String, bool, bool, Vec<u8>)],
    named: &[String],
) {
    let had = DOCUMENT_FACES
        .read()
        .ok()
        .map(|held| held.is_some())
        .unwrap_or(false);

    // Every face this document could want, from the two places one can come
    // from. The machine's own copy is preferred over the package's: an
    // embedded font is what Word falls back to when the face is missing, not
    // something it draws with in preference to the real thing — see
    // [`crate::catalogue`].
    let mut wanted: Vec<(String, bool, bool, Vec<u8>)> = Vec::new();
    let mut seen: BTreeMap<(String, bool, bool), usize> = BTreeMap::new();
    let mut want = |name: &str, bold: bool, italic: bool, fallback: Option<&Vec<u8>>| {
        let key = (name.to_ascii_lowercase(), bold, italic);
        if seen.contains_key(&key) {
            return;
        }
        // Already known to the named-face table, and registered from the same
        // file: registering it twice would only cost the atlas.
        if NAMED.iter().any(|(known, _)| *known == key.0) {
            return;
        }
        let bytes = match crate::catalogue::file(name, bold, italic) {
            Some(path) => std::fs::read(path).ok(),
            None => None,
        };
        let Some(bytes) = bytes.or_else(|| fallback.cloned()) else {
            return;
        };
        seen.insert(key, wanted.len());
        wanted.push((name.to_owned(), bold, italic, bytes));
    };
    for (name, bold, italic, bytes) in faces {
        want(name, *bold, *italic, Some(bytes));
    }
    for name in named {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            want(name, bold, italic, None);
        }
    }

    if wanted.is_empty() && !had {
        return;
    }
    let faces = &wanted;

    let mut definitions = SYSTEM.get().cloned().unwrap_or_default();
    let mut registered = BTreeMap::new();
    for (index, (name, bold, italic, bytes)) in faces.iter().enumerate() {
        // Two embeddings of one face would otherwise collide on the key; the
        // position keeps them apart and the later one wins the family.
        let key = format!("embedded-{index}-{name}-{}{}", *bold as u8, *italic as u8);
        let family =
            egui::FontFamily::Name(format!("{name}-{}{}", *bold as u8, *italic as u8).into());
        let mut chain: Vec<String> = definitions
            .families
            .get(&face(Family::of(name), *bold, *italic))
            .cloned()
            .unwrap_or_default();
        definitions.font_data.insert(
            key.clone(),
            Arc::new(egui::FontData::from_owned(bytes.clone())),
        );
        chain.insert(0, key);
        definitions.families.insert(family.clone(), chain);
        registered.insert(
            (name.to_ascii_lowercase(), *bold, *italic),
            (family, Arc::from(bytes.clone().into_boxed_slice())),
        );
    }

    if let Ok(mut held) = DOCUMENT_FACES.write() {
        *held = Some(registered);
    }
    ctx.set_fonts(definitions);
}

/// The font file a document's face name resolves to, and where it came from.
///
/// This is the *same* resolution [`register`] performs — the exact-name table
/// first, the generic shape's candidates after — re-read from disk, for a
/// renderer that must embed the bytes rather than draw with them. The path is
/// returned so a caller can recognise two names resolving to one file and
/// embed it once.
pub fn face_file(name: &str, bold: bool, italic: bool) -> Option<(PathBuf, Vec<u8>)> {
    let dirs = font_directories();
    let index = match (bold, italic) {
        (false, false) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (true, true) => 3,
    };
    // The exact face, then Word's substitute for it, then the generic shape —
    // the same order the screen resolves in. The substitute's own generic
    // shape matters: Liberation Serif's stand-in is Times New Roman, which is
    // not in the exact-name table but is the serif chain's first candidate.
    let name = first_name(name);
    let lower = name.to_ascii_lowercase();
    let mut files: Vec<&str> = Vec::new();
    let push_named = |files: &mut Vec<&str>, name: &str| {
        if let Some((_, faces)) = NAMED.iter().find(|(named, _)| *named == name) {
            if !faces[index].is_empty() {
                files.push(faces[index]);
            }
        }
    };
    push_named(&mut files, &lower);
    if let Some(sub) = substitute(&lower) {
        push_named(&mut files, &sub.to_ascii_lowercase());
        files.extend(candidates(Family::of(sub), bold, italic));
    }
    files.extend(candidates(Family::of(name), bold, italic));
    // A cloud face is found by family, not by file name, so it is asked for
    // separately — and before the generic candidates, because it is the exact
    // face the document named.
    if let Some((_, _, _, path)) = cloud_faces()
        .into_iter()
        .find(|(family, b, i, _)| *family == lower && *b == bold && *i == italic)
    {
        if let Ok(bytes) = std::fs::read(&path) {
            return Some((path, bytes));
        }
    }
    for file in files {
        for dir in &dirs {
            let path = dir.join(file);
            if let Ok(bytes) = std::fs::read(&path) {
                if bytes.len() > 4 && &bytes[..4] == b"ttcf" {
                    continue;
                }
                return Some((path, bytes));
            }
        }
    }
    None
}

/// The family name GDI should be asked for, mirroring what the screen shows.
///
/// A face the screen actually registered is real and GDI knows it. A face
/// with a substitute prints as the substitute the screen drew it in. Anything
/// else was drawn in the generic face of its shape, and naming that face
/// keeps the printout in the type the user approved — GDI's own fuzzy
/// matching would pick something else for an unknown name.
pub fn gdi_family(name: &str) -> String {
    let name = first_name(name);
    let lower = name.to_ascii_lowercase();
    if exact_face(&lower, false, false).is_some() {
        return name.to_owned();
    }
    if let Some(sub) = substitute(&lower) {
        return sub.to_owned();
    }
    if NAMED.iter().any(|(named, _)| *named == lower) {
        return name.to_owned();
    }
    match Family::of(name) {
        Family::Sans => "Arial",
        Family::Serif => "Times New Roman",
        Family::Mono => "Consolas",
    }
    .to_owned()
}

/// Which faces turned out to exist on this machine.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    faces: BTreeMap<(Family, bool, bool), bool>,
}

impl Loaded {
    /// True when a genuine face was found, rather than a fallback standing in.
    pub fn has(&self, family: Family, bold: bool, italic: bool) -> bool {
        self.faces
            .get(&(family, bold, italic))
            .copied()
            .unwrap_or(false)
    }
}

// `Family` is a map key above, so it needs the ordering traits; deriving them
// on the enum itself would let it be compared, which means nothing.
impl PartialOrd for Family {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Family {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_name_resolves_to_the_shape_it_belongs_to() {
        assert_eq!(Family::of("Arial"), Family::Sans);
        assert_eq!(Family::of("Calibri"), Family::Sans);
        assert_eq!(Family::of("Times New Roman"), Family::Serif);
        assert_eq!(Family::of("Cambria"), Family::Serif);
        assert_eq!(Family::of("Courier New"), Family::Mono);
        assert_eq!(Family::of("Consolas"), Family::Mono);
        // A name nobody has is sans, which is what Excel substitutes too.
        assert_eq!(Family::of("Chalkduster Pro"), Family::Sans);
    }

    #[test]
    fn the_named_table_is_lowercase_and_unique_because_lookups_lowercase() {
        let mut seen = std::collections::BTreeSet::new();
        for (name, files) in NAMED {
            assert_eq!(*name, name.to_ascii_lowercase(), "{name} would never match");
            assert!(seen.insert(*name), "{name} is listed twice");
            assert!(!files[0].is_empty(), "{name} has no regular face at all");
        }
    }

    #[test]
    fn the_printer_is_asked_for_the_face_the_screen_showed() {
        // Named faces print as themselves; unknown names print as the
        // substitute the screen drew them in, not whatever GDI would guess.
        assert_eq!(gdi_family("Verdana"), "Verdana");
        assert_eq!(gdi_family("verdana"), "verdana", "case is GDI's problem");
        assert_eq!(gdi_family("Chalkduster Pro"), "Arial");
        assert_eq!(gdi_family("Cambria"), "Times New Roman");
        assert_eq!(gdi_family("Courier New"), "Courier New");
        // A missing face prints as Word's substitute for it, not as the
        // generic shape.
        assert_eq!(gdi_family("Open Sans"), "Segoe UI");
        assert_eq!(gdi_family("DejaVu Sans"), "Verdana");
        assert_eq!(gdi_family("Liberation Serif"), "Times New Roman");
    }

    #[test]
    fn a_missing_face_substitutes_the_face_word_would() {
        assert_eq!(substitute("dejavu sans"), Some("Verdana"));
        assert_eq!(substitute("open sans"), Some("Segoe UI"));
        assert_eq!(substitute("liberation sans"), Some("Arial"));
        assert_eq!(substitute("verdana"), None, "real faces are not mapped");
    }

    #[test]
    fn every_face_has_its_own_family_name() {
        let mut seen = std::collections::BTreeSet::new();
        for family in [Family::Sans, Family::Serif, Family::Mono] {
            for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                assert!(
                    seen.insert(face(family, bold, italic)),
                    "{family:?} {bold} {italic} collided"
                );
            }
        }
        assert_eq!(seen.len(), 12);
    }
}
