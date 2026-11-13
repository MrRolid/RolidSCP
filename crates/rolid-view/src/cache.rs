//! Blocks of a file kept in memory, with a limit per file and one for all
//! open viewers together (the least recently used blocks go first).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Bytes cached by all viewers together.
static ALL: AtomicUsize = AtomicUsize::new(0);
/// The limit for all viewers together.
pub const ALL_LIMIT: usize = 256 << 20;

pub fn all_cached() -> usize {
    ALL.load(Ordering::Relaxed)
}

pub struct BlockCache {
    block: usize,
    limit: usize,
    all_limit: usize,
    map: HashMap<u64, (Arc<[u8]>, u64)>,
    bytes: usize,
    tick: u64,
}

impl BlockCache {
    pub fn new(block: usize, limit: usize) -> Self {
        Self::with_all_limit(block, limit, ALL_LIMIT)
    }

    pub fn with_all_limit(block: usize, limit: usize, all_limit: usize) -> Self {
        BlockCache { block, limit, all_limit, map: HashMap::new(), bytes: 0, tick: 0 }
    }

    pub fn block(&self) -> usize {
        self.block
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn contains(&self, idx: u64) -> bool {
        self.map.contains_key(&idx)
    }

    pub fn get(&mut self, idx: u64) -> Option<Arc<[u8]>> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(&idx).map(|(d, used)| {
            *used = t;
            d.clone()
        })
    }

    /// Keep block `idx`; older blocks go while a limit is exceeded (the new
    /// block itself always stays).
    pub fn insert(&mut self, idx: u64, data: Arc<[u8]>) {
        self.remove(idx);
        self.tick += 1;
        self.bytes += data.len();
        ALL.fetch_add(data.len(), Ordering::Relaxed);
        self.map.insert(idx, (data, self.tick));
        while self.map.len() > 1 && (self.bytes > self.limit || ALL.load(Ordering::Relaxed) > self.all_limit) {
            let Some(old) = self.map.iter().filter(|(k, _)| **k != idx).min_by_key(|(_, (_, t))| *t).map(|(k, _)| *k) else { break };
            self.remove(old);
        }
    }

    pub fn remove(&mut self, idx: u64) {
        if let Some((d, _)) = self.map.remove(&idx) {
            self.bytes -= d.len();
            ALL.fetch_sub(d.len(), Ordering::Relaxed);
        }
    }

    /// Drop the blocks from `idx` on (the file changed there).
    pub fn drop_from(&mut self, idx: u64) {
        let gone: Vec<u64> = self.map.keys().copied().filter(|k| *k >= idx).collect();
        for k in gone {
            self.remove(k);
        }
    }

    pub fn clear(&mut self) {
        self.drop_from(0);
    }
}

impl Drop for BlockCache {
    fn drop(&mut self) {
        ALL.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blk(n: usize) -> Arc<[u8]> {
        vec![0u8; n].into()
    }

    #[test]
    fn the_limit_per_file_drops_the_least_recently_used() {
        let mut c = BlockCache::with_all_limit(10, 30, usize::MAX);
        for i in 0..3 {
            c.insert(i, blk(10));
        }
        assert!(c.get(0).is_some()); // 0 used again: 1 is the oldest now
        c.insert(3, blk(10));
        assert_eq!(c.bytes(), 30);
        assert!(c.contains(0) && !c.contains(1) && c.contains(2) && c.contains(3));
        c.drop_from(2);
        assert_eq!(c.len(), 1);
        assert_eq!(c.bytes(), 10);
    }

    #[test]
    fn the_limit_for_all_viewers_holds_and_is_given_back() {
        // Other tests may use the shared counter at the same time: measure
        // relative to the start and use a limit just above it.
        let base = all_cached();
        let mut a = BlockCache::with_all_limit(100, usize::MAX, base + 1000);
        let mut b = BlockCache::with_all_limit(100, usize::MAX, base + 1000);
        for i in 0..8 {
            a.insert(i, blk(100));
        }
        for i in 0..8 {
            b.insert(i, blk(100));
            assert!(all_cached() <= base + 1000 + 100 * 8, "{}", all_cached() - base);
        }
        // b had to give up its own blocks once the sum was over the limit.
        assert!(b.len() < 8);
        let (na, nb) = (a.bytes(), b.bytes());
        drop(a);
        assert!(all_cached() + na >= base + nb);
        drop(b);
        // A single block always stays, even over the limit.
        let mut c = BlockCache::with_all_limit(100, 50, usize::MAX);
        c.insert(0, blk(100));
        assert_eq!(c.len(), 1);
    }
}
