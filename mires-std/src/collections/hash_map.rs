//! Pure-Rust open-addressing `HashMap<K, V, S = RandomState>`, `RandomState`,
//! `DefaultHasher` (SipHash-1-3), and the full `Entry` / iterator API.
//!
//! Implemented entirely in safe Rust without raw-pointer byte punning or SIMD
//! control-byte tricks so it lowers cleanly through `mires`'s JIR value model.

use crate::borrow::Borrow;
use crate::cell::Cell;
use crate::fmt;
use crate::hash::{BuildHasher, Hash, Hasher};
use crate::iter::FusedIterator;
use crate::mem;
use crate::ops::Index;
use crate::vec::Vec;

const MIN_CAPACITY: usize = 8;

/// SipHash-1-3 hasher — pure safe Rust `Hasher` implementation.
#[derive(Clone, Debug)]
pub struct DefaultHasher {
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    tail: u64,
    ntail: usize,
    length: usize,
}

impl DefaultHasher {
    /// Creates a new `DefaultHasher` initialized with zero keys.
    #[must_use]
    #[inline]
    pub fn new() -> DefaultHasher {
        Self::new_with_keys(0, 0)
    }

    #[must_use]
    #[inline]
    pub(crate) fn new_with_keys(k0: u64, k1: u64) -> DefaultHasher {
        DefaultHasher {
            v0: 0x736f_6d65_7073_6575 ^ k0,
            v1: 0x646f_7261_6e64_6f6d ^ k1,
            v2: 0x6c79_6765_6e65_7261 ^ k0,
            v3: 0x7465_6462_7974_6573 ^ k1,
            tail: 0,
            ntail: 0,
            length: 0,
        }
    }

    #[inline]
    fn sip_round(&mut self) {
        self.v0 = self.v0.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(13) ^ self.v0;
        self.v0 = self.v0.rotate_left(32);
        self.v2 = self.v2.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(16) ^ self.v2;
        self.v0 = self.v0.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(21) ^ self.v0;
        self.v2 = self.v2.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(17) ^ self.v2;
        self.v2 = self.v2.rotate_left(32);
    }

    #[inline]
    fn compress(&mut self, m: u64) {
        self.v3 ^= m;
        self.sip_round();
        self.v0 ^= m;
    }
}

impl Default for DefaultHasher {
    #[inline]
    fn default() -> DefaultHasher {
        DefaultHasher::new()
    }
}

impl Hasher for DefaultHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len());
        let mut i = 0;

        if self.ntail > 0 {
            while i < bytes.len() && self.ntail < 8 {
                self.tail |= u64::from(bytes[i]) << (self.ntail * 8);
                self.ntail += 1;
                i += 1;
            }
            if self.ntail == 8 {
                let m = self.tail;
                self.tail = 0;
                self.ntail = 0;
                self.compress(m);
            }
        }

        while i + 8 <= bytes.len() {
            let m = u64::from_le_bytes([
                bytes[i],
                bytes[i + 1],
                bytes[i + 2],
                bytes[i + 3],
                bytes[i + 4],
                bytes[i + 5],
                bytes[i + 6],
                bytes[i + 7],
            ]);
            self.compress(m);
            i += 8;
        }

        while i < bytes.len() {
            self.tail |= u64::from(bytes[i]) << (self.ntail * 8);
            self.ntail += 1;
            i += 1;
        }
    }

    #[inline]
    fn finish(&self) -> u64 {
        let mut state = self.clone();
        let b = ((state.length as u64 & 0xff) << 56) | state.tail;
        state.compress(b);
        state.v2 ^= 0xff;
        state.sip_round();
        state.sip_round();
        state.sip_round();
        state.v0 ^ state.v1 ^ state.v2 ^ state.v3
    }
}

struct SeedCounter(Cell<u32>);

// SAFETY: `mires` targets single-threaded JS (`singlethread = true`).
unsafe impl Sync for SeedCounter {}

