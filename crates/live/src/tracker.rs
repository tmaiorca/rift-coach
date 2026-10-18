//! Game lifecycle from a stream of polls: idle → loading → in game → ended.
//! Pure (time comes in as an argument), so every transition is unit-tested.

use std::time::Duration;

use serde::Serialize;

use crate::client::Observation;
use crate::model::OwnSnapshot;

/// A game whose endpoint vanished without a `GameEnd` event is treated as
/// over after this long (a disconnect shorter than this is the same game).
pub const DISCONNECT_GRACE_MS: i64 = 120_000;
/// Game time going backwards by more than this means a new game.
const NEW_GAME_REWIND_S: f64 = 30.0;

pub const POLL_IN_GAME: Duration = Duration::from_secs(2);
const POLL_ENDED: Duration = Duration::from_secs(5);
const POLL_IDLE_MIN: Duration = Duration::from_secs(2);
const POLL_IDLE_MAX: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// No game running.
    #[default]
    Idle,
    /// The game process answers but has no player data (loading screen,
    /// spectating, replay).
    Loading,
    InGame,
    /// `GameEnd` seen; the end-of-game screen may still be up.
    Ended,
}

impl Phase {
    /// Background work waits while the game is loading or running.
    pub fn is_playing(self) -> bool {
        matches!(self, Self::Loading | Self::InGame)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// The game's own `GameEnd` event.
    GameEnd,
    /// The endpoint disappeared (crash, quit, or a long disconnect).
    Disappeared,
}

impl EndReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GameEnd => "game_end",
            Self::Disappeared => "disappeared",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transition {
    Phase(Phase),
    /// A new game. `started_at_ms` is wall-clock time at game time 0.
    Started {
        started_at_ms: i64,
        snapshot: Box<OwnSnapshot>,
    },
    Ended {
        reason: EndReason,
        last: Box<OwnSnapshot>,
    },
}

#[derive(Debug)]
struct Current {
    last: OwnSnapshot,
    ended: bool,
}

#[derive(Debug)]
pub struct Tracker {
    phase: Phase,
    current: Option<Current>,
    absent_since: Option<i64>,
    absent_streak: u32,
}

impl Default for Tracker {
    fn default() -> Self {
        Self { phase: Phase::Idle, current: None, absent_since: None, absent_streak: 0 }
    }
}

impl Tracker {
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The newest snapshot of the current (or just-ended) game.
    pub fn last(&self) -> Option<&OwnSnapshot> {
        self.current.as_ref().map(|c| &c.last)
    }

    /// How long to wait before the next poll: fast in game, backing off
    /// while nothing is running.
    pub fn next_poll(&self) -> Duration {
        match self.phase {
            Phase::InGame | Phase::Loading => POLL_IN_GAME,
            Phase::Ended => POLL_ENDED,
            Phase::Idle => (POLL_IDLE_MIN * 2u32.saturating_pow(self.absent_streak.min(4))).min(POLL_IDLE_MAX),
        }
    }

    fn set_phase(&mut self, phase: Phase, out: &mut Vec<Transition>) {
        if self.phase != phase {
            self.phase = phase;
            out.push(Transition::Phase(phase));
        }
    }

    pub fn observe(&mut self, now_ms: i64, obs: Observation) -> Vec<Transition> {
        let mut out = Vec::new();
        match obs {
            Observation::Absent => {
                self.absent_streak = self.absent_streak.saturating_add(1);
                match self.phase {
                    Phase::InGame => {
                        let since = *self.absent_since.get_or_insert(now_ms);
                        if now_ms - since >= DISCONNECT_GRACE_MS {
                            self.end(EndReason::Disappeared, &mut out);
                            self.set_phase(Phase::Idle, &mut out);
                        }
                    }
                    Phase::Loading | Phase::Ended => self.set_phase(Phase::Idle, &mut out),
                    Phase::Idle => {}
                }
            }
            Observation::NotReady => {
                self.absent_since = None;
                self.absent_streak = 0;
                if self.phase == Phase::Idle {
                    self.set_phase(Phase::Loading, &mut out);
                }
            }
            Observation::Data(s) => {
                self.absent_since = None;
                self.absent_streak = 0;
                let new_game = match &self.current {
                    None => true,
                    Some(c) if c.ended => {
                        s.game_end.is_none() || s.game_time_s + NEW_GAME_REWIND_S < c.last.game_time_s
                    }
                    Some(c) => s.game_time_s + NEW_GAME_REWIND_S < c.last.game_time_s,
                };
                if new_game {
                    // The previous game ended without us seeing it.
                    self.end(EndReason::Disappeared, &mut out);
                    let started_at_ms = now_ms - (s.game_time_s * 1000.0).round() as i64;
                    out.push(Transition::Started { started_at_ms, snapshot: s.clone() });
                    self.current = Some(Current { last: (*s).clone(), ended: false });
                    self.set_phase(Phase::InGame, &mut out);
                } else if let Some(c) = &mut self.current {
                    c.last = (*s).clone();
                }
                if s.game_end.is_some() {
                    self.end(EndReason::GameEnd, &mut out);
                    self.set_phase(Phase::Ended, &mut out);
                } else if self.phase == Phase::Loading {
                    self.set_phase(Phase::InGame, &mut out);
                }
            }
        }
        out
    }

