//! Picks the frames worth sending: a few around each of the player's deaths,
//! around objective fights, and single frames at laning checkpoints, within
//! a per-game budget.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorKind {
    Death,
    Objective,
    /// A fixed game time (lane state, positioning).
    Checkpoint,
}

impl AnchorKind {
    /// Frame offsets from the anchor in ms, most informative first. Before
    /// a death shows the setup; around an objective shows the approach.
    fn offsets(self) -> &'static [i64] {
        match self {
            Self::Death => &[-6_000, -15_000, -1_000],
            Self::Objective => &[-10_000, -25_000, 0],
            Self::Checkpoint => &[0],
        }
    }
}

/// A moment to look at, from the timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Anchor {
    /// Id the facts and the AI reply use (e.g. `death_2`, `objective_1`).
    pub id: String,
    pub kind: AnchorKind,
    pub t_ms: i64,
    /// Lower is picked first (0 = most important).
    pub priority: u8,
}

/// An anchor with the frames picked for it (game times, ascending).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Moment {
    pub anchor: Anchor,
    pub frames: Vec<i64>,
}

/// A frame counts for a target time if it is this close.
const TOLERANCE_MS: i64 = 3_500;
/// Anchors this close to an already picked one would show the same fight.
const OVERLAP_MS: i64 = 20_000;

fn nearest(frames: &[i64], target: i64, latest: i64) -> Option<i64> {
    frames
        .iter()
        .copied()
        .filter(|&t| t <= latest && (t - target).abs() <= TOLERANCE_MS)
        .min_by_key(|&t| (t - target).abs())
}

/// Chooses up to `max_frames` frames. Deaths get up to 3 frames each (fewer
/// when there are many, so objectives still fit), then objectives, then
/// checkpoints. Returns moments in game order.
pub fn select(frames: &[i64], anchors: &[Anchor], max_frames: usize) -> Vec<Moment> {
    let mut order: Vec<&Anchor> = anchors.iter().collect();
    order.sort_by_key(|a| (a.priority, a.t_ms));
    let deaths = anchors.iter().filter(|a| a.kind == AnchorKind::Death).count();
    // Deaths may use two thirds of the budget when there are many.
    let per_death = (max_frames * 2 / 3).checked_div(deaths).map_or(3, |n| n.clamp(1, 3));

    let mut used = 0usize;
    let mut out: Vec<Moment> = Vec::new();
    for a in order {
        if used >= max_frames {
            break;
        }
        if a.kind != AnchorKind::Death && out.iter().any(|m| (m.anchor.t_ms - a.t_ms).abs() < OVERLAP_MS) {
            continue;
        }
        let cap = if a.kind == AnchorKind::Death { per_death } else { a.kind.offsets().len() };
        // Never after a death (the grey screen shows nothing).
        let latest = if a.kind == AnchorKind::Death { a.t_ms } else { i64::MAX };
        let mut picked: Vec<i64> = Vec::new();
        for off in a.kind.offsets().iter().take(cap) {
            if used + picked.len() >= max_frames {
                break;
            }
            if let Some(t) = nearest(frames, a.t_ms + off, latest).filter(|t| !picked.contains(t)) {
                picked.push(t);
            }
        }
        if picked.is_empty() {
            continue;
        }
        picked.sort_unstable();
        used += picked.len();
        out.push(Moment { anchor: a.clone(), frames: picked });
    }
    out.sort_by_key(|m| m.anchor.t_ms);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(id: &str, kind: AnchorKind, t_s: i64, priority: u8) -> Anchor {
        Anchor { id: id.into(), kind, t_ms: t_s * 1000, priority }
    }

    /// A frame every 4 s for 30 minutes.
    fn frames() -> Vec<i64> {
        (0..450).map(|i| i * 4_000).collect()
    }

    #[test]
    fn deaths_first_then_objectives_then_checkpoints() {
        let anchors = vec![
            anchor("checkpoint_1", AnchorKind::Checkpoint, 600, 3),
            anchor("death_1", AnchorKind::Death, 400, 0),
            anchor("objective_1", AnchorKind::Objective, 900, 2),
            // Same fight as death_1: skipped.
            anchor("objective_2", AnchorKind::Objective, 410, 2),
        ];
        let m = select(&frames(), &anchors, 24);
        let ids: Vec<&str> = m.iter().map(|m| m.anchor.id.as_str()).collect();
        assert_eq!(ids, ["death_1", "checkpoint_1", "objective_1"]);
        let death = &m[0].frames;
        assert_eq!(death.len(), 3);
        assert!(death.iter().all(|&t| t <= 400_000), "never after the death");
        assert_eq!(m[2].frames.len(), 3);
        assert_eq!(m[1].frames, vec![600_000]);
    }

    #[test]
    fn budget_spreads_over_many_deaths() {
        let anchors: Vec<Anchor> =
            (0..12).map(|i| anchor(&format!("death_{i}"), AnchorKind::Death, 100 + i * 120, 0)).collect();
        let m = select(&frames(), &anchors, 18);
        let total: usize = m.iter().map(|m| m.frames.len()).sum();
        assert!(total <= 18);
        assert_eq!(m.len(), 12, "every death gets a frame");
        assert!(m.iter().all(|m| m.frames.len() == 1));
    }

    #[test]
    fn no_frames_near_anchor() {
        let anchors = vec![anchor("death_1", AnchorKind::Death, 5000, 0)];
        assert!(select(&frames(), &anchors, 24).is_empty());
        assert!(select(&[], &anchors, 24).is_empty());
    }
}
