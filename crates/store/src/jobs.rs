//! Persistent job queue primitives. Job semantics (kinds, payloads, retry
//! policy) live in `rc-jobs`; this module only moves rows between states.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::Result;

#[derive(Debug, Clone)]
pub struct NewJob {
    pub kind: String,
    pub payload: String,
    pub priority: i32,
    pub dedupe_key: String,
    pub run_at_ms: i64,
    pub max_attempts: i32,
}

#[derive(Debug, Clone)]
pub struct ClaimedJob {
    pub id: i64,
    pub kind: String,
    pub payload: String,
    pub dedupe_key: String,
    /// Includes the current attempt (1 on first run).
    pub attempts: i32,
    pub max_attempts: i32,
}

/// Adds a job. If one with the same dedupe key is queued, nothing happens;
/// if one is running, it runs again once it finishes (its inputs may have
/// changed since it started). If it already finished or failed, it's
/// re-queued fresh. Returns whether anything changed.
pub fn enqueue(conn: &Connection, job: &NewJob, now: i64) -> Result<bool> {
    let changed = conn.execute(
        "INSERT INTO jobs (kind, payload, status, priority, attempts, max_attempts, next_run_at, dedupe_key,
                           created_at, updated_at)
         VALUES (?1, ?2, 'queued', ?3, 0, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT (dedupe_key) DO UPDATE SET
             status = 'queued', attempts = 0, last_error = NULL,
             kind = excluded.kind, payload = excluded.payload, priority = excluded.priority,
             max_attempts = excluded.max_attempts, next_run_at = excluded.next_run_at,
             updated_at = excluded.updated_at
         WHERE jobs.status IN ('done', 'failed')",
        params![job.kind, job.payload, job.priority, job.max_attempts, job.run_at_ms, job.dedupe_key, now],
    )?;
    if changed > 0 {
        return Ok(true);
    }
    let rerun = conn.execute(
        "UPDATE jobs SET rerun = 1, payload = ?2, updated_at = ?3
          WHERE dedupe_key = ?1 AND status = 'running' AND rerun = 0",
        params![job.dedupe_key, job.payload, now],
    )?;
    Ok(rerun > 0)
}

/// A job with this dedupe key is queued or running.
pub fn is_active(conn: &Connection, dedupe_key: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM jobs WHERE dedupe_key = ?1 AND status IN ('queued', 'running'))",
        [dedupe_key],
        |r| r.get(0),
    )?)
}

/// Atomically marks the highest-priority due job as running and returns it.
pub fn claim_next(conn: &Connection, now: i64) -> Result<Option<ClaimedJob>> {
    Ok(conn
        .query_row(
            "UPDATE jobs SET status = 'running', attempts = attempts + 1, updated_at = ?1
              WHERE id = (SELECT id FROM jobs WHERE status = 'queued' AND next_run_at <= ?1
                          ORDER BY priority DESC, next_run_at, id LIMIT 1)
             RETURNING id, kind, payload, dedupe_key, attempts, max_attempts",
            [now],
            |r| {
                Ok(ClaimedJob {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    payload: r.get(2)?,
                    dedupe_key: r.get(3)?,
                    attempts: r.get(4)?,
                    max_attempts: r.get(5)?,
                })
            },
        )
        .optional()?)
}

pub fn complete(conn: &Connection, id: i64, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE jobs SET status = CASE WHEN rerun = 1 THEN 'queued' ELSE 'done' END,
                         attempts = CASE WHEN rerun = 1 THEN 0 ELSE attempts END,
                         next_run_at = CASE WHEN rerun = 1 THEN ?2 ELSE next_run_at END,
                         rerun = 0, last_error = NULL, updated_at = ?2
          WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

pub fn retry_later(conn: &Connection, id: i64, run_at: i64, error: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE jobs SET status = 'queued', next_run_at = ?2, last_error = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, run_at, error, now],
    )?;
    Ok(())
}

/// Puts a job back without counting the attempt (e.g. the key was rejected,
/// which isn't the job's fault).
pub fn requeue_without_attempt(conn: &Connection, id: i64, run_at: i64, error: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE jobs SET status = 'queued', attempts = MAX(attempts - 1, 0), next_run_at = ?2, last_error = ?3,
                         updated_at = ?4
          WHERE id = ?1",
        params![id, run_at, error, now],
    )?;
    Ok(())
}

pub fn fail(conn: &Connection, id: i64, error: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE jobs SET status = 'failed', last_error = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, error, now],
    )?;
    Ok(())
}

/// After a crash or restart, anything left `running` goes back to the queue
/// (its attempt isn't counted).
pub fn recover_running(conn: &Connection, now: i64) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE jobs SET status = 'queued', attempts = MAX(attempts - 1, 0), next_run_at = ?1, updated_at = ?1
          WHERE status = 'running'",
        [now],
    )?)
}

/// Earliest time any queued job becomes due.
pub fn next_due_at(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT MIN(next_run_at) FROM jobs WHERE status = 'queued'", [], |r| r.get(0))?)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct JobCounts {
    /// Due now.
    pub ready: i64,
    /// Queued for later (backoff or scheduled).
    pub scheduled: i64,
    pub running: i64,
    pub done: i64,
    pub failed: i64,
}

