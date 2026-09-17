//! A tool a helper may ask for: a name, what it is for, and the shape of what
//! it takes. Running it is the application's business, never the helper's.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: String,
    /// When to use the tool as much as what it does: a helper decides whether to
    /// call a tool from this sentence alone.
    pub description: String,
    /// A JSON Schema for the tool's input. Claude is asked to keep to it
    /// strictly, which it can only do when every object in it lists what it
    /// requires and forbids anything else; [`Tool::is_strict`] says whether this
    /// one does.
    pub schema: Value,
}

impl Tool {
    pub fn new(name: impl Into<String>, description: impl Into<String>, schema: Value) -> Tool {
        Tool {
            name: name.into(),
            description: description.into(),
            schema,
        }
    }

    /// Whether every object in the schema says `additionalProperties: false`
    /// and names what it requires — what a strict tool must.
    pub fn is_strict(&self) -> bool {
        fn strict(schema: &Value) -> bool {
            let Some(map) = schema.as_object() else {
                return true;
            };
            if map.get("type").and_then(Value::as_str) == Some("object")
                && (map.get("additionalProperties") != Some(&Value::Bool(false))
                    || !map.contains_key("required"))
            {
                return false;
            }
            map.values().all(|value| match value {
                Value::Array(items) => items.iter().all(strict),
                other => strict(other),
            })
        }
        strict(&self.schema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_schema_is_strict_only_when_every_object_in_it_is_closed() {
        let open = Tool::new("t", "d", json!({"type": "object", "properties": {}}));
        assert!(!open.is_strict());
        let closed = Tool::new(
            "t",
            "d",
            json!({"type": "object", "additionalProperties": false, "required": ["cells"],
                   "properties": {"cells": {"type": "array", "items":
                       {"type": "object", "properties": {"at": {"type": "string"}}}}}}),
        );
        assert!(!closed.is_strict(), "the inner object is still open");
        let all = Tool::new(
            "t",
            "d",
            json!({"type": "object", "additionalProperties": false, "required": ["cells"],
                   "properties": {"cells": {"type": "array", "items":
                       {"type": "object", "additionalProperties": false, "required": ["at"],
                        "properties": {"at": {"type": "string"}}}}}}),
        );
        assert!(all.is_strict());
    }
}
