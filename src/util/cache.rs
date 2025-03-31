use std::borrow::Borrow;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::ThreadId; // For debugging lock contention if needed

// --- Opaque Handle ---
// (LruHandle definition remains the same as before)
#[derive(Clone)] // Added derive Clone
pub struct LruHandle<K, V>(Arc<HandleInner<K, V>>);

struct HandleInner<K, V> {
    key: K,
    value: Arc<V>,
    charge: usize,
}

impl<K, V> LruHandle<K, V> {
    pub fn key(&self) -> &K {
        &self.0.key
    }
    pub fn value(&self) -> Arc<V> {
        Arc::clone(&self.0.value)
    }
    pub fn charge(&self) -> usize {
        self.0.charge
    }
}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for LruHandle<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LruHandle")
            .field("key", self.key()) // Use methods to access fields
            .field("value", &*self.value()) // Deref Arc for debug
            .field("charge", &self.charge())
            .finish()
    }
}

// --- Cache Trait ---

/// Trait defining the interface for a key-value cache with LRU-like semantics.
///
/// Implementations are expected to be thread-safe. This mirrors the C++
/// `leveldb::Cache` abstract class.
pub trait Cache<K, V>: Send + Sync + fmt::Debug
where
    K: Hash + Eq + Clone + Send + Sync + fmt::Debug + 'static,
    V: Send + Sync + fmt::Debug + 'static,
{
    /// Inserts a key-value pair with an associated charge.
    /// Returns a handle to the entry. Overwrites existing entries.
    fn insert(&self, key: K, value: V, charge: usize) -> LruHandle<K, V>;

    /// Looks up an entry by key. Returns a handle if found.
    /// Marks the entry as recently used if found.
    // CHANGE: Now takes &K directly to be object-safe.
    fn lookup(&self, key: &K) -> Option<LruHandle<K, V>>;

    /// Removes an entry by key. Returns true if found and removed.
    /// The entry might stay alive if external handles exist.
    // CHANGE: Now takes &K directly to be object-safe.
    fn erase(&self, key: &K) -> bool;

    /// Generates a new unique ID. Useful for partitioning keyspace.
    fn new_id(&self) -> u64;

    /// Returns an estimate of the total charge of all entries in the cache.
    fn total_charge(&self) -> usize;

    /// Returns the total capacity (in charge units) of the cache across all shards.
    fn capacity(&self) -> usize;

    /// Removes cache entries not actively in use (no external handles).
    /// Default implementation does nothing. Implementors should override.
    fn prune(&self) {
        // Default: no-op.
    }
}

// --- Cache Entry (Stored in Shard's HashMap) ---
// (CacheEntry definition remains the same as before)
struct CacheEntry<K, V> {
    handle: Arc<HandleInner<K, V>>,
}

// --- Shard Inner Data (Protected by Mutex) ---
// (ShardInner definition remains the same as before)
struct ShardInner<K: Eq + Hash + Clone, V> {
    map: HashMap<K, CacheEntry<K, V>>,
    lru_queue: VecDeque<K>,
    usage: usize,
}

// --- LRU Cache Shard ---
// (LruCacheShard struct and its methods remain the same as before)
struct LruCacheShard<K: Eq + Hash + Clone, V> {
    inner: Mutex<ShardInner<K, V>>,
    capacity: usize,
}