static SEED_COUNTER: SeedCounter = SeedCounter(Cell::new(1));

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// The default [`BuildHasher`] used by [`HashMap`] and [`crate::collections::HashSet`].
#[derive(Clone, Debug)]
pub struct RandomState {
    k0: u64,
    k1: u64,
}

impl RandomState {
    /// Constructs a new `RandomState` seeded with distinct keys.
    #[must_use]
    pub fn new() -> RandomState {
        let n = SEED_COUNTER.0.get();
        SEED_COUNTER.0.set(n.wrapping_add(1));
        let seed = u64::from(n) | (u64::from(n.wrapping_mul(0x9e37_79b1)) << 32);
        let k0 = splitmix64(seed);
        let k1 = splitmix64(k0);
        RandomState { k0, k1 }
    }
}

impl Default for RandomState {
    #[inline]
    fn default() -> RandomState {
        RandomState::new()
    }
}

impl BuildHasher for RandomState {
    type Hasher = DefaultHasher;

    #[inline]
    fn build_hasher(&self) -> DefaultHasher {
        DefaultHasher::new_with_keys(self.k0, self.k1)
    }
}

#[derive(Clone)]
enum Slot<K, V> {
    Empty,
    Tombstone,
    Occupied { hash: u64, key: K, value: V },
}

/// A hash map implemented with safe open addressing and linear probing.
pub struct HashMap<K, V, S = RandomState> {
    slots: Vec<Slot<K, V>>,
    len: usize,
    tombstones: usize,
    hash_builder: S,
}

#[inline]
fn make_hash<Q: Hash + ?Sized, S: BuildHasher>(hash_builder: &S, val: &Q) -> u64 {
    let mut state = hash_builder.build_hasher();
    val.hash(&mut state);
    state.finish()
}

#[inline]
fn capacity_for_items(items: usize) -> usize {
    if items == 0 {
        return 0;
    }
    let needed = items.saturating_mul(4) / 3 + 1;
    needed.max(MIN_CAPACITY).next_power_of_two()
}

impl<K, V> HashMap<K, V, RandomState> {
    /// Creates an empty `HashMap`.
    #[must_use]
    #[inline]
    pub fn new() -> HashMap<K, V, RandomState> {
        HashMap::with_hasher(RandomState::new())
    }

    /// Creates an empty `HashMap` with at least the specified capacity.
    #[must_use]
    #[inline]
    pub fn with_capacity(capacity: usize) -> HashMap<K, V, RandomState> {
        HashMap::with_capacity_and_hasher(capacity, RandomState::new())
    }
}

impl<K, V, S> HashMap<K, V, S> {
    /// Creates an empty `HashMap` which will use the given hash builder.
    #[must_use]
    #[inline]
    pub const fn with_hasher(hash_builder: S) -> HashMap<K, V, S> {
        HashMap {
            slots: Vec::new(),
            len: 0,
            tombstones: 0,
            hash_builder,
        }
    }

    /// Creates an empty `HashMap` with at least the specified capacity and hash builder.
    #[must_use]
    pub fn with_capacity_and_hasher(capacity: usize, hash_builder: S) -> HashMap<K, V, S> {
        let cap = capacity_for_items(capacity);
        let mut slots = Vec::with_capacity(cap);
        for _ in 0..cap {
            slots.push(Slot::Empty);
        }
        HashMap {
            slots,
            len: 0,
            tombstones: 0,
            hash_builder,
        }
    }

    /// Returns the number of elements the map can hold without reallocating.
    #[must_use]
    #[inline]
    pub fn capacity(&self) -> usize {
        self.slots.len() * 3 / 4
    }

    /// Returns the number of elements in the map.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the map contains no elements.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns a reference to the map's [`BuildHasher`].
    #[must_use]
    #[inline]
    pub fn hasher(&self) -> &S {
        &self.hash_builder
    }

    /// Clears the map, removing all key-value pairs while keeping allocated storage.
    pub fn clear(&mut self) {
        if self.len == 0 && self.tombstones == 0 {
            return;
        }
        for slot in &mut self.slots {
            *slot = Slot::Empty;
        }
        self.len = 0;
        self.tombstones = 0;
    }

