//! Measures the app's own CPU and memory while a game runs: this process
//! (the backend) plus its child processes (the WebView2 window). Memory is
//! private memory (on Windows, `PrivateUsage`): summing working sets would
//! count the DLL pages WebView2's processes share once per process.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Budget while a game runs, as a share of the whole machine's CPU and in
/// private memory. Shown next to each game's measured footprint.
pub const BUDGET_CPU_AVG_PCT: f64 = 1.0;
pub const BUDGET_MEM_MB: f64 = 400.0;
/// The process list is re-scanned for new window processes this often.
const RESCAN_EVERY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Sample {
    /// Share of total CPU capacity (all cores), 0–100.
    pub cpu_pct: f64,
    /// Backend process only.
    pub backend_cpu_pct: f64,
    pub mem_mb: f64,
    pub backend_mem_mb: f64,
}

/// Running totals for one game.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Footprint {
    pub samples: u32,
    pub cpu_avg_pct: f64,
    pub cpu_max_pct: f64,
    pub backend_cpu_avg_pct: f64,
    pub mem_max_mb: f64,
    pub backend_mem_max_mb: f64,
}

impl Footprint {
    pub fn add(&mut self, s: Sample) {
        let n = f64::from(self.samples);
        self.cpu_avg_pct = (self.cpu_avg_pct * n + s.cpu_pct) / (n + 1.0);
        self.backend_cpu_avg_pct = (self.backend_cpu_avg_pct * n + s.backend_cpu_pct) / (n + 1.0);
        self.cpu_max_pct = self.cpu_max_pct.max(s.cpu_pct);
        self.mem_max_mb = self.mem_max_mb.max(s.mem_mb);
        self.backend_mem_max_mb = self.backend_mem_max_mb.max(s.backend_mem_mb);
        self.samples += 1;
    }

    pub fn within_budget(&self) -> bool {
        self.cpu_avg_pct <= BUDGET_CPU_AVG_PCT && self.mem_max_mb <= BUDGET_MEM_MB
    }
}

pub struct Sampler {
    sys: System,
    me: Pid,
    tree: Vec<Pid>,
    scanned: Option<Instant>,
    cores: f64,
    primed: bool,
}

impl Sampler {
    pub fn new() -> Option<Self> {
        let me = sysinfo::get_current_pid().ok()?;
        let cores = std::thread::available_parallelism().map_or(1.0, |n| n.get() as f64);
        Some(Self { sys: System::new(), me, tree: vec![me], scanned: None, cores, primed: false })
    }

    /// This process and every descendant (WebView2 spawns a process tree).
    fn rescan(&mut self) {
        self.sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
        let mut tree = vec![self.me];
        let mut i = 0;
        while i < tree.len() {
            let parent = tree[i];
            tree.extend(self.sys.processes().iter().filter(|(_, p)| p.parent() == Some(parent)).map(|(pid, _)| *pid));
            i += 1;
        }
        self.tree = tree;
        self.scanned = Some(Instant::now());
    }

    /// One reading. The first call only primes CPU counters and returns `None`.
    pub fn sample(&mut self) -> Option<Sample> {
        if self.scanned.is_none_or(|t| t.elapsed() >= RESCAN_EVERY) {
            self.rescan();
            self.primed = false;
        }
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&self.tree),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        if !self.primed {
            self.primed = true;
            return None;
        }
        let mut s = Sample::default();
        for pid in &self.tree {
            let Some(p) = self.sys.process(*pid) else { continue };
            let cpu = f64::from(p.cpu_usage()) / self.cores;
            let mem = p.virtual_memory() as f64 / (1024.0 * 1024.0);
            s.cpu_pct += cpu;
            s.mem_mb += mem;
            if *pid == self.me {
                s.backend_cpu_pct = cpu;
                s.backend_mem_mb = mem;
            }
        }
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_totals() {
        let mut f = Footprint::default();
        f.add(Sample { cpu_pct: 1.0, backend_cpu_pct: 0.2, mem_mb: 200.0, backend_mem_mb: 40.0 });
        f.add(Sample { cpu_pct: 0.0, backend_cpu_pct: 0.0, mem_mb: 300.0, backend_mem_mb: 50.0 });
        assert_eq!(f.samples, 2);
        assert!((f.cpu_avg_pct - 0.5).abs() < 1e-9);
        assert_eq!((f.cpu_max_pct, f.mem_max_mb, f.backend_mem_max_mb), (1.0, 300.0, 50.0));
        assert!(f.within_budget());
    }

    #[test]
    fn measures_this_process() {
        let mut s = Sampler::new().expect("sampler");
        assert!(s.sample().is_none(), "first call primes");
        std::thread::sleep(Duration::from_millis(250));
        let m = s.sample().expect("sample");
        assert!(m.backend_mem_mb > 1.0, "{m:?}");
        assert!(m.mem_mb >= m.backend_mem_mb);
    }
}
