//! Play sessions: consecutive games where each starts within `gap_ms` of
//! the previous one ending.

use std::ops::Range;

/// Groups games into sessions. `spans` are `(start_ms, end_ms)` sorted by
/// start, oldest first; returns index ranges into `spans`, oldest first.
pub fn group(spans: &[(i64, i64)], gap_ms: i64) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..spans.len() {
        if spans[i].0 - spans[i - 1].1 > gap_ms {
            out.push(start..i);
            start = i;
        }
    }
    if !spans.is_empty() {
        out.push(start..spans.len());
    }
    out
}

/// Each game's position in its session (0 = first game).
pub fn session_positions(spans: &[(i64, i64)], gap_ms: i64) -> Vec<usize> {
    let mut pos = vec![0; spans.len()];
    for r in group(spans, gap_ms) {
        for (k, i) in r.enumerate() {
            pos[i] = k;
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn splits_on_gaps() {
        // Two games 10 min apart, then one 2 hours later.
        let spans = [(0, 30 * MIN), (40 * MIN, 70 * MIN), (190 * MIN, 220 * MIN)];
        assert_eq!(group(&spans, 60 * MIN), vec![0..2, 2..3]);
        assert_eq!(session_positions(&spans, 60 * MIN), vec![0, 1, 0]);
        assert_eq!(group(&spans, 200 * MIN), vec![0..3]);
        assert!(group(&[], 60 * MIN).is_empty());
    }
}
