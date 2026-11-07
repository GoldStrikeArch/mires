//! [`HashSet`], and iterating one.

use super::key::MapKey;
use super::map::HashMap;

/// A hash set that is a host `Map` whose values are all the unit.
///
/// A `Map` rather than a host `Set` so that one shim and one marker set serve both, and because
/// nothing a set does needs anything a map does not already have.
#[repr(transparent)]
pub struct HashSet<T> {
    map: HashMap<T, ()>,
}

impl<T: MapKey> HashSet<T> {
    /// An empty set.
    #[must_use]
    pub fn new() -> HashSet<T> {
        HashSet {
            map: HashMap::new(),
        }
    }

    /// How many values the set holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the set holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Removes every value.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Adds a value, answering whether it was new.
    pub fn insert(&mut self, value: T) -> bool {
        self.map.insert(value, ()).is_none()
    }

    /// Whether a value is in the set.
    #[must_use]
    pub fn contains(&self, value: &T::Class) -> bool {
        self.map.contains_key(value)
    }

    /// Removes a value, answering whether it was there.
    pub fn remove(&mut self, value: &T::Class) -> bool {
        self.map.remove(value).is_some()
    }

    /// Every value, in the host's insertion order.
    ///
    /// Over a snapshot of the keys, exactly as [`HashMap::iter`] is.
    #[must_use]
    pub fn iter(&self) -> SetIter<'_, T> {
        SetIter {
            entries: self.map.iter(),
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
    entries: super::map::Iter<'a, T, ()>,
}

impl<'a, T: MapKey> Iterator for SetIter<'a, T> {
    type Item = &'a T::Class;

    fn next(&mut self) -> Option<&'a T::Class> {
        self.entries.next().map(|(value, _)| value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}
