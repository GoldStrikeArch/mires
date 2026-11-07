//! [`HashMap`], and iterating one.

use core::marker::PhantomData;

use super::key::MapKey;
use super::raw::{self, RawMap};

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
            raw: unsafe { raw::map_new() },
            marker: PhantomData,
        }
    }

    /// How many entries the map holds.
    #[must_use]
    pub fn len(&self) -> usize {
        unsafe { raw::map_len(self.raw) }
    }

    /// Whether the map holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        unsafe { raw::map_clear(self.raw) }
    }

    /// Inserts a value, returning the one the key held before.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let class = key.as_class();
        if raw::map_has(self.raw, class) {
            // Safe by the test above, and the borrow ends inside this arm.
            let held: &mut Option<V> = unsafe { raw::map_get_mut(self.raw, class) };
            return held.replace(value);
        }
        raw::map_set(self.raw, class, Some(value));
        None
    }

    /// The value a key holds.
    #[must_use]
    pub fn get(&self, key: &K::Class) -> Option<&V> {
        if !raw::map_has(self.raw, key) {
            return None;
        }
        // Safe by the test above. The reference is taken at the lifetime of this borrow of the
        // map, which is what keeps it from outliving a `clear` or a `remove`.
        let held: &Option<V> = unsafe { raw::map_get(self.raw, key) };
        held.as_ref()
    }

    /// The value a key holds, to write through.
    ///
    /// The reference names the place inside the host `Map`, so a write through it is what the next
    /// [`get`](HashMap::get) reads.
    #[must_use]
    pub fn get_mut(&mut self, key: &K::Class) -> Option<&mut V> {
        if !raw::map_has(self.raw, key) {
            return None;
        }
        // Safe by the test above, and unique because `&mut self` is.
        let held: &mut Option<V> = unsafe { raw::map_get_mut(self.raw, key) };
        held.as_mut()
    }

    /// Removes a key, returning the value it held.
    pub fn remove(&mut self, key: &K::Class) -> Option<V> {
        if !raw::map_has(self.raw, key) {
            return None;
        }
        // Safe by the test above, and unique because `&mut self` is. The value is taken out before
        // the entry goes, so nothing is left owning it twice.
        let held: &mut Option<V> = unsafe { raw::map_get_mut(self.raw, key) };
        let taken = held.take();
        raw::map_del(self.raw, key);
        taken
    }

    /// Whether a key is in the map.
    #[must_use]
    pub fn contains_key(&self, key: &K::Class) -> bool {
        raw::map_has(self.raw, key)
    }

    /// Every entry, in the host's insertion order.
    ///
    /// The keys are snapshotted into a host array first, so this costs one array of `n` keys and
    /// one lookup per key. The map cannot change while the iterator is alive.
    #[must_use]
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            map: self,
            keys: raw::map_keys::<K::Snapshot>(self.raw),
            index: 0,
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

/// Every entry of a [`HashMap`], over a snapshot of its keys.
///
/// A key is yielded as the [class](MapKey::Class) it compares in, which is `&str` for every string
/// keyed map whatever the key type is written as.
pub struct Iter<'a, K: MapKey, V> {
    map: &'a HashMap<K, V>,
    keys: &'a [K::Snapshot],
    index: usize,
}

impl<'a, K: MapKey, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K::Class, &'a V);

    fn next(&mut self) -> Option<(&'a K::Class, &'a V)> {
        let keys = self.keys;
        while self.index < keys.len() {
            let snapshot: &'a K::Snapshot = &keys[self.index];
            self.index += 1;
            let key = K::class_of(snapshot);
            if let Some(value) = self.map.get(key) {
                return Some((key, value));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.keys.len() - self.index))
    }
}
