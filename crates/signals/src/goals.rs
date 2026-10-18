//! Focus-goal progress: how the games since a goal started measure up.

use serde::Serialize;

use crate::Comparator;

#[derive(Debug, Clone, Serialize)]
pub struct GoalGame {
    pub match_id: String,
    pub game_start_ms: i64,
    pub champion_name: String,
    pub champion_id: i32,
    pub win: bool,
    pub value: f64,
    /// This game alone met the target.
    pub met: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub games_played: usize,
    pub games_total: u32,
    /// Average over the games played so far.
    pub average: Option<f64>,
    pub games_met: usize,
    /// The average meets the target (so far).
    pub on_track: Option<bool>,
    /// All games played.
    pub done: bool,
    /// Average change vs the baseline, signed so positive = better.
    pub improvement: Option<f64>,
    /// Oldest first.
    pub games: Vec<GoalGame>,
}

/// `games` are the counted games since the goal started, oldest first (any
/// beyond `total` are ignored). `higher_is_better` orients `improvement`.
pub fn progress(
    mut games: Vec<GoalGame>,
    comparator: Comparator,
    target: f64,
    total: u32,
    baseline: Option<f64>,
    higher_is_better: Option<bool>,
) -> Progress {
    games.truncate(total as usize);
    for g in &mut games {
        g.met = comparator.met(g.value, target);
    }
    let n = games.len();
    let average = (n > 0).then(|| games.iter().map(|g| g.value).sum::<f64>() / n as f64);
    let higher = higher_is_better.unwrap_or(comparator == Comparator::AtLeast);
    Progress {
        games_played: n,
        games_total: total,
        average,
        games_met: games.iter().filter(|g| g.met).count(),
        on_track: average.map(|a| comparator.met(a, target)),
        done: n >= total as usize,
        improvement: average.zip(baseline).map(|(a, b)| if higher { a - b } else { b - a }),
        games,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(i: usize, value: f64) -> GoalGame {
        GoalGame {
            match_id: format!("NA1_{i}"),
            game_start_ms: i as i64,
            champion_name: "Leblanc".into(),
            champion_id: 7,
            win: true,
            value,
            met: false,
        }
    }

    #[test]
    fn at_most_goal() {
        // Deaths before 14:00 at most 1 over 5 games; baseline 2.5.
        let p = progress(vec![g(1, 0.0), g(2, 2.0), g(3, 1.0)], Comparator::AtMost, 1.0, 5, Some(2.5), Some(false));
        assert_eq!(p.games_played, 3);
        assert_eq!(p.average, Some(1.0));
        assert_eq!(p.games_met, 2);
        assert_eq!(p.on_track, Some(true));
        assert!(!p.done);
        assert_eq!(p.improvement, Some(1.5));
    }

    #[test]
    fn at_least_goal_done_and_truncated() {
        let games = (0..12).map(|i| g(i, 60.0 + i as f64)).collect();
        let p = progress(games, Comparator::AtLeast, 70.0, 10, Some(66.0), Some(true));
        assert!(p.done);
        assert_eq!(p.games_played, 10);
        assert_eq!(p.average, Some(64.5));
        assert_eq!(p.on_track, Some(false));
        assert_eq!(p.games_met, 0);
        assert_eq!(p.improvement, Some(-1.5));
        let empty = progress(vec![], Comparator::AtLeast, 70.0, 10, None, Some(true));
        assert_eq!((empty.average, empty.on_track, empty.done), (None, None, false));
    }
}