    /// An iterator visiting all key-value pairs in arbitrary order.
    #[inline]
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            inner: self.slots.iter(),
            remaining: self.len,
        }
    }

    /// An iterator visiting all key-value pairs in arbitrary order, with mutable references to values.
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, K, V> {
        IterMut {
            inner: self.slots.iter_mut(),
            remaining: self.len,
        }
    }

    /// An iterator visiting all keys in arbitrary order.
    #[inline]
    pub fn keys(&self) -> Keys<'_, K, V> {
        Keys { inner: self.iter() }
    }

    /// Creates a consuming iterator visiting all the keys in arbitrary order.
    #[inline]
    pub fn into_keys(self) -> IntoKeys<K, V> {
        IntoKeys {
            inner: self.into_iter(),
        }
    }

    /// An iterator visiting all values in arbitrary order.
    #[inline]
    pub fn values(&self) -> Values<'_, K, V> {
        Values { inner: self.iter() }
    }

    /// An iterator visiting all values mutably in arbitrary order.
    #[inline]
    pub fn values_mut(&mut self) -> ValuesMut<'_, K, V> {
        ValuesMut {
            inner: self.iter_mut(),
        }
    }

    /// Creates a consuming iterator visiting all the values in arbitrary order.
    #[inline]
    pub fn into_values(self) -> IntoValues<K, V> {
        IntoValues {
            inner: self.into_iter(),
        }
    }

    /// Clears the map, returning all key-value pairs as an iterator.
    #[inline]
    pub fn drain(&mut self) -> Drain<'_, K, V> {
        let remaining = self.len;
        self.len = 0;
        self.tombstones = 0;
        Drain {
            inner: self.slots.iter_mut(),
            remaining,
        }
    }

    /// Retains only the elements specified by the predicate.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&K, &mut V) -> bool,
    {
        for slot in &mut self.slots {
            let keep = match slot {
                Slot::Occupied { key, value, .. } => f(key, value),
                _ => true,
            };
            if !keep {
                *slot = Slot::Tombstone;
                self.len -= 1;
                self.tombstones += 1;
            }
        }
    }
}

impl<K, V, S> HashMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher,
{
    fn rehash(&mut self, new_cap: usize) {
        let old_slots = mem::take(&mut self.slots);
        if new_cap == 0 {
            self.len = 0;
            self.tombstones = 0;
            return;
        }
        let mut new_slots = Vec::with_capacity(new_cap);
        for _ in 0..new_cap {
            new_slots.push(Slot::Empty);
        }
        let mask = new_cap - 1;
        for slot in old_slots {
            if let Slot::Occupied { hash, key, value } = slot {
                let mut idx = (hash as usize) & mask;
                while matches!(new_slots[idx], Slot::Occupied { .. }) {
                    idx = (idx + 1) & mask;
                }
                new_slots[idx] = Slot::Occupied { hash, key, value };
            }
        }
        self.slots = new_slots;
        self.tombstones = 0;
    }

    fn ensure_capacity_for_one(&mut self) {
        let cap = self.slots.len();
        if cap == 0 {
            self.rehash(MIN_CAPACITY);
            return;
        }
        if (self.len + self.tombstones + 1) * 4 > cap * 3 {
            let new_cap = if self.len * 2 < cap { cap } else { cap * 2 };
            self.rehash(new_cap);
        }
    }

    /// Reserves capacity for at least `additional` more elements.
    pub fn reserve(&mut self, additional: usize) {
        let needed_cap = capacity_for_items(self.len.saturating_add(additional));
        if needed_cap > self.slots.len() {
            self.rehash(needed_cap);
        }
    }

    /// Tries to reserve capacity for at least `additional` more elements.
    pub fn try_reserve(
        &mut self,
        additional: usize,
    ) -> Result<(), crate::collections::TryReserveError> {
        self.reserve(additional);
        Ok(())
    }