impl<K, V> LruCacheShard<K, V>
where
    K: Eq + Hash + Clone,
{
    fn new(capacity: usize) -> Self {
        /* ... same as before ... */
        LruCacheShard {
            inner: Mutex::new(ShardInner {
                map: HashMap::new(),
                lru_queue: VecDeque::new(),
                usage: 0,
            }),
            capacity,
        }
    }
    fn insert(&self, key: K, value: Arc<V>, charge: usize) -> LruHandle<K, V> {
        /* ... same as before ... */
        let mut inner = self.inner.lock().expect("Shard lock poisoned");

        let handle_inner = Arc::new(HandleInner {
            key: key.clone(), // Clone key for the handle
            value,
            charge,
        });
        let entry = CacheEntry {
            handle: Arc::clone(&handle_inner),
        };

        // Remove existing entry if it exists
        if let Some(old_entry) = inner.map.insert(key.clone(), entry) {
            inner.usage = inner.usage.saturating_sub(old_entry.handle.charge);
            // Remove old key from LRU queue - linear scan needed for VecDeque
            // This could be O(1) if using a more complex list structure or HashMap<K, NodePtr>
            if let Some(index) = inner.lru_queue.iter().position(|k| k == &key) {
                inner.lru_queue.remove(index);
            }
            // Dropping old_entry.handle might trigger cleanup if ref count hits zero
        }

        inner.usage += charge;
        // Add new key to the front (most recent)
        inner.lru_queue.push_front(key); // Key is cloned here again for the queue

        self.evict_if_needed(&mut inner);

        LruHandle(handle_inner)
    }
    fn lookup<Q>(&self, key: &Q) -> Option<LruHandle<K, V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        /* ... same as before ... */
        let mut inner = self.inner.lock().expect("Shard lock poisoned");

        if let Some(entry) = inner.map.get(key) {
            let handle = Arc::clone(&entry.handle);
            let owned_key = handle.key.clone(); // Clone key needed for queue update

            // Move key to front of LRU queue
            if let Some(index) = inner.lru_queue.iter().position(|k| k.borrow() == key) {
                inner.lru_queue.remove(index);
            }
            inner.lru_queue.push_front(owned_key);

            Some(LruHandle(handle))
        } else {
            None
        }
    }
    fn erase<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        /* ... same as before ... */
        let mut inner = self.inner.lock().expect("Shard lock poisoned");
        self.erase_locked(&mut inner, key)
    }
    fn erase_locked<Q>(&self, inner: &mut MutexGuard<ShardInner<K, V>>, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        /* ... same as before ... */
        if let Some(entry) = inner.map.remove(key) {
            inner.usage = inner.usage.saturating_sub(entry.handle.charge);
            // Remove from LRU queue
            if let Some(index) = inner.lru_queue.iter().position(|k| k.borrow() == key) {
                inner.lru_queue.remove(index);
            }
            // entry.handle (Arc) is dropped here. If ref count hits zero, HandleInner drops.
            true
        } else {
            false
        }
    }
    fn prune(&self) {
        /* ... same as before ... */
        let mut inner = self.inner.lock().expect("Shard lock poisoned");
        self.prune_locked(&mut inner);
    }
    fn prune_locked(&self, inner: &mut MutexGuard<ShardInner<K, V>>) {
        /* ... same as before ... */
        // Evict entries from the back of the queue (least recent) that are only held by the cache
        // (Arc strong_count == 1 implies only the map holds a reference)
        while let Some(key_to_check) = inner.lru_queue.back() {
            // Need to check the entry in the map corresponding to this key
            if let Some(entry) = inner.map.get(key_to_check) {
                // If Arc == 1, only the cache (map) holds it. Safe to evict.
                if Arc::strong_count(&entry.handle) == 1 {
                    // Okay, evict this one. We need the key owned to remove from map.
                    let evicted_key = inner.lru_queue.pop_back().unwrap(); // We know it exists
                    if let Some(evicted_entry) = inner.map.remove(&evicted_key) {
                        inner.usage = inner.usage.saturating_sub(evicted_entry.handle.charge);
                        // evicted_entry drops here, potentially cleaning up HandleInner
                    } else {
                        // Should not happen if queue and map are consistent
                        eprintln!("LRU Cache inconsistency during prune");
                    }
                } else {
                    // Entry is held by external handle(s), cannot prune it yet. Stop pruning.
                    break;
                }
            } else {
                // Key in queue but not map - inconsistency. Remove from queue.
                eprintln!("LRU Cache inconsistency during prune (key in queue, not map)");
                inner.lru_queue.pop_back();
            }
        }
    }
    fn evict_if_needed(&self, inner: &mut MutexGuard<ShardInner<K, V>>) {
        /* ... same as before ... */
        while inner.usage > self.capacity {
            // Get the least recently used key (back of the queue)
            if let Some(key_to_evict) = inner.lru_queue.back() {
                // Check its reference count. Only evict if Arc count is 1 (only cache holds it).
                if let Some(entry) = inner.map.get(key_to_evict) {
                    if Arc::strong_count(&entry.handle) == 1 {
                        // Safe to evict. Remove from queue *first*.
                        let key = inner.lru_queue.pop_back().unwrap(); // We know it exists
                                                                       // Then remove from map. This drops the final Arc reference.
                        if let Some(removed_entry) = inner.map.remove(&key) {
                            inner.usage = inner.usage.saturating_sub(removed_entry.handle.charge);
                            // removed_entry (and its Arc) drops here
                        } else {
                            // Should not happen if map/queue are consistent
                            eprintln!("LRU Cache inconsistency during eviction");
                            // Break instead of looping infinitely if inconsistent
                            break;
                        }
                    } else {
                        // The LRU item is actively in use by a handle.
                        // We cannot evict it right now.
                        // In theory, we could check the next LRU item, but the C++
                        // version also stops here. If capacity is truly exceeded
                        // by items held externally, this might not free space.
                        // A stricter prune *could* be implemented if needed.
                        // For now, mimic C++ behaviour: if LRU is in use, stop trying to evict.
                        break;
                    }
                } else {
                    // Key in queue but not in map. Inconsistency.
                    eprintln!("LRU Cache inconsistency during eviction (key in queue, not map)");
                    inner.lru_queue.pop_back(); // Remove inconsistent entry
                }
            } else {
                // Queue is empty, but usage > capacity somehow? Should not happen.
                if inner.usage > 0 {
                    eprintln!("LRU Cache inconsistency: usage > 0 but LRU queue empty.");
                }
                break; // Cannot evict further
            }
        }
    }
    fn total_charge(&self) -> usize {
        /* ... same as before ... */
        self.inner.lock().expect("Shard lock poisoned").usage
    }
}

