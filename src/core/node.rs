use crossbeam_epoch::Atomic;
use parking_lot::Mutex;

use super::bin_entry::BinEntry;

#[derive(Debug)]
pub struct Node<K, V> {
    pub hash: u64,
    pub key: K,
    pub value: Atomic<V>,
    pub next: Atomic<BinEntry<K, V>>,
    pub lock: Mutex<()>,
}

impl<K, V> Node<K, V> {
    pub fn new<AV>(hash: u64, key: K, value: AV, next: Atomic<BinEntry<K, V>>) -> Self
    where
        AV: Into<Atomic<V>>,
    {
        Node {
            hash,
            key,
            value: value.into(),
            next,
            lock: Mutex::new(()),
        }
    }
}
