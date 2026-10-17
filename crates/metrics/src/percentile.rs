//! Percentile comparisons ("better than X% of ...").

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Comparison {
    /// Share of the population this value beats, 0–100. Ties count half.
    /// For lower-is-better metrics, beating means being lower.
    pub better_than_pct: f64,
    pub median: f64,
    pub n: usize,
}

/// `None` if the population is empty or the metric has no "better" direction.
pub fn compare(value: f64, population: &[f64], higher_is_better: Option<bool>) -> Option<Comparison> {
    let higher = higher_is_better?;
    if population.is_empty() {
        return None;
    }
    let (mut beaten, mut ties) = (0usize, 0usize);
    for &v in population {
        if (v - value).abs() < 1e-9 {
            ties += 1;
        } else if (higher && v < value) || (!higher && v > value) {
            beaten += 1;
        }
    }
    let n = population.len();
    let better_than_pct = (beaten as f64 + ties as f64 / 2.0) / n as f64 * 100.0;
    Some(Comparison { better_than_pct, median: median(population), n })
}

pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) { (v[mid - 1] + v[mid]) / 2.0 } else { v[mid] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_is_better() {
        let pop = [1.0, 2.0, 3.0, 4.0];
        let c = compare(3.5, &pop, Some(true)).expect("some");
        assert_eq!(c.better_than_pct, 75.0);
        assert_eq!(c.median, 2.5);
        assert_eq!(c.n, 4);
    }

    #[test]
    fn lower_is_better_and_ties() {
        let pop = [0.0, 1.0, 1.0, 3.0];
        // Beats the 3, ties two 1s: (1 + 1) / 4.
        assert_eq!(compare(1.0, &pop, Some(false)).expect("some").better_than_pct, 50.0);
    }

    #[test]
    fn no_direction_or_empty() {
        assert!(compare(1.0, &[1.0], None).is_none());
        assert!(compare(1.0, &[], Some(true)).is_none());
    }
}
