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
use std::sync::{Arc, OnceLock};

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
];

/// The faces from [`NAMED`] that were actually found and registered.
///
/// A `FontFamily::Name` epaint has never been given *panics* when drawn with,
/// so the one source of truth for "may I ask for Verdana bold" is what
/// [`register`] managed to load. Set once; a second registration in the same
/// process keeps the first answer, which is also the one epaint kept.
static NAMED_FACES: OnceLock<BTreeMap<(String, bool, bool), egui::FontFamily>> = OnceLock::new();

/// The registered face for a document's exact font name, if the machine has it.
///
/// `None` says to fall back to [`face`] of [`Family::of`] the same name — the
/// substitution that was previously the only answer.
pub fn named_face(name: &str, bold: bool, italic: bool) -> Option<egui::FontFamily> {
    let faces = NAMED_FACES.get()?;
    faces
        .get(&(name.to_ascii_lowercase(), bold, italic))
        .cloned()
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

/// Where to look for a bare font file name.
fn font_directories() -> Vec<PathBuf> {
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
    // A second registration keeps the first process-wide answer — which is
    // fine, because it is also the set of families epaint was actually given.
    let _ = NAMED_FACES.set(named);

    ctx.set_fonts(definitions);
    loaded
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
