use crate::util::comparator::Comparator;
use crate::util::random::Random;
use std::cmp::Ordering as CmpOrdering;
use std::marker::PhantomData;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

// Maximum height of the skip list
const MAX_HEIGHT: usize = 12;
// Branching factor - the probability of increasing height is 1/BRANCHING
const BRANCHING: i32 = 4;

/// Node in the skip list
struct Node<K> {
    key: K,
    // Array of next pointers with variable length
    next: Box<[AtomicPtr<Node<K>>]>,
}

impl<K> Node<K> {
    /// Create a new node
    fn new(key: K, height: usize) -> Self {
        let mut next = Vec::with_capacity(height);
        for _ in 0..height {
            next.push(AtomicPtr::new(ptr::null_mut()));
        }

        Node {
            key,
            next: next.into_boxed_slice(),
        }
    }

    /// Get the next node at the specified level (with Acquire ordering)
    fn next(&self, n: usize) -> *mut Node<K> {
        debug_assert!(n < self.next.len());
        self.next[n].load(Ordering::Acquire)
    }

    /// Set the next node at the specified level (with Release ordering)
    fn set_next(&self, n: usize, x: *mut Node<K>) {
        debug_assert!(n < self.next.len());
        self.next[n].store(x, Ordering::Release);
    }

    /// Get the next node at the specified level (with Relaxed ordering)
    fn no_barrier_next(&self, n: usize) -> *mut Node<K> {
        debug_assert!(n < self.next.len());
        self.next[n].load(Ordering::Relaxed)
    }

    /// Set the next node at the specified level (with Relaxed ordering)
    fn no_barrier_set_next(&self, n: usize, x: *mut Node<K>) {
        debug_assert!(n < self.next.len());
        self.next[n].store(x, Ordering::Relaxed);
    }
}

/// Skip list implementation
///
/// # Thread Safety
///
/// Writes require external synchronization, most likely a mutex.
/// Reads require a guarantee that the SkipList will not be destroyed
/// while the read is in progress. Apart from that, reads progress
/// without any internal locking or synchronization.
pub struct SkipList<K, C>
where
    C: Comparator,
    K: AsRef<[u8]>,
{
    head: Box<Node<K>>,
    max_height: AtomicUsize,
    compare: C,
    rnd: Random,
    _marker: PhantomData<K>,
}

/// Iterator for traversing the contents of a skip list
pub struct Iterator<'a, K, C>
where
    C: Comparator,
    K: AsRef<[u8]>,
{
    list: &'a SkipList<K, C>,
    node: *mut Node<K>,
}

