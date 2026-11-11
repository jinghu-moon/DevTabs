//! Small shared bounds for task background work. No worker owns UI input locks.
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub bytes: u64,
    pub entries: usize,
    pub time: Duration,
}

pub struct Budget {
    limits: Limits,
    started: Instant,
    bytes: u64,
    entries: usize,
}
impl Budget {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            started: Instant::now(),
            bytes: 0,
            entries: 0,
        }
    }
    pub fn charge(&mut self, bytes: u64) -> bool {
        if self.expired()
            || self.entries >= self.limits.entries
            || bytes > self.limits.bytes.saturating_sub(self.bytes)
        {
            return false;
        }
        self.bytes += bytes;
        self.entries += 1;
        true
    }
    pub fn used(&self) -> (u64, usize) {
        (self.bytes, self.entries)
    }
    pub fn expired(&self) -> bool {
        self.started.elapsed() >= self.limits.time
    }
}

#[derive(Default)]
pub struct RevisionGate {
    pub generation: u64,
    pub revision: u64,
}
impl RevisionGate {
    pub fn accept(&mut self, generation: u64, revision: u64) -> bool {
        if generation != self.generation || revision <= self.revision {
            return false;
        }
        self.revision = revision;
        true
    }
}
