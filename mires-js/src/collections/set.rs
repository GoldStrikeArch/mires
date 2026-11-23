//! [`HashSet`], and iterating one.

use core::marker::PhantomData;

use super::key::MapKey;
use super::raw::{self, RawMap};

/// A hash set that is a host `Map` whose values are booleans.
///
/// Using the host `Map` directly avoids `Option<()>` wrapper allocations on insert and eliminates
/// per-entry lookups during iteration.
#[repr(transparent)]
pub struct HashSet<T> {
    raw: RawMap,
    marker: PhantomData<T>,
}

impl<T: MapKey> HashSet<T> {
    /// An empty set.
    #[must_use]
    pub fn new() -> HashSet<T> {
        HashSet {
            raw: raw::map_new(),
            marker: PhantomData,
        }
    }

    /// How many values the set holds.
    #[must_use]
    pub fn len(&self) -> usize {
        raw::map_len(self.raw)
    }

    /// Whether the set holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes every value.
    pub fn clear(&mut self) {
        raw::map_clear(self.raw)
    }

    /// Adds a value, answering whether it was new.
    pub fn insert(&mut self, value: T) -> bool {
        let class = value.as_class();
        if raw::map_has(self.raw, class) {
            return false;
        }
        raw::map_set(self.raw, class, true);
        true
    }

    /// Whether a value is in the set.
    #[must_use]
    pub fn contains(&self, value: &T::Class) -> bool {
        raw::map_has(self.raw, value)
    }

    /// Removes a value, answering whether it was there.
    pub fn remove(&mut self, value: &T::Class) -> bool {
        raw::map_del(self.raw, value)
    }

    /// Every value, in the host's insertion order.
    ///
    /// Over a snapshot of the keys, with zero per-entry map lookups.
    #[must_use]
    pub fn iter(&self) -> SetIter<'_, T> {
        SetIter {
            keys: raw::map_keys::<T::Snapshot>(self.raw),
            index: 0,
            marker: PhantomData,
        }
    }
}

impl<T: MapKey> Default for HashSet<T> {
    fn default() -> HashSet<T> {
        HashSet::new()
    }
}

impl<'a, T: MapKey> IntoIterator for &'a HashSet<T> {
    type Item = &'a T::Class;
    type IntoIter = SetIter<'a, T>;

    fn into_iter(self) -> SetIter<'a, T> {
        self.iter()
    }
}

/// Every value of a [`HashSet`], over a snapshot of them.
///
/// Named apart from [`crate::collections::Iter`] because both are re-exported from one module.
pub struct SetIter<'a, T: MapKey> {
    keys: &'a [T::Snapshot],
    index: usize,
    marker: PhantomData<&'a T>,
}

impl<'a, T: MapKey> Iterator for SetIter<'a, T> {
    type Item = &'a T::Class;

    fn next(&mut self) -> Option<&'a T::Class> {
        if self.index < self.keys.len() {
            let i = self.index;
            self.index += 1;
            return Some(T::class_of(&self.keys[i]));
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.keys.len() - self.index;
        (rem, Some(rem))
    }
}
