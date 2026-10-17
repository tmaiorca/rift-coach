//! Checks an AI report against the facts it was given: structure, evidence
//! ids, and every number in the text. Structural problems mean the reply is
//! unusable; number problems mean it's stored but flagged as unverified.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::facts::evidence_ids;

#[derive(Debug, Default)]
pub struct Validation {
    /// The reply can't be used (wrong shape, unknown evidence).
    pub structural: Vec<String>,
    /// Numbers in the text that don't trace back to a fact.
    pub untraced_numbers: Vec<String>,
}

impl Validation {
    pub fn is_clean(&self) -> bool {
        self.structural.is_empty() && self.untraced_numbers.is_empty()
    }

    pub fn feedback(&self) -> String {
        let mut lines = Vec::new();
        lines.extend(self.structural.iter().cloned());
        if !self.untraced_numbers.is_empty() {
            lines.push(format!(
                "These numbers do not appear in the facts; remove them or use the exact fact values: {}",
                self.untraced_numbers.join(", ")
            ));
        }
        lines.join("\n")
    }
}

static TIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{1,2}:\d{2}\b").expect("regex"));
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[^\w.])(-?\d[\d,]*(?:\.\d+)?)(k)?").expect("regex"));
/// Counts and minute marks that are always fine ("next 3 games", "at 15 minutes").
const ALWAYS_OK: [f64; 14] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 14.0, 15.0, 20.0];

struct Allowed {
    numbers: Vec<f64>,
    times: HashSet<String>,
}

fn collect(v: &Value, out: &mut Allowed) {
    match v {
        Value::Number(n) => out.numbers.extend(n.as_f64()),
        Value::String(s) => {
            for t in TIME.find_iter(s) {
                out.times.insert(t.as_str().trim_start_matches('0').to_owned());
            }
            for c in NUMBER.captures_iter(&TIME.replace_all(s, " ")) {
                out.numbers.extend(c[1].replace(',', "").parse::<f64>().ok());
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect(x, out)),
        Value::Object(o) => o.values().for_each(|x| collect(x, out)),
        _ => {}
    }
}

fn traces(n: f64, decimal: bool, allowed: &[f64]) -> bool {
    if ALWAYS_OK.contains(&n.abs()) && !decimal {
        return true;
    }
    let tol = if decimal { 0.051 } else { 0.5 };
    allowed.iter().any(|&v| (n - v).abs() <= tol || (n.abs() - v.abs()).abs() <= tol)
}

/// Strings in the report that carry prose (where numbers are checked).
fn texts(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| texts(x, out)),
        Value::Object(o) => {
            for (k, x) in o {
                // A goal may set a new target number ("60 CS by 10:00").
                if !matches!(k.as_str(), "evidence" | "metric" | "confidence" | "goal") {
                    texts(x, out);
                }
            }
        }
        _ => {}
    }
}

