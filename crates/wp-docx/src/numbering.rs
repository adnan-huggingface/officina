//! `numbering.xml` — abstract list definitions and the instances of them.

use quick_xml::events::Event;
use quick_xml::Reader;

use wp_model::numbering::{
    AbstractNum, Level, LevelOverride, MultiLevel, Num, NumFormat, Numbering, PictureBullet, Suffix,
};
use wp_model::prop::Justify;
use wp_model::units::Emu;

use crate::ctx::Ctx;
use crate::props;
use crate::xml::{attr, attr_i32, attr_u32, end_local_name, local_name, on_off, val};

pub(crate) fn read(xml: &[u8], ctx: &mut Ctx<'_>) -> Numbering {
    let mut numbering = Numbering::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => match local_name(&e) {
                b"abstractNum" => {
                    let Some(id) = attr_u32(&e, b"abstractNumId") else {
                        continue;
                    };
                    numbering.insert_abstract(read_abstract(&mut reader, ctx, id));
                }
                b"num" => {
                    let Some(id) = attr_u32(&e, b"numId") else {
                        continue;
                    };
                    if let Some(instance) = read_num(&mut reader, ctx, id) {
                        numbering.insert_num(instance);
                    }
                }
                b"numPicBullet" => {
                    let id = attr_u32(&e, b"numPicBulletId").unwrap_or(0);
                    if let Some(bullet) = read_picture_bullet(&mut reader) {
                        numbering.insert_picture_bullet(id, bullet);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    numbering
}

/// `<w:numPicBullet>` — the image a level draws instead of a bullet glyph.
///
/// Word writes it as VML: a `<v:shape>` whose `style` states the size and whose
/// `<v:imagedata>` names the image. The size matters as much as the picture —
/// the icons Word ships are hundreds of pixels across and belong on a line nine
/// points tall. A `<w:drawing>` form is accepted too, for the producer that
/// writes one, and its size then comes from the extent in EMU.
fn read_picture_bullet(reader: &mut Reader<&[u8]>) -> Option<PictureBullet> {
    let mut rel: Option<String> = None;
    let (mut width, mut height) = (0.0, 0.0);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"shape" => {
                    if let Some(style) = attr(&e, b"style") {
                        width = css_points(&style, "width").unwrap_or(width);
                        height = css_points(&style, "height").unwrap_or(height);
                    }
                }
                b"imagedata" => rel = attr(&e, b"id").or(rel),
                b"blip" => rel = attr(&e, b"embed").or(rel),
                b"extent" => {
                    let emu = |name: &[u8]| attr_i32(&e, name).map(|v| Emu(v as i64).points());
                    width = emu(b"cx").unwrap_or(width);
                    height = emu(b"cy").unwrap_or(height);
                }
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"numPicBullet" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    Some(PictureBullet {
        rel: crate::parts::qualified(crate::parts::NUMBERING, &rel?).into(),
        // A shape that states no size is drawn at the size of the line it sits
        // on, which is the one measurement always to hand. Zero says so.
        width: width.max(0.0),
        height: height.max(0.0),
    })
}

/// One length out of a VML `style` attribute: `width:9pt;height:9pt`.
///
/// CSS units, because VML's `style` is CSS. Word writes points here; the
/// others are cheap to accept.
fn css_points(style: &str, want: &str) -> Option<f64> {
    let value = style
        .split(';')
        .filter_map(|pair| pair.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case(want))?
        .1
        .trim();
    let split = value
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+')
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: f64 = number.parse().ok()?;
    let scale = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "pt" => 1.0,
        "in" => 72.0,
        "pc" => 12.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        // A CSS pixel is a ninety-sixth of an inch, which is what Word means
        // by one everywhere else too.
        "px" => 72.0 / 96.0,
        _ => return None,
    };
    Some(number * scale)
}

fn read_abstract(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>, id: u32) -> AbstractNum {
    let mut definition = AbstractNum::new(id);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"nsid" => definition.nsid = val(&e).map(Into::into),
                b"tmpl" => definition.template = val(&e).map(Into::into),
                b"name" => definition.name = val(&e).map(Into::into),
                b"multiLevelType" => {
                    definition.multi_level = val(&e)
                        .as_deref()
                        .and_then(MultiLevel::from_val)
                        .unwrap_or_default()
                }
                b"numStyleLink" => definition.num_style_link = val(&e).map(Into::into),
                b"styleLink" => definition.style_link = val(&e).map(Into::into),
                b"lvl" => {
                    let index = attr_u32(&e, b"ilvl").unwrap_or(0).min(8) as u8;
                    definition.set_level(read_level(reader, ctx, index));
                }
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"abstractNum" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    definition
}

fn read_level(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>, index: u8) -> Level {
    let mut level = Level::new(index);
    // `Level::new` invents a plausible `%n.` for a level nobody spelled out. A
    // level that *is* spelled out states its own text, and a bullet level states
    // a glyph; starting from a default that is never cleared would print `%1.`
    // beside every bullet in the document.
    let mut stated_text = false;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"start" => level.start = attr_i32(&e, b"val").unwrap_or(1),
                b"numFmt" => {
                    level.format = val(&e)
                        .as_deref()
                        .map(NumFormat::from_val)
                        .unwrap_or_default()
                }
                b"lvlText" => {
                    if let Some(text) = val(&e) {
                        level.text = text.into();
                        stated_text = true;
                    }
                }
                b"lvlJc" => {
                    level.justify = val(&e)
                        .as_deref()
                        .and_then(Justify::from_val)
                        .unwrap_or_default()
                }
                b"lvlRestart" => level.restart = attr_u32(&e, b"val").map(|v| v.min(9) as u8),
                b"suff" => {
                    level.suffix = val(&e)
                        .as_deref()
                        .and_then(Suffix::from_val)
                        .unwrap_or_default()
                }
                b"isLgl" => level.legal = on_off(&e),
                b"lvlPicBulletId" => level.picture_bullet = attr_u32(&e, b"val"),
                b"pPr" => level.para = props::para_props(reader, ctx).props,
                b"rPr" => level.run = props::run_props(reader, ctx).props,
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"lvl" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    if !stated_text && level.format == NumFormat::Bullet {
        level.text = "\u{2022}".into();
    }
    level
}

fn read_num(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>, id: u32) -> Option<Num> {
    let mut abstract_id = None;
    let mut overrides = Vec::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"abstractNumId" => abstract_id = attr_u32(&e, b"val"),
                b"lvlOverride" => {
                    let index = attr_u32(&e, b"ilvl").unwrap_or(0).min(8) as u8;
                    overrides.push(read_override(reader, ctx, index));
                }
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"num" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    Some(Num {
        id,
        // A `<w:num>` with no `<w:abstractNumId>` names nothing and cannot be
        // resolved; dropping it is what makes `Numbering::level` return `None`
        // and the paragraph draw no number, which is what Word does.
        abstract_id: abstract_id?,
        overrides,
    })
}

fn read_override(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>, index: u8) -> LevelOverride {
    let mut over = LevelOverride {
        index,
        start: None,
        level: None,
    };
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"startOverride" => over.start = attr_i32(&e, b"val"),
                b"lvl" => {
                    let level_index = attr_u32(&e, b"ilvl").unwrap_or(index as u32).min(8) as u8;
                    over.level = Some(Box::new(read_level(reader, ctx, level_index)));
                }
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"lvlOverride" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    over
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::test_ctx;
    use wp_model::numbering::Counters;
    use wp_model::prop::NumRef;

    fn numbering_of(xml: &str) -> Numbering {
        let (mut styles, mut headers) = test_ctx();
        let mut ctx = Ctx::new(&mut styles, &mut headers);
        read(xml.as_bytes(), &mut ctx)
    }

    const TWO_LEVELS: &str = r#"<w:numbering>
      <w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/>
        <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl>
        <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2)"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl>
      </w:abstractNum>
      <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
    </w:numbering>"#;

    #[test]
    fn a_picture_bullet_is_read_with_the_size_its_shape_states() {
        // Word writes the picture as VML and leaves a Symbol dot in
        // `<w:lvlText>` for a reader that cannot fetch it. Taking the dot and
        // ignoring the picture is exactly the mistake that leaves the
        // demonstration document saying "this bullet uses an image" beside a
        // bullet that is not one.
        let numbering = numbering_of(
            r##"<w:numbering>
              <w:numPicBullet w:numPicBulletId="0"><w:pict>
                <v:shape id="_x0000_i1030" type="#_x0000_t75" style="width:9pt;height:9pt" o:bullet="t">
                  <v:imagedata r:id="rId1" o:title="BD10267_"/>
                </v:shape>
              </w:pict></w:numPicBullet>
              <w:abstractNum w:abstractNumId="0">
                <w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="&#xF0B7;"/><w:lvlPicBulletId w:val="0"/></w:lvl>
              </w:abstractNum>
              <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
            </w:numbering>"##,
        );
        let level = numbering.level(1, 0).expect("the level");
        assert_eq!(level.picture_bullet, Some(0), "the level names a picture");
        let bullet = numbering.picture_bullet(0).expect("and the part holds it");
        assert_eq!(
            &*bullet.rel, "numbering:rId1",
            "qualified: numbering.xml numbers its relationships from rId1 too"
        );
        assert_eq!((bullet.width, bullet.height), (9.0, 9.0));
        assert!(
            numbering.picture_bullet(7).is_none(),
            "and a level pointing at nothing gets nothing"
        );
    }

    #[test]
    fn a_shapes_size_is_read_in_whatever_css_unit_it_states() {
        assert_eq!(css_points("width:9pt;height:12pt", "height"), Some(12.0));
        assert_eq!(css_points("WIDTH: 1in ; height:9pt", "width"), Some(72.0));
        assert_eq!(css_points("width:96px", "width"), Some(72.0));
        assert_eq!(css_points("width:9", "width"), Some(9.0), "bare is points");
        assert_eq!(css_points("width:9pt", "height"), None);
        assert_eq!(css_points("width:auto", "width"), None);
    }

    fn at(num_id: u32, level: u8) -> NumRef {
        NumRef { num_id, level }
    }

    #[test]
    fn a_list_definition_and_its_instance_are_read_and_count() {
        let numbering = numbering_of(TWO_LEVELS);
        let mut counters = Counters::new();
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("1.")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 1)).as_deref(),
            Some("a)")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 1)).as_deref(),
            Some("b)")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("2.")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 1)).as_deref(),
            Some("a)")
        );
    }

    #[test]
    fn a_level_carries_the_indent_a_paragraph_inherits_from_it() {
        let numbering = numbering_of(TWO_LEVELS);
        let level = numbering.level(1, 1).expect("level two exists");
        assert_eq!(level.para.indent.start, Some(wp_model::Twips(1440)));
        assert_eq!(level.para.indent.first_line_offset(), wp_model::Twips(-360));
    }

    #[test]
    fn a_bullet_level_with_no_text_still_draws_a_bullet() {
        // A `<w:lvl>` for a bullet always states its glyph in practice, but a
        // level built from a default `%1.` would print a number beside every
        // bullet if one ever did not.
        let numbering = numbering_of(
            r#"<w:numbering><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#,
        );
        let mut counters = Counters::new();
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("\u{2022}")
        );
    }

    #[test]
    fn restart_numbering_is_a_start_override_on_an_instance() {
        let numbering = numbering_of(
            r#"<w:numbering>
              <w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum>
              <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
              <w:num w:numId="2"><w:abstractNumId w:val="0"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="7"/></w:lvlOverride></w:num>
            </w:numbering>"#,
        );
        let mut counters = Counters::new();
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("1.")
        );
        assert_eq!(
            counters.advance(&numbering, at(2, 0)).as_deref(),
            Some("7.")
        );
        assert_eq!(
            counters.advance(&numbering, at(2, 0)).as_deref(),
            Some("8.")
        );
    }

    #[test]
    fn a_num_naming_no_definition_is_dropped_rather_than_guessed() {
        let numbering = numbering_of(r#"<w:numbering><w:num w:numId="4"/></w:numbering>"#);
        assert!(numbering.num(4).is_none());
        let mut counters = Counters::new();
        assert_eq!(counters.advance(&numbering, at(4, 0)), None);
    }
}
