//! Pivot table parts: the layout and the cache definition beside it.
//!
//! Two files for one table, and the join between them is positional: a
//! `<pivotField>` is not named, it is the *nth* child, and the name comes from
//! the nth `<cacheField>` in the cache definition. Reading either list without
//! keeping its order labels every field with its neighbour's name.
//!
//! Which field goes where is a third list again: `<rowFields><field x="2"/>`
//! indexes back into the `<pivotField>` list. Three indirections for one label,
//! which is why this is read into a flat `Field` and never touched afterwards.

use quick_xml::events::Event;
use quick_xml::Reader;

use ss_model::pivot::{Axis, Field, PivotTable};

use crate::error::{xml_err, Result};
use crate::xml::{attr_raw, attr_text, attr_u32, end_local_name, local_name};

/// The layout half, read from `pivotTableN.xml`.
#[derive(Debug, Default)]
pub(crate) struct Layout {
    pub name: String,
    pub location: Option<ss_model::CellRange>,
    /// One entry per `<pivotField>`, in order.
    pub axes: Vec<Option<Axis>>,
    /// `(field index, aggregate function)` for each `<dataField>`.
    pub data: Vec<(usize, Option<String>)>,
}

pub(crate) fn parse_table(part: &str, data: &[u8]) -> Result<Layout> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = false;
    let mut out = Layout::default();
    let mut buf = Vec::new();
    // `<field x="2"/>` appears under four different parents and means a
    // different axis under each.
    let mut section: Option<Axis> = None;

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => match local_name(e) {
                b"pivotTableDefinition" => {
                    out.name = attr_text(e, b"name").unwrap_or_default();
                }
                b"location" => {
                    out.location =
                        attr_raw(e, b"ref").and_then(|raw| crate::sheet::parse_range_bytes(&raw));
                }
                b"pivotField" => {
                    // `axis="axisRow"`, or a data field flagged rather than
                    // placed. A field on no axis is still in the list, and
                    // dropping it would shift every index after it.
                    let axis = match attr_raw(e, b"axis").as_deref() {
                        Some(b"axisRow") => Some(Axis::Row),
                        Some(b"axisCol") => Some(Axis::Column),
                        Some(b"axisPage") => Some(Axis::Filter),
                        _ => None,
                    };
                    out.axes.push(axis);
                }
                b"rowFields" => section = Some(Axis::Row),
                b"colFields" => section = Some(Axis::Column),
                b"pageFields" => section = Some(Axis::Filter),
                b"dataFields" => section = Some(Axis::Data),
                b"field" => {
                    // `x="-2"` is the placeholder for "values", which is not a
                    // source field and has no name in the cache.
                    if let (Some(Axis::Row | Axis::Column | Axis::Filter), Some(index)) =
                        (section, attr_u32(e, b"x"))
                    {
                        if let Some(slot) = out.axes.get_mut(index as usize) {
                            *slot = section;
                        }
                    }
                }
                b"dataField" => {
                    if let Some(index) = attr_u32(e, b"fld") {
                        out.data.push((index as usize, attr_text(e, b"subtotal")));
                    }
                }
                _ => {}
            },
            Event::End(ref e) => match end_local_name(e) {
                b"rowFields" | b"colFields" | b"pageFields" | b"dataFields" => section = None,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// The names half, read from `pivotCacheDefinitionN.xml`.
#[derive(Debug, Default)]
pub(crate) struct Cache {
    pub names: Vec<String>,
    /// `<worksheetSource ref="A1:C20" sheet="Data"/>` written back as an A1
    /// reference, when the cache says where its records came from.
    pub source: Option<String>,
}

pub(crate) fn parse_cache(part: &str, data: &[u8]) -> Result<Cache> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = false;
    let mut out = Cache::default();
    let mut buf = Vec::new();
    // `<cacheField>` has `<sharedItems>` under it holding `<s v="..."/>`
    // elements, which are values rather than field names.
    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => match local_name(e) {
                b"cacheField" => out.names.push(attr_text(e, b"name").unwrap_or_default()),
                b"worksheetSource" => {
                    let area = attr_text(e, b"ref").or_else(|| attr_text(e, b"name"));
                    out.source = match (attr_text(e, b"sheet"), area) {
                        (Some(sheet), Some(area)) => Some(format!("{sheet}!{area}")),
                        (None, Some(area)) => Some(area),
                        _ => None,
                    };
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// Joins the two halves into the model's flat view.
pub(crate) fn build(part: &str, layout: Layout, cache: Cache) -> Option<PivotTable> {
    let location = layout.location?;
    let name_of = |index: usize| {
        cache
            .names
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("Field {}", index + 1))
    };

    let mut fields: Vec<Field> = layout
        .axes
        .iter()
        .enumerate()
        .filter_map(|(index, axis)| {
            axis.map(|axis| Field {
                name: name_of(index),
                axis,
                function: None,
            })
        })
        .collect();
    for (index, function) in layout.data {
        fields.push(Field {
            name: name_of(index),
            axis: Axis::Data,
            // Absent means `sum`, which is the default and by far the common
            // case; writing it out makes the model say the same thing whichever
            // way the file spelled it.
            function: Some(function.unwrap_or_else(|| "sum".to_string())),
        });
    }

    Some(PivotTable {
        part: part.to_string(),
        name: layout.name,
        location,
        source: cache.source,
        fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = r#"<?xml version="1.0"?>
<pivotTableDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
  name="PivotTable1" cacheId="1" dataOnRows="0">
  <location ref="A3:C9" firstHeaderRow="1" firstDataRow="2" firstDataCol="1"/>
  <pivotFields count="3">
    <pivotField axis="axisRow" showAll="0"><items count="2"/></pivotField>
    <pivotField showAll="0"/>
    <pivotField dataField="1" showAll="0"/>
  </pivotFields>
  <rowFields count="1"><field x="0"/></rowFields>
  <colFields count="1"><field x="-2"/></colFields>
  <dataFields count="1"><dataField name="Sum of Sales" fld="2" baseField="0" baseItem="0"/></dataFields>
</pivotTableDefinition>"#;

    const CACHE: &str = r#"<?xml version="1.0"?>
<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" recordCount="6">
  <cacheSource type="worksheet"><worksheetSource ref="A1:C7" sheet="Data"/></cacheSource>
  <cacheFields count="3">
    <cacheField name="Region" numFmtId="0"><sharedItems count="2"><s v="North"/><s v="South"/></sharedItems></cacheField>
    <cacheField name="Month" numFmtId="0"><sharedItems/></cacheField>
    <cacheField name="Sales" numFmtId="0"><sharedItems containsSemiMixedTypes="0"/></cacheField>
  </cacheFields>
</pivotCacheDefinition>"#;

    fn built() -> PivotTable {
        let layout = parse_table("pivotTable1.xml", TABLE.as_bytes()).expect("parses");
        let cache = parse_cache("pivotCacheDefinition1.xml", CACHE.as_bytes()).expect("parses");
        build("/xl/pivotTables/pivotTable1.xml", layout, cache).expect("has a location")
    }

    #[test]
    fn the_region_and_the_source_come_back() {
        let pivot = built();
        assert_eq!(pivot.name, "PivotTable1");
        assert_eq!(
            pivot.location,
            ss_model::CellRange::new(
                ss_model::CellRef::from_a1("A3").expect("a1"),
                ss_model::CellRef::from_a1("C9").expect("a1"),
            )
        );
        assert_eq!(pivot.source.as_deref(), Some("Data!A1:C7"));
    }

    #[test]
    fn fields_are_named_by_position_in_the_cache_and_not_by_the_layout() {
        // The join is positional. A `<pivotField>` carries no name at all, and
        // reading the two lists out of step labels every field with its
        // neighbour's name — which is invisible until someone reads the table.
        let pivot = built();
        let row: Vec<&str> = pivot
            .fields_on(Axis::Row)
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(row, ["Region"]);
        let data: Vec<&str> = pivot
            .fields_on(Axis::Data)
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(data, ["Sales"]);
        assert_eq!(
            pivot
                .fields_on(Axis::Data)
                .next()
                .and_then(|f| f.function.as_deref()),
            Some("sum"),
            "an absent subtotal means sum"
        );
    }

    #[test]
    fn the_values_placeholder_is_not_a_source_field() {
        // `<field x="-2"/>` under `<colFields>` is where the data fields go,
        // not a field of the cache. Read as an index it would be negative.
        let pivot = built();
        assert_eq!(pivot.fields_on(Axis::Column).count(), 0);
    }
}
