//! Win-correlation: which of *your* metrics differ between your wins and
//! your losses. Correlation, not causation: metrics measured late in the
//! game mostly reflect who was winning, so early-game metrics (things you
//! control before the game is decided) are flagged separately.

use rc_metrics::defs::{METRICS, format_value, lower_label};
use rc_metrics::percentile::median;
use serde::Serialize;

use crate::Game;
use crate::stats::mann_whitney;

/// Games needed, and wins and losses each, before correlating.
pub const MIN_GAMES: usize = 12;
pub const MIN_EACH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStrength {
    None,
    Weak,
    Moderate,
    Strong,
}

#[derive(Debug, Clone, Serialize)]
pub struct WinLink {
    pub metric: &'static str,
    pub label: &'static str,
    /// Measured by ~15:00, before most games are decided.
    pub early: bool,
    pub games: usize,
    pub wins: usize,
    /// Share of (win, loss) game pairs where the metric was higher in the
    /// win, 0–100 (50 = no link).
    pub higher_in_wins_pct: f64,
    pub win_median: f64,
    pub loss_median: f64,
    /// Your win rate when above / below your own median (ties excluded).
    pub win_rate_above: Option<f64>,
    pub win_rate_below: Option<f64>,
    pub p_value: f64,
    pub strength: LinkStrength,
    /// e.g. "In wins your median CS at 10:00 was 62, in losses 51."
    pub summary: String,
}

fn is_early(key: &str) -> bool {
    key.ends_with("_10")
        || key.ends_with("_15")
        || matches!(key, "deaths_pre14" | "first_back_min" | "first_back_gold" | "first_item_min")
}

fn strength(effect: f64, p: f64) -> LinkStrength {
    if p < 0.01 && effect >= 15.0 {
        LinkStrength::Strong
    } else if p < 0.05 {
        LinkStrength::Moderate
    } else if p < 0.2 {
        LinkStrength::Weak
    } else {
        LinkStrength::None
    }
}

/// Every metric with enough data, strongest link first.
pub fn win_links(games: &[Game]) -> Vec<WinLink> {
    let mut out = Vec::new();
    for d in METRICS {
        let values: Vec<(f64, bool)> = games.iter().filter_map(|g| Some((g.metric(d.key)?, g.win))).collect();
        let wins: Vec<f64> = values.iter().filter(|v| v.1).map(|v| v.0).collect();
        let losses: Vec<f64> = values.iter().filter(|v| !v.1).map(|v| v.0).collect();
        if values.len() < MIN_GAMES || wins.len() < MIN_EACH || losses.len() < MIN_EACH {
            continue;
        }
        let Some((auc, p)) = mann_whitney(&wins, &losses) else { continue };
        let all: Vec<f64> = values.iter().map(|v| v.0).collect();
        let med = median(&all);
        let rate = |keep: &dyn Fn(f64) -> bool| {
            let side: Vec<&(f64, bool)> = values.iter().filter(|v| keep(v.0)).collect();
            (side.len() >= 3).then(|| (side.iter().filter(|v| v.1).count() as f64 / side.len() as f64 * 100.0).round())
        };
        let (wm, lm) = (median(&wins), median(&losses));
        let higher = (auc * 100.0).round();
        out.push(WinLink {
            metric: d.key,
            label: d.label,
            early: is_early(d.key),
            games: values.len(),
            wins: wins.len(),
            higher_in_wins_pct: higher,
            win_median: wm,
            loss_median: lm,
            win_rate_above: rate(&|v| v > med + 1e-9),
            win_rate_below: rate(&|v| v < med - 1e-9),
            p_value: p,
            strength: strength((higher - 50.0).abs(), p),
            summary: format!(
                "In wins your median {} was {}, in losses {}.",
                lower_label(d.label),
                format_value(d, wm),
                format_value(d, lm)
            ),
        });
    }
    out.sort_by(|a, b| {
        b.strength
            .cmp(&a.strength)
            .then_with(|| (b.higher_in_wins_pct - 50.0).abs().total_cmp(&(a.higher_in_wins_pct - 50.0).abs()))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::game;

    #[test]
    fn finds_the_metric_that_separates_wins() {
        // CS@10 is high in every win and low in every loss; vision is noise.
        let games: Vec<Game> = (0..20)
            .map(|i| {
                let win = i % 2 == 0;
                game(
                    i,
                    win,
                    &[("cs_10", if win { 70.0 } else { 50.0 } + i as f64 % 4.0), ("vision_pm", (i % 5) as f64)],
                )
            })
            .collect();
        let links = win_links(&games);
        let cs = &links[0];
        assert_eq!(cs.metric, "cs_10");
        assert!(cs.early);
        assert_eq!(cs.higher_in_wins_pct, 100.0);
        assert_eq!(cs.strength, LinkStrength::Strong);
        assert_eq!((cs.win_rate_above, cs.win_rate_below), (Some(100.0), Some(0.0)));
        let vision = links.iter().find(|l| l.metric == "vision_pm").expect("vision");
        assert!(!vision.early);
        assert!(vision.strength <= LinkStrength::Weak, "{vision:?}");
    }

    #[test]
    fn needs_enough_wins_and_losses() {
        let games: Vec<Game> = (0..20).map(|i| game(i, i != 0, &[("cs_10", i as f64)])).collect();
        assert!(win_links(&games).is_empty());
    }
}