    fn end(&mut self, reason: EndReason, out: &mut Vec<Transition>) {
        if let Some(c) = &mut self.current
            && !c.ended
        {
            c.ended = true;
            out.push(Transition::Ended { reason, last: Box::new(c.last.clone()) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(t: f64, end: Option<&str>) -> Observation {
        Observation::Data(Box::new(OwnSnapshot {
            game_time_s: t,
            game_end: end.map(str::to_owned),
            ..Default::default()
        }))
    }

    fn kinds(ts: &[Transition]) -> Vec<String> {
        ts.iter()
            .map(|t| match t {
                Transition::Phase(p) => format!("{p:?}"),
                Transition::Started { .. } => "Started".into(),
                Transition::Ended { reason, .. } => format!("Ended({reason:?})"),
            })
            .collect()
    }

    #[test]
    fn normal_game() {
        let mut t = Tracker::default();
        assert!(t.observe(0, Observation::Absent).is_empty());
        assert_eq!(kinds(&t.observe(1_000, Observation::NotReady)), ["Loading"]);
        assert!(t.phase().is_playing());
        let start = t.observe(60_000, data(5.0, None));
        assert_eq!(kinds(&start), ["Started", "InGame"]);
        let Transition::Started { started_at_ms, .. } = &start[0] else { panic!() };
        assert_eq!(*started_at_ms, 55_000);
        assert!(t.observe(62_000, data(7.0, None)).is_empty());
        assert_eq!(t.next_poll(), POLL_IN_GAME);
        assert_eq!(kinds(&t.observe(2_000_000, data(1800.0, Some("Win")))), ["Ended(GameEnd)", "Ended"]);
        assert!(!t.phase().is_playing());
        // End screen keeps answering; nothing new.
        assert!(t.observe(2_005_000, data(1800.0, Some("Win"))).is_empty());
        assert_eq!(kinds(&t.observe(2_030_000, Observation::Absent)), ["Idle"]);
        assert_eq!(t.last().map(|s| s.game_time_s), Some(1800.0));
    }

    #[test]
    fn short_disconnect_is_the_same_game_long_one_ends_it() {
        let mut t = Tracker::default();
        t.observe(0, data(100.0, None));
        assert!(t.observe(10_000, Observation::Absent).is_empty());
        assert!(t.observe(60_000, Observation::Absent).is_empty());
        assert_eq!(t.phase(), Phase::InGame);
        assert!(t.observe(70_000, data(170.0, None)).is_empty(), "reconnected: same game");
        t.observe(80_000, Observation::Absent);
        assert_eq!(
            kinds(&t.observe(80_000 + DISCONNECT_GRACE_MS, Observation::Absent)),
            ["Ended(Disappeared)", "Idle"]
        );
    }

    #[test]
    fn new_game_without_a_gap_ends_the_old_one() {
        let mut t = Tracker::default();
        t.observe(0, data(1500.0, None));
        assert_eq!(kinds(&t.observe(10_000, data(3.0, None))), ["Ended(Disappeared)", "Started"]);
        // And after a normal end, the next game starts cleanly.
        t.observe(20_000, data(900.0, Some("Lose")));
        assert_eq!(kinds(&t.observe(30_000, data(1.0, None))), ["Started", "InGame"]);
    }

    #[test]
    fn idle_polling_backs_off() {
        let mut t = Tracker::default();
        let first = t.next_poll();
        for i in 0..10 {
            t.observe(i, Observation::Absent);
        }
        assert!(t.next_poll() > first);
        assert_eq!(t.next_poll(), POLL_IDLE_MAX);
    }
}