    /// Shrinks the capacity of the map as much as possible.
    pub fn shrink_to_fit(&mut self) {
        self.shrink_to(0);
    }

    /// Shrinks the capacity of the map with a lower limit.
    pub fn shrink_to(&mut self, min_capacity: usize) {
        let target = capacity_for_items(self.len.max(min_capacity));
        if target < self.slots.len() {
            self.rehash(target);
        }
    }

    fn find_slot<Q>(&self, key: &Q) -> Option<usize>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let cap = self.slots.len();
        if cap == 0 {
            return None;
        }
        let hash = make_hash(&self.hash_builder, key);
        let mask = cap - 1;
        let mut idx = (hash as usize) & mask;
        for _ in 0..cap {
            match &self.slots[idx] {
                Slot::Empty => return None,
                Slot::Tombstone => {}
                Slot::Occupied {
                    hash: slot_hash,
                    key: slot_key,
                    ..
                } => {
                    if *slot_hash == hash && slot_key.borrow() == key {
                        return Some(idx);
                    }
                }
            }
            idx = (idx + 1) & mask;
        }
        None
    }

    fn find_or_insert_slot(&self, hash: u64, key: &K) -> Result<usize, usize> {
        let cap = self.slots.len();
        let mask = cap - 1;
        let mut idx = (hash as usize) & mask;
        let mut first_tombstone: Option<usize> = None;
        for _ in 0..cap {
            match &self.slots[idx] {
                Slot::Empty => return Err(first_tombstone.unwrap_or(idx)),
                Slot::Tombstone => {
                    if first_tombstone.is_none() {
                        first_tombstone = Some(idx);
                    }
                }
                Slot::Occupied {
                    hash: slot_hash,
                    key: slot_key,
                    ..
                } => {
                    if *slot_hash == hash && slot_key == key {
                        return Ok(idx);
                    }
                }
            }
            idx = (idx + 1) & mask;
        }
        Err(first_tombstone.expect("rehash guarantees at least one non-occupied slot"))
    }

    /// Returns a reference to the value corresponding to the key.
    #[inline]
    pub fn get<Q>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let idx = self.find_slot(k)?;
        match &self.slots[idx] {
            Slot::Occupied { value, .. } => Some(value),
            _ => None,
        }
    }

    /// Returns the key-value pair corresponding to the supplied key.
    #[inline]
    pub fn get_key_value<Q>(&self, k: &Q) -> Option<(&K, &V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let idx = self.find_slot(k)?;
        match &self.slots[idx] {
            Slot::Occupied { key, value, .. } => Some((key, value)),
            _ => None,
        }
    }

    /// Returns `true` if the map contains a value for the specified key.
    #[inline]
    pub fn contains_key<Q>(&self, k: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.find_slot(k).is_some()
    }

    /// Returns a mutable reference to the value corresponding to the key.
    #[inline]
    pub fn get_mut<Q>(&mut self, k: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let idx = self.find_slot(k)?;
        match &mut self.slots[idx] {
            Slot::Occupied { value, .. } => Some(value),
            _ => None,
        }
    }

    /// Inserts a key-value pair into the map, returning the previous value if present.
    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        self.ensure_capacity_for_one();
        let hash = make_hash(&self.hash_builder, &k);
        match self.find_or_insert_slot(hash, &k) {
            Ok(idx) => match &mut self.slots[idx] {
                Slot::Occupied { value, .. } => Some(mem::replace(value, v)),
                _ => unreachable!(),
            },
            Err(idx) => {
                if matches!(self.slots[idx], Slot::Tombstone) {
                    self.tombstones -= 1;
                }
                self.slots[idx] = Slot::Occupied {
                    hash,
                    key: k,
                    value: v,
                };
                self.len += 1;
                None
            }
        }
    }

    /// Removes a key from the map, returning the value at the key if it was present.
    #[inline]
    pub fn remove<Q>(&mut self, k: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.remove_entry(k).map(|(_, v)| v)
    }

    /// Removes a key from the map, returning the stored key and value if present.
    pub fn remove_entry<Q>(&mut self, k: &Q) -> Option<(K, V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let idx = self.find_slot(k)?;
        let prev = mem::replace(&mut self.slots[idx], Slot::Tombstone);
        self.len -= 1;
        self.tombstones += 1;
        match prev {
            Slot::Occupied { key, value, .. } => Some((key, value)),
            _ => unreachable!(),
        }
    }

    /// Gets the given key's corresponding entry in the map for in-place manipulation.
    pub fn entry(&mut self, key: K) -> Entry<'_, K, V> {
        self.ensure_capacity_for_one();
        let hash = make_hash(&self.hash_builder, &key);
        match self.find_or_insert_slot(hash, &key) {
            Ok(index) => Entry::Occupied(OccupiedEntry {
                slots: &mut self.slots,
                len: &mut self.len,
                tombstones: &mut self.tombstones,
                index,
            }),
            Err(index) => Entry::Vacant(VacantEntry {
                slots: &mut self.slots,
                len: &mut self.len,
                tombstones: &mut self.tombstones,
                index,
                hash,
                key,
            }),
        }
    }
}

