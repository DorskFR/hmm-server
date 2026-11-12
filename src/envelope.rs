//! Structural validation of a report against `schema/envelope.v1.json`, the schema published by
//! `@dorsk/hmm`, plus the three rules a JSON schema cannot express.

use std::collections::HashSet;
use std::sync::LazyLock;

use jsonschema::Validator;
use serde_json::Value;

const CONTEXT_CHARS: usize = 16_000;

static SCHEMA: LazyLock<Validator> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(include_str!("../schema/envelope.v1.json"))
        .expect("the embedded schema is JSON");
    jsonschema::validator_for(&schema).expect("the embedded schema is valid")
});

/// Returns every problem found, as `path: message`.
pub fn validate(envelope: &Value) -> Result<(), Vec<String>> {
    let mut errors: Vec<String> = SCHEMA
        .iter_errors(envelope)
        .map(|e| format!("envelope{}: {e}", e.instance_path()))
        .collect();
    if !errors.is_empty() {
        errors.truncate(20);
        return Err(errors);
    }

    let serialised = envelope["context"].to_string().encode_utf16().count();
    if serialised > CONTEXT_CHARS {
        errors.push(format!(
            "envelope/context: must serialise to at most {CONTEXT_CHARS} characters"
        ));
    }

    let mut ids = HashSet::new();
    for (i, attachment) in envelope["attachments"].as_array().into_iter().flatten().enumerate() {
        if !ids.insert(attachment["id"].as_str().unwrap_or_default()) {
            errors.push(format!("envelope/attachments/{i}/id: is duplicated"));
        }
    }
    for (i, item) in envelope["items"].as_array().into_iter().flatten().enumerate() {
        if let Some(id) = item.get("attachment").and_then(Value::as_str)
            && !ids.contains(id)
        {
            errors.push(format!("envelope/items/{i}/attachment: does not match an attachment"));
        }
    }

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::validate;

    fn sample() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/sample.json")).unwrap()
    }

    fn rejected(change: impl FnOnce(&mut Value), path: &str) {
        let mut envelope = sample();
        change(&mut envelope);
        let errors = validate(&envelope).expect_err(path).join("\n");
        assert!(errors.contains(path), "expected {path} in:\n{errors}");
    }

    #[test]
    fn a_complete_report_is_valid() {
        assert_eq!(validate(&sample()), Ok(()));
    }

    #[test]
    fn a_minimal_report_is_valid() {
        let mut e = sample();
        let o = e.as_object_mut().unwrap();
        o.remove("page");
        o.remove("category");
        o.insert("items".into(), json!([]));
        o.insert("attachments".into(), json!([]));
        o.insert("context".into(), json!({}));
        o.insert("kind".into(), json!("alert"));
        o.insert("fingerprint".into(), json!("disk-full"));
        assert_eq!(validate(&e), Ok(()));
    }

    #[test]
    fn rejects_what_the_client_validator_rejects() {
        rejected(|e| e["extra"] = json!(1), "envelope:");
        rejected(|e| e["v"] = json!(2), "envelope/v");
        rejected(|e| drop(e.as_object_mut().unwrap().remove("project")), "project");
        rejected(|e| e["project"] = json!("a/b"), "envelope/project");
        rejected(|e| e["id"] = json!("not an id"), "envelope/id");
        rejected(|e| e["kind"] = json!("spam"), "envelope/kind");
        rejected(|e| e["category"] = json!("rant"), "envelope/category");
        rejected(|e| e["createdAt"] = json!("2026-11-12 08:00"), "envelope/createdAt");
        rejected(|e| e["comment"] = json!("x".repeat(5001)), "envelope/comment");
        rejected(|e| e["comment"] = json!(5), "envelope/comment");
        rejected(|e| e["context"] = json!([]), "envelope/context");
        rejected(
            |e| {
                let shot = e["items"][1].clone();
                e["items"] = (0..21)
                    .map(|i| {
                        let mut item = shot.clone();
                        item["id"] = json!(format!("i{i}"));
                        item
                    })
                    .collect();
            },
            "envelope/items",
        );
        rejected(|e| e["items"][0]["type"] = json!("note"), "envelope/items/0");
        rejected(
            |e| drop(e["items"][0].as_object_mut().unwrap().remove("target")),
            "envelope/items/0",
        );
        rejected(
            |e| drop(e["items"][1].as_object_mut().unwrap().remove("attachment")),
            "envelope/items/1",
        );
        rejected(|e| e["items"][1]["target"] = e["items"][0]["target"].clone(), "envelope/items/1");
        rejected(|e| e["items"][0]["target"]["rect"]["x"] = json!(1.5), "envelope/items/0");
        rejected(|e| e["items"][0]["target"]["attrs"]["role"] = json!(3), "envelope/items/0");
        rejected(|e| e["attachments"][0]["mime"] = json!("image/svg+xml"), "attachments/0/mime");
        rejected(
            |e| e["attachments"][0]["data"] = json!("<svg onload=alert(1)>"),
            "attachments/0/data",
        );
        rejected(|e| e["attachments"][0]["width"] = json!(0), "attachments/0/width");
        rejected(
            |e| e["attachments"][0]["data"] = json!("A".repeat(2_800_004)),
            "attachments/0/data",
        );
        rejected(|e| e["page"]["cookies"] = json!("a=b"), "envelope/page");
    }

    #[test]
    fn rejects_what_the_schema_cannot_express() {
        rejected(|e| e["items"][0]["attachment"] = json!("ghost"), "does not match an attachment");
        rejected(|e| e["attachments"][1]["id"] = json!("att-1"), "is duplicated");
        rejected(|e| e["context"] = json!({ "blob": "x".repeat(16_000) }), "must serialise");
    }

    #[test]
    fn rejects_values_that_are_not_objects() {
        for value in [json!(null), json!("text"), json!(42), json!([])] {
            assert!(validate(&value).is_err());
        }
    }
}