impl JobCounts {
    pub fn pending(&self) -> i64 {
        self.ready + self.scheduled + self.running
    }
}

pub fn counts(conn: &Connection, now: i64) -> Result<JobCounts> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(status = 'queued' AND next_run_at <= ?1), 0),
                COALESCE(SUM(status = 'queued' AND next_run_at > ?1), 0),
                COALESCE(SUM(status = 'running'), 0),
                COALESCE(SUM(status = 'done'), 0),
                COALESCE(SUM(status = 'failed'), 0)
           FROM jobs",
        [now],
        |r| {
            Ok(JobCounts {
                ready: r.get(0)?,
                scheduled: r.get(1)?,
                running: r.get(2)?,
                done: r.get(3)?,
                failed: r.get(4)?,
            })
        },
    )?)
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedJob {
    pub kind: String,
    pub dedupe_key: String,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

pub fn recent_failures(conn: &Connection, limit: u32) -> Result<Vec<FailedJob>> {
    let mut stmt = conn.prepare(
        "SELECT kind, dedupe_key, last_error, updated_at FROM jobs WHERE status = 'failed'
          ORDER BY updated_at DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok(FailedJob { kind: r.get(0)?, dedupe_key: r.get(1)?, last_error: r.get(2)?, updated_at: r.get(3)? })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    fn job(key: &str, priority: i32, run_at: i64) -> NewJob {
        NewJob {
            kind: "test".into(),
            payload: "{}".into(),
            priority,
            dedupe_key: key.into(),
            run_at_ms: run_at,
            max_attempts: 3,
        }
    }

    #[test]
    fn claims_by_priority_and_due_time() {
        let db = Db::open_in_memory().expect("db");
        db.with(|c| {
            enqueue(c, &job("low", 1, 0), 0)?;
            enqueue(c, &job("high", 9, 0), 0)?;
            enqueue(c, &job("future", 99, 1_000), 0)?;
            assert_eq!(claim_next(c, 10)?.map(|j| j.dedupe_key), Some("high".into()));
            let next = claim_next(c, 10)?.expect("job");
            assert_eq!(next.dedupe_key, "low");
            assert_eq!(next.attempts, 1);
            assert!(claim_next(c, 10)?.is_none(), "future job not due yet");
            assert_eq!(next_due_at(c)?, Some(1_000));
            assert_eq!(claim_next(c, 1_000)?.map(|j| j.kind), Some("test".into()));
            Ok(())
        })
        .expect("ok");
    }

    #[test]
    fn dedupes_active_jobs_and_requeues_finished_ones() {
        let db = Db::open_in_memory().expect("db");
        db.with(|c| {
            assert!(enqueue(c, &job("k", 0, 0), 0)?);
            assert!(!enqueue(c, &job("k", 0, 0), 0)?, "queued: deduped");
            let j = claim_next(c, 0)?.expect("job");
            assert!(enqueue(c, &job("k", 0, 0), 0)?, "running: marked to run again");
            assert!(!enqueue(c, &job("k", 0, 0), 0)?, "only once");
            complete(c, j.id, 1)?;
            let j = claim_next(c, 1)?.expect("re-run after finishing");
            complete(c, j.id, 1)?;
            assert!(claim_next(c, 1)?.is_none(), "and then done");
            assert!(enqueue(c, &job("k", 0, 0), 2)?, "done: re-queued");
            let j = claim_next(c, 2)?.expect("job");
            assert_eq!(j.attempts, 1, "attempts reset on re-queue");
            Ok(())
        })
        .expect("ok");
    }

    #[test]
    fn retry_fail_and_crash_recovery() {
        let db = Db::open_in_memory().expect("db");
        db.with(|c| {
            enqueue(c, &job("a", 0, 0), 0)?;
            enqueue(c, &job("b", 0, 0), 0)?;
            let a = claim_next(c, 0)?.expect("a");
            retry_later(c, a.id, 500, "boom", 0)?;
            assert_eq!(counts(c, 0)?.scheduled, 1);

            let b = claim_next(c, 0)?.expect("b");
            fail(c, b.id, "permanent", 0)?;
            assert_eq!(recent_failures(c, 5)?[0].last_error.as_deref(), Some("permanent"));

            let a = claim_next(c, 500)?.expect("a again");
            assert_eq!(a.attempts, 2);
            // Simulate a crash mid-job.
            assert_eq!(recover_running(c, 600)?, 1);
            let a = claim_next(c, 600)?.expect("a recovered");
            assert_eq!(a.attempts, 2, "crash didn't burn an attempt");
            requeue_without_attempt(c, a.id, 700, "key rejected", 600)?;
            let a = claim_next(c, 700)?.expect("a requeued");
            assert_eq!(a.attempts, 2);
            complete(c, a.id, 700)?;

            let n = counts(c, 700)?;
            assert_eq!((n.ready, n.scheduled, n.running, n.done, n.failed), (0, 0, 0, 1, 1));
            assert_eq!(n.pending(), 0);
            Ok(())
        })
        .expect("ok");
    }
}
