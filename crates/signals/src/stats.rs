//! Small statistics helpers.

/// Standard normal CDF (Abramowitz & Stegun 7.1.26 erf, |error| < 1.5e-7).
pub fn normal_cdf(z: f64) -> f64 {
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly =
        t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    if z >= 0.0 { 0.5 * (1.0 + erf) } else { 0.5 * (1.0 - erf) }
}

/// Mann–Whitney U test of `a` vs `b`: returns (P(a > b) + ½·P(a = b), two-sided
/// p-value from the normal approximation with tie correction).
pub fn mann_whitney(a: &[f64], b: &[f64]) -> Option<(f64, f64)> {
    let (n1, n2) = (a.len() as f64, b.len() as f64);
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let mut u = 0.0;
    for x in a {
        for y in b {
            if (x - y).abs() < 1e-9 {
                u += 0.5;
            } else if x > y {
                u += 1.0;
            }
        }
    }
    let auc = u / (n1 * n2);
    // Tie correction from the pooled ranks.
    let mut all: Vec<f64> = a.iter().chain(b).copied().collect();
    all.sort_by(f64::total_cmp);
    let n = n1 + n2;
    let mut ties = 0.0;
    let mut i = 0;
    while i < all.len() {
        let mut j = i;
        while j + 1 < all.len() && (all[j + 1] - all[i]).abs() < 1e-9 {
            j += 1;
        }
        let t = (j - i + 1) as f64;
        ties += t * t * t - t;
        i = j + 1;
    }
    let var = n1 * n2 / 12.0 * ((n + 1.0) - ties / (n * (n - 1.0)));
    if var <= 0.0 {
        return Some((auc, 1.0));
    }
    let z = (u - n1 * n2 / 2.0) / var.sqrt();
    Some((auc, (2.0 * (1.0 - normal_cdf(z.abs()))).min(1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdf() {
        assert!((normal_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((normal_cdf(1.96) - 0.975).abs() < 1e-4);
        assert!((normal_cdf(-1.0) - 0.158_655).abs() < 1e-5);
    }

    #[test]
    fn mann_whitney_separated_and_identical() {
        let a: Vec<f64> = (10..20).map(f64::from).collect();
        let b: Vec<f64> = (0..10).map(f64::from).collect();
        let (auc, p) = mann_whitney(&a, &b).expect("some");
        assert_eq!(auc, 1.0);
        // Exact value for n1 = n2 = 10 fully separated: z ≈ 3.78.
        assert!(p < 0.001, "{p}");
        let (auc, p) = mann_whitney(&[1.0, 1.0, 1.0], &[1.0, 1.0]).expect("some");
        assert_eq!((auc, p), (0.5, 1.0));
    }
}