/// A view into a single entry in a map, which may either be vacant or occupied.
pub enum Entry<'a, K: 'a, V: 'a> {
    /// An occupied entry.
    Occupied(OccupiedEntry<'a, K, V>),
    /// A vacant entry.
    Vacant(VacantEntry<'a, K, V>),
}

/// A view into an occupied entry in a [`HashMap`].
pub struct OccupiedEntry<'a, K: 'a, V: 'a> {
    slots: &'a mut [Slot<K, V>],
    len: &'a mut usize,
    tombstones: &'a mut usize,
    index: usize,
}

/// A view into a vacant entry in a [`HashMap`].
pub struct VacantEntry<'a, K: 'a, V: 'a> {
    slots: &'a mut [Slot<K, V>],
    len: &'a mut usize,
    tombstones: &'a mut usize,
    index: usize,
    hash: u64,
    key: K,
}

impl<'a, K, V> Entry<'a, K, V> {
    /// Ensures a value is in the entry by inserting the default if empty.
    #[inline]
    pub fn or_insert(self, default: V) -> &'a mut V {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(default),
        }
    }

    /// Ensures a value is in the entry by inserting the result of `default` if empty.
    #[inline]
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(default()),
        }
    }

    /// Ensures a value is in the entry by inserting the result of `default(&key)` if empty.
    #[inline]
    pub fn or_insert_with_key<F: FnOnce(&K) -> V>(self, default: F) -> &'a mut V {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let value = default(entry.key());
                entry.insert(value)
            }
        }
    }

    /// Returns a reference to this entry's key.
    #[inline]
    pub fn key(&self) -> &K {
        match self {
            Entry::Occupied(entry) => entry.key(),
            Entry::Vacant(entry) => entry.key(),
        }
    }

    /// Provides in-place mutable access to an occupied entry before any potential inserts.
    #[inline]
    pub fn and_modify<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut V),
    {
        if let Entry::Occupied(ref mut entry) = self {
            f(entry.get_mut());
        }
        self
    }

    /// Sets the value of the entry, returning an [`OccupiedEntry`].
    #[inline]
    pub fn insert_entry(self, value: V) -> OccupiedEntry<'a, K, V> {
        match self {
            Entry::Occupied(mut entry) => {
                entry.insert(value);
                entry
            }
            Entry::Vacant(entry) => entry.insert_entry(value),
        }
    }
}

impl<'a, K, V: Default> Entry<'a, K, V> {
    /// Ensures a value is in the entry by inserting `V::default()` if empty.
    #[inline]
    pub fn or_default(self) -> &'a mut V {
        match self {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(Default::default()),
        }
    }
}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for Entry<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Entry::Vacant(v) => f.debug_tuple("Entry").field(v).finish(),
            Entry::Occupied(o) => f.debug_tuple("Entry").field(o).finish(),
        }
    }
}