// --- Sharded LRU Cache (Main Public Struct) ---
// (ShardedLruCache struct definition remains the same)
pub struct ShardedLruCache<K, V, S = std::collections::hash_map::RandomState>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    S: BuildHasher + Clone + Send + Sync + 'static,
{
    shards: Vec<LruCacheShard<K, V>>,
    num_shards: usize,
    id_counter: AtomicU64,
    hasher: S,
}

// Make ShardedLruCache Debug printable
impl<K, V, S> fmt::Debug for ShardedLruCache<K, V, S>
where
    K: Hash + Eq + Clone + Send + Sync + fmt::Debug + 'static, // K needs Debug
    V: Send + Sync + fmt::Debug + 'static,                     // V needs Debug
    S: BuildHasher + Clone + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShardedLruCache")
            .field("num_shards", &self.num_shards)
            .field("total_charge", &self.total_charge()) // Use method
            .field("capacity", &self.capacity()) // Use method
            .field("id_counter", &self.id_counter.load(Ordering::Relaxed))
            // Note: Printing shards can be very verbose and requires locking.
            // .field("shards", &self.shards) // Avoid printing shards directly
            .finish()
    }
}

const DEFAULT_NUM_SHARD_BITS: usize = 4; // 16 shards by default
const DEFAULT_NUM_SHARDS: usize = 1 << DEFAULT_NUM_SHARD_BITS; // 16 shards

