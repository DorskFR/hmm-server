//! Scrubs secrets from the text of a report before it is stored. Mirrors the client's rules, so a
//! report from a client that did not scrub is stored the same way.

use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::Value;
use url::Url;

static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
            "[redacted-private-key]",
        ),
        (r"\beyJ[\w-]{8,}\.[\w-]{8,}\.[\w-]{8,}", "[redacted-jwt]"),
        (r"(?i)\b(Bearer|Basic)\s+[\w.~+/=-]{8,}", "$1 [redacted]"),
        (r"\b(?:sk|pk|rk)-[\w-]{20,}", "[redacted-key]"),
        (r"\bgh[pousr]_\w{30,}", "[redacted-key]"),
        (r"\bgithub_pat_\w{30,}", "[redacted-key]"),
        (r"\bxox[abposr]-[\w-]{10,}", "[redacted-key]"),
        (r"\bAKIA[0-9A-Z]{16}\b", "[redacted-key]"),
    ]
    .into_iter()
    .map(|(pattern, replacement)| (Regex::new(pattern).expect("a valid rule"), replacement))
    .collect()
});

static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)([\w-]*(?:pass(?:word|wd)?|secret|token|api[_-]?key|access[_-]?key|authorization))(["']?\s*[:=]\s*["']?)([^\s"'&,;]{4,})"#,
    )
    .expect("a valid rule")
});

static SECRET_PARAM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)token|key|secret|pass|code|sig|auth|session").expect("a valid rule")
});

#[must_use]
pub fn text(input: &str) -> String {
    let mut out = input.to_owned();
    for (pattern, replacement) in RULES.iter() {
        out = pattern.replace_all(&out, *replacement).into_owned();
    }
    ASSIGNMENT
        .replace_all(&out, |caps: &Captures| {
            let value = caps[3].trim_start_matches('[');
            if value.starts_with("redacted") {
                caps[0].to_owned()
            } else {
                format!("{}{}[redacted]", &caps[1], &caps[2])
            }
        })
        .into_owned()
}

#[must_use]
pub fn url(input: &str) -> String {
    let Ok(mut parsed) = Url::parse(input) else { return text(input) };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    if parsed.query().is_some() {
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(name, value)| {
                let value = if SECRET_PARAM.is_match(&name) {
                    "redacted".into()
                } else {
                    value.into_owned()
                };
                (name.into_owned(), value)
            })
            .collect();
        parsed.query_pairs_mut().clear().extend_pairs(pairs);
    }
    if let Some(fragment) = parsed.fragment().filter(|f| f.contains('=')).map(text) {
        parsed.set_fragment(Some(&fragment));
    }
    text(parsed.as_str())
}

fn deep(value: &mut Value) {
    match value {
        Value::String(s) => *s = text(s),
        Value::Array(list) => list.iter_mut().for_each(deep),
        Value::Object(map) => map.values_mut().for_each(deep),
        _ => {}
    }
}

fn scrub_url(holder: &mut Value) {
    if let Some(Value::String(s)) = holder.get_mut("url") {
        *s = url(s);
    }
}

/// Scrubs every string of a valid report except the attachments, which are images.
pub fn envelope(envelope: &mut Value) {
    let attachments = envelope["attachments"].take();
    if let Some(page) = envelope.get_mut("page") {
        scrub_url(page);
    }
    envelope["items"].as_array_mut().into_iter().flatten().for_each(scrub_url);
    deep(envelope);
    envelope["attachments"] = attachments;
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    const JWT: &str =
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r";

    #[test]
    fn redacts_tokens_keys_and_credentials_in_free_text() {
        let cases = [
            (format!("my session is {JWT} ok"), "my session is [redacted-jwt] ok"),
            (
                "Authorization: Bearer abcdef1234567890".into(),
                "Authorization: [redacted] [redacted]",
            ),
            ("key sk-abcdefghijklmnopqrstuvwx here".into(), "key [redacted-key] here"),
            ("ghp_abcdefghijklmnopqrstuvwxyz0123456789".into(), "[redacted-key]"),
            ("AKIAIOSFODNN7EXAMPLE".into(), "[redacted-key]"),
            ("password=hunter2secret".into(), "password=[redacted]"),
            (r#"{"access_token": "abcd1234"}"#.into(), r#"{"access_token": "[redacted]"}"#),
            ("API_KEY = zzzz9999".into(), "API_KEY = [redacted]"),
            (
                "-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----".into(),
                "[redacted-private-key]",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(super::text(&input), expected);
        }
    }

    #[test]
    fn leaves_ordinary_text_alone_and_is_stable_when_applied_twice() {
        let plain = "The author said the checkout button is hard to find: move it up, please.";
        assert_eq!(super::text(plain), plain);
        let once = super::text("token=abcd1234 and password: hunter2");
        assert_eq!(super::text(&once), once);
    }

    #[test]
    fn redacts_credentials_and_secret_query_parameters_in_urls() {
        assert_eq!(
            super::url("https://user:pw@example.com/a?access_token=abc&q=shoes&code=xyz#top"),
            "https://example.com/a?access_token=redacted&q=shoes&code=redacted#top"
        );
        assert_eq!(
            super::url("https://example.com/cb#id_token=abcdefgh"),
            "https://example.com/cb#id_token=[redacted]"
        );
        assert_eq!(super::url("not a url token=abcd1234"), "not a url token=[redacted]");
    }

    #[test]
    fn scrubs_every_string_of_a_report_except_attachment_data() {
        let mut e: Value =
            serde_json::from_str(include_str!("../tests/fixtures/sample.json")).unwrap();
        let png = e["attachments"][0]["data"].clone();
        e["comment"] = json!("token=abcd1234");
        e["context"] = json!({ "deep": { "list": ["password=hunter2secret"] } });
        e["items"][0]["comment"] = json!(format!("see {JWT}"));
        e["items"][0]["url"] = json!("https://example.com/?api_key=abc");
        e["page"]["url"] = json!("https://example.com/?session=abc&q=1");

        super::envelope(&mut e);

        assert_eq!(e["comment"], "token=[redacted]");
        assert_eq!(e["context"], json!({ "deep": { "list": ["password=[redacted]"] } }));
        assert_eq!(e["items"][0]["comment"], "see [redacted-jwt]");
        assert_eq!(e["items"][0]["url"], "https://example.com/?api_key=redacted");
        assert_eq!(e["page"]["url"], "https://example.com/?session=redacted&q=1");
        assert_eq!(e["attachments"][0]["data"], png);
    }
}