impl<'a, K, V> OccupiedEntry<'a, K, V> {
    /// Gets a reference to the key in the entry.
    #[inline]
    pub fn key(&self) -> &K {
        match &self.slots[self.index] {
            Slot::Occupied { key, .. } => key,
            _ => unreachable!(),
        }
    }

    /// Gets a reference to the value in the entry.
    #[inline]
    pub fn get(&self) -> &V {
        match &self.slots[self.index] {
            Slot::Occupied { value, .. } => value,
            _ => unreachable!(),
        }
    }

    /// Gets a mutable reference to the value in the entry.
    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        match &mut self.slots[self.index] {
            Slot::Occupied { value, .. } => value,
            _ => unreachable!(),
        }
    }

    /// Converts the `OccupiedEntry` into a mutable reference to the value in the entry.
    #[inline]
    pub fn into_mut(self) -> &'a mut V {
        match &mut self.slots[self.index] {
            Slot::Occupied { value, .. } => value,
            _ => unreachable!(),
        }
    }

    /// Sets the value of the entry and returns the entry's old value.
    #[inline]
    pub fn insert(&mut self, value: V) -> V {
        mem::replace(self.get_mut(), value)
    }

    /// Removes the key-value pair from the map and returns it.
    #[inline]
    pub fn remove_entry(self) -> (K, V) {
        let prev = mem::replace(&mut self.slots[self.index], Slot::Tombstone);
        *self.len -= 1;
        *self.tombstones += 1;
        match prev {
            Slot::Occupied { key, value, .. } => (key, value),
            _ => unreachable!(),
        }
    }

    /// Takes the value out of the entry and returns it.
    #[inline]
    pub fn remove(self) -> V {
        self.remove_entry().1
    }
}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for OccupiedEntry<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OccupiedEntry")
            .field("key", self.key())
            .field("value", self.get())
            .finish()
    }
}

impl<'a, K, V> VacantEntry<'a, K, V> {
    /// Gets a reference to the key that would be used when inserting a value.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Takes ownership of the key.
    #[inline]
    pub fn into_key(self) -> K {
        self.key
    }

    /// Sets the value of the entry with the `VacantEntry`'s key, returning a mutable reference.
    #[inline]
    pub fn insert(self, value: V) -> &'a mut V {
        self.insert_entry(value).into_mut()
    }

    /// Sets the value of the entry with the `VacantEntry`'s key, returning an [`OccupiedEntry`].
    #[inline]
    pub fn insert_entry(self, value: V) -> OccupiedEntry<'a, K, V> {
        if matches!(self.slots[self.index], Slot::Tombstone) {
            *self.tombstones -= 1;
        }
        self.slots[self.index] = Slot::Occupied {
            hash: self.hash,
            key: self.key,
            value,
        };
        *self.len += 1;
        OccupiedEntry {
            slots: self.slots,
            len: self.len,
            tombstones: self.tombstones,
            index: self.index,
        }
    }
}

impl<K: fmt::Debug, V> fmt::Debug for VacantEntry<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("VacantEntry").field(self.key()).finish()
    }
}

/// An iterator over the entries of a [`HashMap`].
pub struct Iter<'a, K: 'a, V: 'a> {
    inner: crate::slice::Iter<'a, Slot<K, V>>,
    remaining: usize,
}

impl<K, V> Clone for Iter<'_, K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Iter {
            inner: self.inner.clone(),
            remaining: self.remaining,
        }
    }
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<(&'a K, &'a V)> {
        for slot in self.inner.by_ref() {
            if let Slot::Occupied { key, value, .. } = slot {
                self.remaining -= 1;
                return Some((key, value));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> ExactSizeIterator for Iter<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.remaining
    }
}

impl<K, V> FusedIterator for Iter<'_, K, V> {}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for Iter<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// A mutable iterator over the entries of a [`HashMap`].
pub struct IterMut<'a, K: 'a, V: 'a> {
    inner: crate::slice::IterMut<'a, Slot<K, V>>,
    remaining: usize,
}

