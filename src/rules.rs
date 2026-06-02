use serde_json::{Map, Value};

/// Match semantics:
/// - anyOf: OR of conditions — matches if any listed condition passes.
/// - allOf: AND of conditions — matches if all listed conditions pass.
/// - If both are given, the rule matches when either clause is satisfied.
/// - An empty match object `{}` (neither clause) is a deliberate catch-all.
pub fn rule_matches(match_json: &Value, facts: &Map<String, Value>) -> bool {
    let any_clause = match_json.get("anyOf").and_then(|v| v.as_array());
    let all_clause = match_json.get("allOf").and_then(|v| v.as_array());

    // No conditions at all (e.g. "{}") is a deliberate catch-all.
    if any_clause.is_none() && all_clause.is_none() {
        return true;
    }

    // A clause only contributes when it is present AND satisfied. In particular,
    // an absent allOf must NOT default to true — otherwise an anyOf-only rule
    // would match every request (and e.g. a blackout rule would drop all traffic).
    let any_ok = any_clause.map_or(false, |arr| arr.iter().any(|c| eval(c, facts)));
    let all_ok = all_clause.map_or(false, |arr| arr.iter().all(|c| eval(c, facts)));

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

/// Glob match supporting any number of `*` wildcards, each matching any
/// (possibly empty) run of characters. Examples:
///   "blk-*"   prefix      "*-end"   suffix
///   "*AFE1*"  contains     "a*b*c"   ordered segments
/// With no `*`, this is an exact-equality test.
fn glob_match(pat: &str, text: &str) -> bool {
    if !pat.contains('*') {
        return pat == text;
    }
    let parts: Vec<&str> = pat.split('*').collect();
    let last = parts.len() - 1;
    let mut pos = 0usize; // how far we have consumed `text`
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue; // adjacent/edge '*' matches anything
        }
        if i == 0 {
            // No leading '*': the first segment must anchor at the start.
            if !text[pos..].starts_with(part) {
                return false;
            }
            pos += part.len();
        } else if i == last {
            // No trailing '*': the final segment must anchor at the end.
            return text[pos..].ends_with(part);
        } else {
            // Interior segment: must appear somewhere after the current position.
            match text[pos..].find(part) {
                Some(idx) => pos += idx + part.len(),
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts(sig: &str) -> Map<String, Value> {
        json!({ "acquisitionSignalID": sig })
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn empty_match_is_catch_all() {
        assert!(rule_matches(&json!({}), &facts("anything")));
    }

    #[test]
    fn anyof_only_matches_only_when_a_condition_passes() {
        let m = json!({ "anyOf": [{ "acquisitionSignalID": "blk-*" }] });
        assert!(rule_matches(&m, &facts("blk-001")));
        // Regression: an anyOf-only rule must NOT match everything.
        assert!(!rule_matches(&m, &facts("news-ad-01")));
    }

    #[test]
    fn allof_requires_every_condition() {
        let m = json!({ "allOf": [{ "acquisitionSignalID": "blk-*" }] });
        assert!(rule_matches(&m, &facts("blk-001")));
        assert!(!rule_matches(&m, &facts("other")));
    }

    #[test]
    fn anyof_is_or_across_conditions() {
        let m = json!({ "anyOf": [
            { "acquisitionSignalID": "blk-*" },
            { "acquisitionSignalID": "stop-*" }
        ]});
        assert!(rule_matches(&m, &facts("stop-9")));
        assert!(!rule_matches(&m, &facts("go-9")));
    }

    #[test]
    fn glob_supports_prefix_suffix_contains_and_exact() {
        // exact
        assert!(glob_match("blk-001", "blk-001"));
        assert!(!glob_match("blk-001", "blk-002"));
        // prefix / suffix
        assert!(glob_match("blk-*", "blk-evening"));
        assert!(!glob_match("blk-*", "news"));
        assert!(glob_match("*-end", "program-end"));
        // contains (multi-*) — the case that previously could not match
        assert!(glob_match("*AFE1*", "0x09:41442DAFE1303031"));
        assert!(glob_match("*AFE1*", "AD-AFE1-001"));
        assert!(!glob_match("*AFE1*", "AD-AFE2-001"));
        // ordered interior segments
        assert!(glob_match("a*b*c", "a__b__c"));
        assert!(!glob_match("a*b*c", "a__c__b"));
        // bare "*" matches anything
        assert!(glob_match("*", "anything"));
    }
}