impl<K, C> SkipList<K, C>
where
    C: Comparator,
    K: AsRef<[u8]>,
{
    /// Create a new SkipList using the specified comparator function
    pub fn new(compare: C) -> Self
    where
        K: Default,
    {
        let head = Box::new(Node::new(K::default(), MAX_HEIGHT));

        SkipList {
            head,
            max_height: AtomicUsize::new(1),
            compare,
            rnd: Random::new(0xdeadbeef),
            _marker: PhantomData,
        }
    }

    // Get the current maximum height
    fn get_max_height(&self) -> usize {
        self.max_height.load(Ordering::Relaxed)
    }

    // Generate a random height
    fn random_height(&mut self) -> usize {
        let mut height = 1;

        while height < MAX_HEIGHT && self.rnd.one_in(BRANCHING as u32) {
            height += 1;
        }

        height
    }

    // Check if the key is after the node
    fn key_is_after_node(&self, key: &K, n: *mut Node<K>) -> bool {
        if n.is_null() {
            return false;
        }

        unsafe { self.compare.compare(&(*n).key.as_ref(), key.as_ref()) < CmpOrdering::Equal }
    }

    // Find the earliest node that comes at or after key
    // If prev is not null, fills prev[level] with pointer to previous node at "level"
    fn find_greater_or_equal(
        &self,
        key: &K,
        mut prev: Option<&mut [*mut Node<K>]>,
    ) -> *mut Node<K> {
        let mut x = &*self.head as *const Node<K> as *mut Node<K>;
        let mut level = self.get_max_height();

        if level == 0 {
            return ptr::null_mut();
        }

        level -= 1; // Start from the highest level

        loop {
            unsafe {
                let next = (*x).next(level);

                if self.key_is_after_node(key, next) {
                    // Key is after next, continue searching at current level
                    x = next;
                } else {
                    // Record the predecessor node
                    if let Some(p) = &mut prev {
                        p[level] = x;
                    }

                    if level == 0 {
                        return next;
                    } else {
                        // Drop to next level
                        level -= 1;
                    }
                }
            }
        }
    }

    // Find the latest node with a key < key
    // Return head if there is no such node
    fn find_less_than(&self, key: &K) -> *mut Node<K> {
        let mut x = &*self.head as *const Node<K> as *mut Node<K>;
        let mut level = self.get_max_height();

        if level == 0 {
            return x;
        }

        level -= 1; // Start from the highest level

        loop {
            unsafe {
                let next = (*x).next(level);

                if next.is_null()
                    || self.compare.compare(&(*next).key.as_ref(), key.as_ref())
                        >= CmpOrdering::Equal
                {
                    if level == 0 {
                        return x;
                    } else {
                        // Drop to next level
                        level -= 1;
                    }
                } else {
                    // Continue searching at current level
                    x = next;
                }
            }
        }
    }

    // Find the last node in the list
    // Return head if list is empty
    fn find_last(&self) -> *mut Node<K> {
        let mut x = &*self.head as *const Node<K> as *mut Node<K>;
        let mut level = self.get_max_height();

        if level == 0 {
            return x;
        }

        level -= 1; // Start from the highest level

        loop {
            unsafe {
                let next = (*x).next(level);

                if next.is_null() {
                    if level == 0 {
                        return x;
                    } else {
                        // Drop to next level
                        level -= 1;
                    }
                } else {
                    // Continue searching at current level
                    x = next;
                }
            }
        }
    }

    // Check if two keys are equal
    fn equal(&self, a: &K, b: &K) -> bool {
        self.compare.compare(a.as_ref(), b.as_ref()) == CmpOrdering::Equal
    }

    /// Insert key into the list
    ///
    /// # Requirements
    ///
    /// Nothing that compares equal to key is currently in the list
    pub fn insert(&mut self, key: K) {
        // Create array for tracking predecessors
        let mut prev = [ptr::null_mut(); MAX_HEIGHT];
        let x = self.find_greater_or_equal(&key, Some(&mut prev));

        // Ensure we're not inserting a duplicate key
        unsafe {
            assert!(x.is_null() || !self.equal(&key, &(*x).key));
        }

        let height = self.random_height();

        if height > self.get_max_height() {
            for i in self.get_max_height()..height {
                prev[i] = &*self.head as *const Node<K> as *mut Node<K>;
            }

            // Update max height (relaxed ordering is sufficient)
            self.max_height.store(height, Ordering::Relaxed);
        }

        // Create new node
        let x = Box::into_raw(Box::new(Node::new(key, height)));

        for i in 0..height {
            unsafe {
                // Set next pointers for the new node
                (*x).no_barrier_set_next(i, (*prev[i]).no_barrier_next(i));
                // Set predecessor's next pointer to the new node
                (*prev[i]).set_next(i, x);
            }
        }
    }

    /// Returns true if an entry that compares equal to key is in the list
    pub fn contains(&self, key: &K) -> bool {
        let x = self.find_greater_or_equal(key, None);

        if !x.is_null() {
            unsafe {
                return self.equal(key, &(*x).key);
            }
        }

        false
    }

    /// Returns an iterator over the skip list
    pub fn iter(&self) -> Iterator<K, C> {
        Iterator {
            list: self,
            node: ptr::null_mut(),
        }
    }
}

impl<'a, K, C> Iterator<'a, K, C>
where
    C: Comparator,
    K: AsRef<[u8]>,
{
    /// Returns true if the iterator is positioned at a valid node
    pub fn valid(&self) -> bool {
        !self.node.is_null()
    }

    /// Returns the key at the current position
    ///
    /// # Requirements
    ///
    /// Iterator must be valid
    pub fn key(&self) -> &K {
        assert!(self.valid());

        unsafe { &(*self.node).key }
    }

    /// Advances to the next position
    ///
    /// # Requirements
    ///
    /// Iterator must be valid
    pub fn next(&mut self) {
        assert!(self.valid());

        unsafe {
            self.node = (*self.node).next(0);
        }
    }

    /// Advances to the previous position
    ///
    /// # Requirements
    ///
    /// Iterator must be valid
    pub fn prev(&mut self) {
        assert!(self.valid());

        let key = unsafe { &(*self.node).key };
        self.node = self.list.find_less_than(key);

        // If we're at the head node, set iterator to invalid
        if ptr::eq(self.node, &*self.list.head as *const _ as *mut _) {
            self.node = ptr::null_mut();
        }
    }

    /// Advance to the first entry with a key >= target
    pub fn seek(&mut self, target: &K) {
        self.node = self.list.find_greater_or_equal(target, None);
    }

    /// Position at the first entry in list
    ///
    /// Final state of iterator is valid if list is not empty
    pub fn seek_to_first(&mut self) {
        unsafe {
            self.node = (*(&*self.list.head as *const Node<K> as *mut Node<K>)).next(0);
        }
    }

    /// Position at the last entry in list
    ///
    /// Final state of iterator is valid if list is not empty
    pub fn seek_to_last(&mut self) {
        self.node = self.list.find_last();

        // If we're at the head node, set iterator to invalid
        if ptr::eq(self.node, &*self.list.head as *const _ as *mut _) {
            self.node = ptr::null_mut();
        }
    }
}

impl<K, C> Drop for SkipList<K, C>
where
    C: Comparator,
    K: AsRef<[u8]>,
{
    fn drop(&mut self) {
        // Free all nodes
        let mut node = unsafe { (*(&*self.head as *const Node<K> as *mut Node<K>)).next(0) };

        while !node.is_null() {
            let next = unsafe { (*node).next(0) };
            unsafe { Box::from_raw(node) }; // Free the node
            node = next;
        }
    }
}