impl<'a, K, V> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    #[inline]
    fn next(&mut self) -> Option<(&'a K, &'a mut V)> {
        for slot in self.inner.by_ref() {
            if let Slot::Occupied { key, value, .. } = slot {
                self.remaining -= 1;
                return Some((key, value));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> ExactSizeIterator for IterMut<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.remaining
    }
}

impl<K, V> FusedIterator for IterMut<'_, K, V> {}

/// An owning iterator over the entries of a [`HashMap`].
pub struct IntoIter<K, V> {
    inner: crate::vec::IntoIter<Slot<K, V>>,
    remaining: usize,
}

impl<K, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);

    #[inline]
    fn next(&mut self) -> Option<(K, V)> {
        for slot in self.inner.by_ref() {
            if let Slot::Occupied { key, value, .. } = slot {
                self.remaining -= 1;
                return Some((key, value));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> ExactSizeIterator for IntoIter<K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.remaining
    }
}

impl<K, V> FusedIterator for IntoIter<K, V> {}

/// An iterator over the keys of a [`HashMap`].
pub struct Keys<'a, K: 'a, V: 'a> {
    inner: Iter<'a, K, V>,
}

impl<K, V> Clone for Keys<'_, K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Keys {
            inner: self.inner.clone(),
        }
    }
}

impl<'a, K, V> Iterator for Keys<'a, K, V> {
    type Item = &'a K;

    #[inline]
    fn next(&mut self) -> Option<&'a K> {
        self.inner.next().map(|(k, _)| k)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K, V> ExactSizeIterator for Keys<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<K, V> FusedIterator for Keys<'_, K, V> {}

impl<K: fmt::Debug, V> fmt::Debug for Keys<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// An owning iterator over the keys of a [`HashMap`].
pub struct IntoKeys<K, V> {
    inner: IntoIter<K, V>,
}

impl<K, V> Iterator for IntoKeys<K, V> {
    type Item = K;

    #[inline]
    fn next(&mut self) -> Option<K> {
        self.inner.next().map(|(k, _)| k)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K, V> ExactSizeIterator for IntoKeys<K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<K, V> FusedIterator for IntoKeys<K, V> {}

/// An iterator over the values of a [`HashMap`].
pub struct Values<'a, K: 'a, V: 'a> {
    inner: Iter<'a, K, V>,
}

impl<K, V> Clone for Values<'_, K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Values {
            inner: self.inner.clone(),
        }
    }
}

impl<'a, K, V> Iterator for Values<'a, K, V> {
    type Item = &'a V;

    #[inline]
    fn next(&mut self) -> Option<&'a V> {
        self.inner.next().map(|(_, v)| v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K, V> ExactSizeIterator for Values<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<K, V> FusedIterator for Values<'_, K, V> {}

impl<K, V: fmt::Debug> fmt::Debug for Values<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// A mutable iterator over the values of a [`HashMap`].
pub struct ValuesMut<'a, K: 'a, V: 'a> {
    inner: IterMut<'a, K, V>,
}

impl<'a, K, V> Iterator for ValuesMut<'a, K, V> {
    type Item = &'a mut V;

