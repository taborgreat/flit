//! Per-peer quotas per song: events per second and stored bytes. Over
//! quota = stop accepting from that peer and show it in the panel.

use flit_core::Id;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quota {
    pub events_per_sec: f64,
    pub burst: f64,
    pub storage_bytes: u64,
}

impl Default for Quota {
    fn default() -> Self {
        Self { events_per_sec: 50.0, burst: 500.0, storage_bytes: 4 << 30 }
    }
}

#[derive(Debug, Clone)]
struct Bucket {
    tokens: f64,
    last_ms: u64,
    stored: u64,
    exceeded: bool,
}

#[derive(Debug, Default)]
pub struct QuotaState {
    quota: Quota,
    peers: HashMap<Id, Bucket>,
}

impl QuotaState {
    pub fn new(quota: Quota) -> Self {
        Self { quota, peers: HashMap::new() }
    }

    fn bucket(&mut self, peer: &Id, now_ms: u64) -> &mut Bucket {
        let q = self.quota;
        self.peers.entry(*peer).or_insert(Bucket { tokens: q.burst, last_ms: now_ms, stored: 0, exceeded: false })
    }

    /// One event arrived from `peer`. False = over quota, drop it.
    pub fn allow_event(&mut self, peer: &Id, now_ms: u64) -> bool {
        let q = self.quota;
        let b = self.bucket(peer, now_ms);
        let elapsed = now_ms.saturating_sub(b.last_ms) as f64 / 1000.0;
        b.tokens = (b.tokens + elapsed * q.events_per_sec).min(q.burst);
        b.last_ms = now_ms;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            b.exceeded = true;
            false
        }
    }

    /// `bytes` more from `peer` are about to be stored. False = refuse.
    pub fn allow_storage(&mut self, peer: &Id, bytes: u64, now_ms: u64) -> bool {
        let q = self.quota;
        let b = self.bucket(peer, now_ms);
        if b.stored + bytes > q.storage_bytes {
            b.exceeded = true;
            false
        } else {
            b.stored += bytes;
            true
        }
    }

    pub fn is_exceeded(&self, peer: &Id) -> bool {
        self.peers.get(peer).map(|b| b.exceeded).unwrap_or(false)
    }

    pub fn exceeded_peers(&self) -> Vec<Id> {
        self.peers.iter().filter(|(_, b)| b.exceeded).map(|(id, _)| *id).collect()
    }

    pub fn reset(&mut self, peer: &Id) {
        self.peers.remove(peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_and_storage() {
        let mut q = QuotaState::new(Quota { events_per_sec: 10.0, burst: 20.0, storage_bytes: 1000 });
        let p = flit_core::Keypair::from_seed(1).id();
        let ok = (0..30).filter(|_| q.allow_event(&p, 0)).count();
        assert_eq!(ok, 20);
        assert!(q.is_exceeded(&p));
        q.reset(&p);
        assert!(q.allow_event(&p, 0));
        // refills at 10/s
        for _ in 0..19 {
            q.allow_event(&p, 0);
        }
        assert!(!q.allow_event(&p, 0));
        assert!(q.allow_event(&p, 100));
        q.reset(&p);
        assert!(q.allow_storage(&p, 600, 0));
        assert!(!q.allow_storage(&p, 600, 0));
        assert_eq!(q.exceeded_peers(), vec![p]);
    }
}
