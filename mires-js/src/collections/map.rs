//! [`HashMap`], and iterating one.

use core::marker::PhantomData;

use super::key::MapKey;
use super::raw::{self, RawMap};

/// Non-transparent storage cell inside the host `Map` so `&MapEntry<V>` / `&mut MapEntry<V>`
/// always stay indirect (`PointerForm::Thin`) even when `Option<V>` uses null-niche lowering.
struct MapEntry<V> {
    value: Option<V>,
}

/// A hash map that is a host `Map`.
///
/// `repr(transparent)`, so the value the backend keeps in a local of this type is the `Map` object
/// and nothing wraps it: a map moved into a struct, captured by a closure or returned from a
/// function is still the same host object, and two Rust values naming one map are one map. There
/// is no `Drop`, because there is nothing to free.
///
/// Lookup takes anything of the key's own [class](MapKey::Class), so a `HashMap<String, V>` is read
/// with a `&str`.
#[repr(transparent)]
pub struct HashMap<K, V> {
    raw: RawMap,
    marker: PhantomData<(K, V)>,
}

impl<K: MapKey, V> HashMap<K, V> {
    /// An empty map.
    #[must_use]
    pub fn new() -> HashMap<K, V> {
        HashMap {
            raw: raw::map_new(),
            marker: PhantomData,
        }
    }

    /// How many entries the map holds.
    #[must_use]
    pub fn len(&self) -> usize {
        raw::map_len(self.raw)
    }

    /// Whether the map holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        raw::map_clear(self.raw)
    }

    /// Inserts a value, returning the one the key held before.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let class = key.as_class();
        // SAFETY: Every entry in `self.raw` is a `MapEntry<V>`, and `&mut self` grants exclusive
        // access for the duration of this borrow.
        #[allow(unsafe_code)]
        let existing = unsafe { raw::map_try_get_mut::<K::Class, MapEntry<V>>(self.raw, class) };
        if let Some(held) = existing {
            return held.value.replace(value);
        }
        raw::map_set(self.raw, class, MapEntry { value: Some(value) });
        None
    }

    /// The value a key holds.
    #[must_use]
    pub fn get(&self, key: &K::Class) -> Option<&V> {
        // SAFETY: Every entry in `self.raw` is a `MapEntry<V>`, and `&self` prevents concurrent
        // mutation for the returned lifetime.
        #[allow(unsafe_code)]
        let held: &MapEntry<V> = unsafe { raw::map_try_get(self.raw, key) }?;
        held.value.as_ref()
    }

    /// The value a key holds, to write through.
    ///
    /// The reference names the place inside the host `Map`, so a write through it is what the next
    /// [`get`](HashMap::get) reads.
    #[must_use]
    pub fn get_mut(&mut self, key: &K::Class) -> Option<&mut V> {
        // SAFETY: Every entry in `self.raw` is a `MapEntry<V>`, and `&mut self` guarantees unique
        // access for the returned lifetime.
        #[allow(unsafe_code)]
        let held: &mut MapEntry<V> = unsafe { raw::map_try_get_mut(self.raw, key) }?;
        held.value.as_mut()
    }

    /// Removes a key, returning the value it held.
    pub fn remove(&mut self, key: &K::Class) -> Option<V> {
        // SAFETY: Every entry in `self.raw` is a `MapEntry<V>`, `&mut self` guarantees exclusive
        // access, and `held` is not used after `raw::map_del`.
        #[allow(unsafe_code)]
        let held: &mut MapEntry<V> = unsafe { raw::map_try_get_mut(self.raw, key) }?;
        let taken = held.value.take();
        let _ = raw::map_del(self.raw, key);
        taken
    }

    /// Whether a key is in the map.
    #[must_use]
    pub fn contains_key(&self, key: &K::Class) -> bool {
        raw::map_has(self.raw, key)
    }

    /// Every entry, in the host's insertion order.
    ///
    /// Keys and stored entry references are snapshotted in matching insertion order so iteration
    /// performs zero per-entry `Map` lookups. The map cannot change while the iterator is alive.
    #[must_use]
    pub fn iter(&self) -> Iter<'_, K, V> {
        // SAFETY: Every key in `self.raw` snapshots as `K::Snapshot`, and the returned slice
        // borrows from `&self`.
        #[allow(unsafe_code)]
        let keys = unsafe { raw::map_keys::<K::Snapshot>(self.raw) };
        // SAFETY: Every value in `self.raw` is a `MapEntry<V>`, and `&self` prevents mutation
        // while `Iter` is alive.
        #[allow(unsafe_code)]
        let vals = unsafe { raw::map_values::<MapEntry<V>>(self.raw) };
        Iter {
            keys,
            vals,
            index: 0,
            marker: PhantomData,
        }
    }
}

impl<K: MapKey, V> Default for HashMap<K, V> {
    fn default() -> HashMap<K, V> {
        HashMap::new()
    }
}

impl<'a, K: MapKey, V> IntoIterator for &'a HashMap<K, V> {
    type Item = (&'a K::Class, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Iter<'a, K, V> {
        self.iter()
    }
}

/// Every entry of a [`HashMap`], over a snapshot of its keys and stored entries.
///
/// A key is yielded as the [class](MapKey::Class) it compares in, which is `&str` for every string
/// keyed map whatever the key type is written as.
pub struct Iter<'a, K: MapKey, V> {
    keys: &'a [K::Snapshot],
    vals: &'a [&'a MapEntry<V>],
    index: usize,
    marker: PhantomData<&'a K>,
}

impl<'a, K: MapKey, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K::Class, &'a V);

    fn next(&mut self) -> Option<(&'a K::Class, &'a V)> {
        while self.index < self.keys.len() {
            let i = self.index;
            self.index += 1;
            if let Some(value) = self.vals[i].value.as_ref() {
                return Some((K::class_of(&self.keys[i]), value));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.keys.len() - self.index))
    }
}