    #[inline]
    fn next(&mut self) -> Option<&'a mut V> {
        self.inner.next().map(|(_, v)| v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K, V> ExactSizeIterator for ValuesMut<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<K, V> FusedIterator for ValuesMut<'_, K, V> {}

/// An owning iterator over the values of a [`HashMap`].
pub struct IntoValues<K, V> {
    inner: IntoIter<K, V>,
}

impl<K, V> Iterator for IntoValues<K, V> {
    type Item = V;

    #[inline]
    fn next(&mut self) -> Option<V> {
        self.inner.next().map(|(_, v)| v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K, V> ExactSizeIterator for IntoValues<K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<K, V> FusedIterator for IntoValues<K, V> {}

/// A draining iterator over the entries of a [`HashMap`].
pub struct Drain<'a, K: 'a, V: 'a> {
    inner: crate::slice::IterMut<'a, Slot<K, V>>,
    remaining: usize,
}

impl<K, V> Iterator for Drain<'_, K, V> {
    type Item = (K, V);

    #[inline]
    fn next(&mut self) -> Option<(K, V)> {
        for slot in self.inner.by_ref() {
            if matches!(slot, Slot::Occupied { .. }) {
                self.remaining -= 1;
                if let Slot::Occupied { key, value, .. } = mem::replace(slot, Slot::Empty) {
                    return Some((key, value));
                }
            } else {
                *slot = Slot::Empty;
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> ExactSizeIterator for Drain<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.remaining
    }
}

impl<K, V> FusedIterator for Drain<'_, K, V> {}

impl<K, V> Drop for Drain<'_, K, V> {
    fn drop(&mut self) {
        for slot in self.inner.by_ref() {
            *slot = Slot::Empty;
        }
    }
}

impl<K: Clone, V: Clone, S: Clone> Clone for HashMap<K, V, S> {
    fn clone(&self) -> HashMap<K, V, S> {
        HashMap {
            slots: self.slots.clone(),
            len: self.len,
            tombstones: self.tombstones,
            hash_builder: self.hash_builder.clone(),
        }
    }
}

impl<K, V, S: Default> Default for HashMap<K, V, S> {
    #[inline]
    fn default() -> HashMap<K, V, S> {
        HashMap::with_hasher(S::default())
    }
}

impl<K, V, S> fmt::Debug for HashMap<K, V, S>
where
    K: fmt::Debug,
    V: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K, V, S> PartialEq for HashMap<K, V, S>
where
    K: Eq + Hash,
    V: PartialEq,
    S: BuildHasher,
{
    fn eq(&self, other: &HashMap<K, V, S>) -> bool {
        if self.len != other.len {
            return false;
        }
        self.iter()
            .all(|(key, value)| other.get(key).is_some_and(|v| *value == *v))
    }
}

impl<K, V, S> Eq for HashMap<K, V, S>
where
    K: Eq + Hash,
    V: Eq,
    S: BuildHasher,
{
}

impl<K, Q, V, S> Index<&Q> for HashMap<K, V, S>
where
    K: Eq + Hash + Borrow<Q>,
    Q: Eq + Hash + ?Sized,
    S: BuildHasher,
{
    type Output = V;

    #[inline]
    fn index(&self, key: &Q) -> &V {
        self.get(key).expect("no entry found for key")
    }
}

impl<K, V, S> IntoIterator for HashMap<K, V, S> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;

    #[inline]
    fn into_iter(self) -> IntoIter<K, V> {
        IntoIter {
            inner: self.slots.into_iter(),
            remaining: self.len,
        }
    }
}

impl<'a, K, V, S> IntoIterator for &'a HashMap<K, V, S> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    #[inline]
    fn into_iter(self) -> Iter<'a, K, V> {
        self.iter()
    }
}

impl<'a, K, V, S> IntoIterator for &'a mut HashMap<K, V, S> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V>;

    #[inline]
    fn into_iter(self) -> IterMut<'a, K, V> {
        self.iter_mut()
    }
}

impl<K, V, S> FromIterator<(K, V)> for HashMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Default,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> HashMap<K, V, S> {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        let mut map = HashMap::with_capacity_and_hasher(lower, S::default());
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

impl<K, V, const N: usize> From<[(K, V); N]> for HashMap<K, V, RandomState>
where
    K: Eq + Hash,
{
    fn from(arr: [(K, V); N]) -> Self {
        Self::from_iter(arr)
    }
}

impl<K, V, S> Extend<(K, V)> for HashMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher,
{
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        self.reserve(lower);
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl<'a, K, V, S> Extend<(&'a K, &'a V)> for HashMap<K, V, S>
where
    K: Eq + Hash + Copy,
    V: Copy,
    S: BuildHasher,
{
    fn extend<T: IntoIterator<Item = (&'a K, &'a V)>>(&mut self, iter: T) {
        for (&k, &v) in iter {
            self.insert(k, v);
        }
    }
}
