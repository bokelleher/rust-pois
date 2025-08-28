use serde_json::{Map, Value};

/// Match semantics:
/// - anyOf: OR of conditions (optional; default false)
/// - allOf: AND of conditions (optional; default true)
/// If either passes, the rule matches.
pub fn rule_matches(match_json: &Value, facts: &Map<String, Value>) -> bool {
    let any_ok = match_json
        .get("anyOf")
        .and_then(|v| v.as_array())
        .map_or(false, |arr| arr.iter().any(|c| eval(c, facts)));

    let all_ok = match_json
        .get("allOf")
        .and_then(|v| v.as_array())
        .map_or(true, |arr| arr.iter().all(|c| eval(c, facts)));

    any_ok || all_ok
}

fn eval(cond: &Value, facts: &Map<String, Value>) -> bool {
    // acquisitionSignalID glob match (single '*' supported)
    if let Some(pat) = cond.get("acquisitionSignalID").and_then(|v| v.as_str()) {
        if let Some(Value::String(actual)) = facts.get("acquisitionSignalID") {
            return glob_match(pat, actual);
        }
    }

    // scte35.command equals (case-insensitive)
    if let Some(cmd) = cond.get("scte35.command").and_then(|v| v.as_str()) {
        if let Some(Value::String(actual)) = facts.get("scte35.command") {
            return actual.eq_ignore_ascii_case(cmd);
        }
    }

    // NEW: segmentation_type_id equals (e.g., "0x34")
    if let Some(typ) = cond.get("scte35.segmentation_type_id").and_then(|v| v.as_str()) {
        if let Some(Value::String(actual)) = facts.get("scte35.segmentation_type_id") {
            return actual.eq_ignore_ascii_case(typ);
        }
    }

    // NEW: segmentation_upid glob match (ASCII or "hex:..." form)
    if let Some(pat) = cond.get("scte35.segmentation_upid").and_then(|v| v.as_str()) {
        if let Some(Value::String(actual)) = facts.get("scte35.segmentation_upid") {
            return glob_match(pat, actual);
        }
    }

    // utcBetween window (lexicographic on ISO-8601 UTC strings)
    if let Some(win) = cond.get("utcBetween").and_then(|v| v.as_object()) {
        let start = win.get("start").and_then(|v| v.as_str()).unwrap_or("");
        let end   = win.get("end").and_then(|v| v.as_str()).unwrap_or("~"); // '~' > 'Z'
        if let Some(Value::String(utc)) = facts.get("utcPoint") {
            let u = utc.as_str();
            return u >= start && u <= end;
        }
    }

    false
}

fn glob_match(pat: &str, text: &str) -> bool {
    if let Some(i) = pat.find('*') {
        let (pre, post) = pat.split_at(i);
        let post = &post[1..];
        text.starts_with(pre) && text.ends_with(post)
    } else {
        pat == text
    }
}
