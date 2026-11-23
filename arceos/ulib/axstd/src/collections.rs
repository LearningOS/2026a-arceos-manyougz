pub use alloc::collections::*;

use alloc::vec::Vec;
use core::hash::{Hash, Hasher};

pub struct HashMap<K, V> {
    buckets: Vec<Vec<(K, V)>>,
    len: usize,
}

impl<K: Eq + Hash, V> HashMap<K, V> {
    pub fn new() -> Self {
        Self {
            buckets: Vec::new(),
            len: 0,
        }
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if self.len == self.buckets.len() {
            let capacity = (self.buckets.len() * 2).max(8);
            let mut buckets = Vec::new();
            buckets.resize_with(capacity, Vec::new);

            // 桶数改变后，按新的桶数重新分配已有条目。
            for (key, value) in self.buckets.drain(..).flatten() {
                let index = bucket_index(&key, capacity);
                buckets[index].push((key, value));
            }
            self.buckets = buckets;
        }

        let index = bucket_index(&key, self.buckets.len());
        let bucket = &mut self.buckets[index];
        for (old_key, old_value) in bucket.iter_mut() {
            if *old_key == key {
                return Some(core::mem::replace(old_value, value));
            }
        }
        bucket.push((key, value));
        self.len += 1;
        None
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.buckets
            .iter()
            .flatten()
            .map(|(key, value)| (key, value))
    }
}

fn bucket_index(key: &impl Hash, capacity: usize) -> usize {
    let mut hasher = FnvHasher(0xcbf2_9ce4_8422_2325);
    key.hash(&mut hasher);
    hasher.finish() as usize % capacity
}

struct FnvHasher(u64);

impl Hasher for FnvHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