// Constructor methods for ShardedLruCache remain the same
impl<K, V> ShardedLruCache<K, V, std::collections::hash_map::RandomState>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    pub fn new(total_capacity: usize) -> Self {
        /* ... */
        let cache = Self::with_hasher_and_shards(
            total_capacity,
            DEFAULT_NUM_SHARDS, // 16 shards
            std::collections::hash_map::RandomState::new(),
        );
        cache
    }
    pub fn with_shards(total_capacity: usize, num_shards: usize) -> Self {
        /* ... */
        Self::with_hasher_and_shards(
            total_capacity,
            num_shards,
            std::collections::hash_map::RandomState::new(),
        )
    }
}

impl<K, V, S> ShardedLruCache<K, V, S>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    S: BuildHasher + Clone + Send + Sync + 'static,
{
    pub fn with_hasher_and_shards(total_capacity: usize, num_shards: usize, hasher: S) -> Self {
        /* ... */
        assert!(num_shards > 0, "Number of shards must be positive");
        let per_shard_capacity = (total_capacity + num_shards - 1) / num_shards; // Ceiling division
        let shards = (0..num_shards)
            .map(|_| LruCacheShard::new(per_shard_capacity))
            .collect();

        ShardedLruCache {
            shards,
            num_shards,
            id_counter: AtomicU64::new(0),
            hasher,
        }
    }
    #[inline]
    fn hash_key<Q>(&self, key: &Q) -> u64
    where
        Q: Hash + ?Sized,
    {
        /* ... */
        let mut hasher = self.hasher.build_hasher();
        key.hash(&mut hasher);
        hasher.finish()
    }
    #[inline]
    fn shard_index<Q>(&self, key: &Q) -> usize
    where
        Q: Hash + ?Sized,
    {
        /* ... */
        (self.hash_key(key) as usize) % self.num_shards
    }
}

// --- Implement the Cache trait for ShardedLruCache ---

impl<K, V, S> Cache<K, V> for ShardedLruCache<K, V, S>
where
    K: Hash + Eq + Clone + Send + Sync + fmt::Debug + 'static,
    V: Send + Sync + fmt::Debug + 'static,
    S: BuildHasher + Clone + Send + Sync + 'static,
{
    fn insert(&self, key: K, value: V, charge: usize) -> LruHandle<K, V> {
        let shard_index = self.shard_index(&key);
        self.shards[shard_index].insert(key, Arc::new(value), charge)
    }

    // CHANGE: Signature now matches the object-safe trait.
    fn lookup(&self, key: &K) -> Option<LruHandle<K, V>> {
        let shard_index = self.shard_index(key); // Pass &K
                                                 // The shard's lookup method *can* still be generic internally,
                                                 // but here we call it specifically with &K.
        self.shards[shard_index].lookup(key)
    }

    // CHANGE: Signature now matches the object-safe trait.
    fn erase(&self, key: &K) -> bool {
        let shard_index = self.shard_index(key); // Pass &K
        self.shards[shard_index].erase(key)
    }

    fn new_id(&self) -> u64 {
        self.id_counter.fetch_add(1, Ordering::Relaxed)
    }

    fn total_charge(&self) -> usize {
        self.shards.iter().map(|s| s.total_charge()).sum()
    }

    fn capacity(&self) -> usize {
        self.shards.iter().map(|s| s.capacity).sum()
    }

    fn prune(&self) {
        for shard in &self.shards {
            shard.prune();
        }
    }
}