pub fn validate(report: &Value, facts: &Value) -> Validation {
    let mut out = Validation::default();
    let ids: HashSet<String> = evidence_ids(facts).into_iter().collect();
    let s = &mut out.structural;

    if !report.is_object() {
        s.push("The reply must be a single JSON object.".into());
        return out;
    }
    if report["summary"].as_str().is_none_or(|x| x.trim().is_empty()) {
        s.push("`summary` must be a non-empty string.".into());
    }
    let check_items = |key: &str, fields: &[&str], min: usize, max: usize, s: &mut Vec<String>| {
        let Some(items) = report[key].as_array() else {
            s.push(format!("`{key}` must be an array."));
            return;
        };
        if items.len() < min || items.len() > max {
            s.push(format!("`{key}` must have {min}-{max} items (has {}).", items.len()));
        }
        for (i, it) in items.iter().enumerate() {
            for f in fields {
                if it[*f].as_str().is_none_or(|x| x.trim().is_empty()) {
                    s.push(format!("`{key}[{i}].{f}` must be a non-empty string."));
                }
            }
            match it["evidence"].as_array() {
                None => s.push(format!("`{key}[{i}].evidence` must be an array of fact ids.")),
                Some(ev) => {
                    for e in ev {
                        let id = e.as_str().unwrap_or_default();
                        if !ids.contains(id) {
                            s.push(format!("`{key}[{i}].evidence` cites unknown fact id \"{id}\"."));
                        }
                    }
                }
            }
        }
    };
    check_items("went_well", &["title", "detail"], 1, 4, s);
    check_items("improve", &["title", "detail"], 1, 4, s);
    check_items("priority_fixes", &["title", "why", "how"], 1, 3, s);
    if report["next_game_focus"]["goal"].as_str().is_none_or(|x| x.trim().is_empty()) {
        s.push("`next_game_focus.goal` must be a non-empty string.".into());
    }
    if let Some(m) = report["next_game_focus"]["metric"].as_str()
        && !m.is_empty()
        && !ids.contains(m)
    {
        s.push(format!("`next_game_focus.metric` \"{m}\" is not a metric id from the facts."));
    }
    if !matches!(report["confidence"].as_str(), Some("low" | "medium" | "high")) {
        s.push("`confidence` must be \"low\", \"medium\" or \"high\".".into());
    }

    let mut allowed = Allowed { numbers: Vec::new(), times: HashSet::new() };
    collect(facts, &mut allowed);
    let mut prose = Vec::new();
    texts(report, &mut prose);
    let mut bad: Vec<String> = Vec::new();
    for text in &prose {
        for t in TIME.find_iter(text) {
            if !allowed.times.contains(t.as_str().trim_start_matches('0'))
                && !["10:00", "14:00", "15:00", "20:00"].contains(&t.as_str())
            {
                bad.push(t.as_str().to_owned());
            }
        }
        for c in NUMBER.captures_iter(&TIME.replace_all(text, " ")) {
            let raw = &c[1];
            let Ok(mut n) = raw.replace(',', "").parse::<f64>() else { continue };
            let mut decimal = raw.contains('.');
            if c.get(2).is_some() {
                n *= 1000.0;
                decimal = false;
            }
            if !traces(n, decimal, &allowed.numbers) {
                bad.push(format!("{raw}{}", c.get(2).map_or("", |k| k.as_str())));
            }
        }
    }
    bad.sort();
    bad.dedup();
    out.untraced_numbers = bad;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts() -> Value {
        json!({
            "match": {"duration": "27:45", "kills": 6},
            "metrics": [
                {"id": "cs_10", "value": 49.0, "vs_role": {"better_than_pct": 31.0, "median": 62.0, "n": 940}},
                {"id": "gold_diff_10", "value": 519.0},
                {"id": "kda", "value": 1.29}
            ],
            "deaths": [{"id": "death_1", "time": "4:12"}]
        })
    }

    fn report(detail: &str) -> Value {
        json!({
            "summary": "A close game.",
            "went_well": [{"title": "Gold lead", "detail": "You were 519 gold ahead at 10:00.", "evidence": ["gold_diff_10"]}],
            "improve": [{"title": "CS", "detail": detail, "evidence": ["cs_10", "death_1"]}],
            "priority_fixes": [{"title": "Wave control", "why": "x", "how": "y", "evidence": ["cs_10"]}],
            "next_game_focus": {"goal": "Hit 60 CS by 10:00 in the next 3 games", "metric": "cs_10"},
            "confidence": "medium",
            "confidence_reason": "One game."
        })
    }

    #[test]
    fn accepts_traceable_numbers() {
        let v = validate(
            &report(
                "49 CS at 10 minutes is better than 31% of mid laners; median 62 (n=940). KDA 1.29. Died at 04:12.",
            ),
            &facts(),
        );
        // "60" in the goal is a target, not a claim, so it isn't checked.
        assert!(v.is_clean(), "{v:?}");
    }

    #[test]
    fn flags_invented_numbers_and_times() {
        let v = validate(&report("You had 2.5k gold unspent and lost 73% of fights at 12:30."), &facts());
        assert!(v.untraced_numbers.contains(&"2.5k".to_string()));
        assert!(v.untraced_numbers.contains(&"73".to_string()));
        assert!(v.untraced_numbers.contains(&"12:30".to_string()));
    }

    #[test]
    fn rejects_bad_structure_and_unknown_evidence() {
        let mut r = report("fine");
        r["improve"][0]["evidence"] = json!(["made_up_metric"]);
        r["confidence"] = json!("very");
        let v = validate(&r, &facts());
        assert_eq!(v.structural.len(), 2, "{:?}", v.structural);
    }
}