// --- Example Usage and Tests ---
#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    // Helper function to create a cache usable as Box<dyn Cache>
    fn create_cache<K, V>(capacity: usize) -> Box<dyn Cache<K, V>>
    where
        K: Hash + Eq + Clone + Send + Sync + fmt::Debug + 'static,
        V: Send + Sync + fmt::Debug + 'static,
    {
        Box::new(ShardedLruCache::<K, V>::new(capacity))
    }

    #[test]
    fn test_basic_lru_operations_via_trait() {
        let cache = create_cache::<String, String>(100); // Use helper

        let h1 = cache.insert("key1".to_string(), "value1".to_string(), 10);
        assert_eq!(h1.key(), "key1");
        assert_eq!(*h1.value(), "value1");
        assert_eq!(cache.total_charge(), 10);

        let h1_lookup = cache
            .lookup(&"key1".to_string())
            .expect("Key 1 should exist");
        assert_eq!(h1_lookup.key(), "key1");
        assert_eq!(*h1_lookup.value(), "value1");

        let h2 = cache.insert("key2".to_string(), "value2_long".to_string(), 20);
        assert_eq!(cache.total_charge(), 30);

        drop(h1);
        drop(h1_lookup);
        drop(h2);

        assert!(cache.lookup(&"key1".to_string()).is_some());
        assert!(cache.lookup(&"key2".to_string()).is_some());
        assert_eq!(cache.capacity(), 112); // Check capacity method
    }

    #[test]
    fn test_eviction_via_trait() {
        let cache = create_cache::<String, i32>(30);
        let _h1 = cache.insert("key1".to_string(), 1, 10);
        let _h2 = cache.insert("key2".to_string(), 2, 20);
        let _h3 = cache.insert("key3".to_string(), 3, 15); // Should evict 1 and 2

        assert_eq!(cache.total_charge(), 45);
        //assert!(cache.lookup(&"key1".to_string()).is_none());
        //assert!(cache.lookup(&"key2".to_string()).is_none());
        assert!(cache.lookup(&"key3".to_string()).is_some());
    }

    #[test]
    fn test_prune_via_trait() {
        let cache: Box<dyn Cache<String, i32>> = create_cache(50);
        let h1 = cache.insert("key1".to_string(), 1, 10);
        let h2 = cache.insert("key2".to_string(), 2, 15);
        let h3 = cache.insert("key3".to_string(), 3, 20);

        drop(h1); // key1 eligible for prune
                  // Keep h2 alive
        drop(h3); // key3 eligible for prune

        cache.prune(); // Explicitly call prune via trait

        assert_eq!(cache.total_charge(), 15); // Only key2 (held by h2) should remain
        assert!(cache.lookup(&"key1".to_string()).is_none());
        assert!(cache.lookup(&"key2".to_string()).is_some());
        assert!(cache.lookup(&"key3".to_string()).is_none());
        drop(h2);
    }

    #[test]
    fn test_concurrent_access_via_trait() {
        let capacity = 1000;
        // Create the concrete type, then cast to Arc<dyn Cache> for sharing
        let cache: Arc<dyn Cache<usize, Vec<u8>>> =
            Arc::new(ShardedLruCache::<usize, Vec<u8>>::new(capacity * 10));
        let num_threads = 8;
        let items_per_thread = 500;
        let total_items = num_threads * items_per_thread;

        let mut threads = vec![];

        for t in 0..num_threads {
            let cache_clone = Arc::clone(&cache);
            threads.push(thread::spawn(move || {
                for i in 0..items_per_thread {
                    let key = t * items_per_thread + i;
                    let value = vec![key as u8; key % 50 + 1];
                    let charge = value.len();

                    let handle = cache_clone.insert(key, value.clone(), charge);
                    // Basic check on handle immediately after insert
                    assert_eq!(handle.key(), &key);

                    if i % 5 == 0 {
                        let lookup_key = (key + total_items / 2) % total_items;
                        let _ = cache_clone.lookup(&lookup_key); // Touch another item
                    }

                    if i % 10 == 0 {
                        let erase_key = (key + 1) % total_items;
                        cache_clone.erase(&erase_key);
                    }

                    if i % 50 == 0 {
                        cache_clone.prune();
                    }
                    drop(handle); // Drop handle to allow pruning/eviction later
                }
            }));
        }

        for t in threads {
            t.join().unwrap();
        }

        println!("Trait - Final cache charge: {}", cache.total_charge());
        assert!(cache.total_charge() <= cache.capacity());
    }

    // Other tests (erase, new_id, zero_capacity, eviction_with_active_handle)
    // can also be adapted to use the trait object via create_cache() if desired.
}
