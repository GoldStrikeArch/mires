//! Pure-Rust `HashSet<T, S = RandomState>` implemented over [`HashMap`].

use crate::borrow::Borrow;
use crate::collections::hash_map::{self, HashMap, RandomState};
use crate::fmt;
use crate::hash::{BuildHasher, Hash};
use crate::iter::{Chain, FusedIterator};
use crate::ops::{BitAnd, BitOr, BitXor, Sub};

/// A hash set implemented as a [`HashMap`] where the value is `()`.
pub struct HashSet<T, S = RandomState> {
    map: HashMap<T, (), S>,
}

impl<T> HashSet<T, RandomState> {
    /// Creates an empty `HashSet`.
    #[must_use]
    #[inline]
    pub fn new() -> HashSet<T, RandomState> {
        HashSet {
            map: HashMap::new(),
        }
    }

    /// Creates an empty `HashSet` with at least the specified capacity.
    #[must_use]
    #[inline]
    pub fn with_capacity(capacity: usize) -> HashSet<T, RandomState> {
        HashSet {
            map: HashMap::with_capacity(capacity),
        }
    }
}

impl<T, S> HashSet<T, S> {
    /// Creates a new empty hash set which will use the given hash builder.
    #[must_use]
    #[inline]
    pub const fn with_hasher(hasher: S) -> HashSet<T, S> {
        HashSet {
            map: HashMap::with_hasher(hasher),
        }
    }

    /// Creates an empty `HashSet` with at least the specified capacity and hash builder.
    #[must_use]
    #[inline]
    pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> HashSet<T, S> {
        HashSet {
            map: HashMap::with_capacity_and_hasher(capacity, hasher),
        }
    }

    /// Returns the number of elements the set can hold without reallocating.
    #[must_use]
    #[inline]
    pub fn capacity(&self) -> usize {
        self.map.capacity()
    }

    /// An iterator visiting all elements in arbitrary order.
    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            iter: self.map.keys(),
        }
    }

    /// Returns the number of elements in the set.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if the set contains no elements.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Clears the set, returning all elements as an iterator.
    #[inline]
    pub fn drain(&mut self) -> Drain<'_, T> {
        Drain {
            iter: self.map.drain(),
        }
    }

    /// Retains only the elements specified by the predicate.
    #[inline]
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.map.retain(|k, _| f(k));
    }

    /// Clears the set, removing all values.
    #[inline]
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Returns a reference to the set's [`BuildHasher`].
    #[must_use]
    #[inline]
    pub fn hasher(&self) -> &S {
        self.map.hasher()
    }
}

impl<T, S> HashSet<T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
    /// Reserves capacity for at least `additional` more elements.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.map.reserve(additional);
    }

    /// Tries to reserve capacity for at least `additional` more elements.
    #[inline]
    pub fn try_reserve(
        &mut self,
        additional: usize,
    ) -> Result<(), crate::collections::TryReserveError> {
        self.map.try_reserve(additional)
    }

    /// Shrinks the capacity of the set as much as possible.
    #[inline]
    pub fn shrink_to_fit(&mut self) {
        self.map.shrink_to_fit();
    }

    /// Shrinks the capacity of the set with a lower limit.
    #[inline]
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.map.shrink_to(min_capacity);
    }

    /// Visits the values representing the difference, i.e., the values in `self` but not in `other`.
    #[inline]
    pub fn difference<'a>(&'a self, other: &'a HashSet<T, S>) -> Difference<'a, T, S> {
        Difference {
            iter: self.iter(),
            other,
        }
    }

    /// Visits the values representing the symmetric difference.
    #[inline]
    pub fn symmetric_difference<'a>(
        &'a self,
        other: &'a HashSet<T, S>,
    ) -> SymmetricDifference<'a, T, S> {
        SymmetricDifference {
            iter: self.difference(other).chain(other.difference(self)),
        }
    }

    /// Visits the values representing the intersection, i.e., values in both `self` and `other`.
    #[inline]
    pub fn intersection<'a>(&'a self, other: &'a HashSet<T, S>) -> Intersection<'a, T, S> {
        let (smaller, larger) = if self.len() <= other.len() {
            (self, other)
        } else {
            (other, self)
        };
        Intersection {
            iter: smaller.iter(),
            other: larger,
        }
    }

    /// Visits the values representing the union, i.e., all values in `self` or `other`.
    #[inline]
    pub fn union<'a>(&'a self, other: &'a HashSet<T, S>) -> Union<'a, T, S> {
        if self.len() >= other.len() {
            Union {
                iter: self.iter().chain(other.difference(self)),
            }
        } else {
            Union {
                iter: other.iter().chain(self.difference(other)),
            }
        }
    }

    /// Returns `true` if the set contains a value.
    #[inline]
    pub fn contains<Q>(&self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(value)
    }

    /// Returns a reference to the value in the set, if any, that is equal to the given value.
    #[inline]
    pub fn get<Q>(&self, value: &Q) -> Option<&T>
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.get_key_value(value).map(|(k, _)| k)
    }

    /// Returns `true` if `self` has no elements in common with `other`.
    #[inline]
    pub fn is_disjoint(&self, other: &HashSet<T, S>) -> bool {
        if self.len() <= other.len() {
            self.iter().all(|v| !other.contains(v))
        } else {
            other.iter().all(|v| !self.contains(v))
        }
    }

    /// Returns `true` if the set is a subset of `other`.
    #[inline]
    pub fn is_subset(&self, other: &HashSet<T, S>) -> bool {
        if self.len() > other.len() {
            return false;
        }
        self.iter().all(|v| other.contains(v))
    }

    /// Returns `true` if the set is a superset of `other`.
    #[inline]
    pub fn is_superset(&self, other: &HashSet<T, S>) -> bool {
        other.is_subset(self)
    }

    /// Adds a value to the set. Returns `true` if the value was newly inserted.
    #[inline]
    pub fn insert(&mut self, value: T) -> bool {
        self.map.insert(value, ()).is_none()
    }

    /// Adds a value to the set, replacing the existing value if equal, returning the old value.
    #[inline]
    pub fn replace(&mut self, value: T) -> Option<T> {
        let old = self.take(&value);
        self.map.insert(value, ());
        old
    }

    /// Removes a value from the set. Returns `true` if the value was present.
    #[inline]
    pub fn remove<Q>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.remove(value).is_some()
    }

    /// Removes and returns the value in the set, if any, that is equal to the given one.
    #[inline]
    pub fn take<Q>(&mut self, value: &Q) -> Option<T>
    where
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.remove_entry(value).map(|(k, _)| k)
    }
}

/// An iterator over the items of a [`HashSet`].
pub struct Iter<'a, K: 'a> {
    iter: hash_map::Keys<'a, K, ()>,
}

impl<K> Clone for Iter<'_, K> {
    #[inline]
    fn clone(&self) -> Self {
        Iter {
            iter: self.iter.clone(),
        }
    }
}

impl<'a, K> Iterator for Iter<'a, K> {
    type Item = &'a K;

    #[inline]
    fn next(&mut self) -> Option<&'a K> {
        self.iter.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<K> ExactSizeIterator for Iter<'_, K> {
    #[inline]
    fn len(&self) -> usize {
        self.iter.len()
    }
}

impl<K> FusedIterator for Iter<'_, K> {}

impl<K: fmt::Debug> fmt::Debug for Iter<'_, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// An owning iterator over the items of a [`HashSet`].
pub struct IntoIter<K> {
    iter: hash_map::IntoKeys<K, ()>,
}

impl<K> Iterator for IntoIter<K> {
    type Item = K;

    #[inline]
    fn next(&mut self) -> Option<K> {
        self.iter.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<K> ExactSizeIterator for IntoIter<K> {
    #[inline]
    fn len(&self) -> usize {
        self.iter.len()
    }
}

impl<K> FusedIterator for IntoIter<K> {}

/// A draining iterator over the items of a [`HashSet`].
pub struct Drain<'a, K: 'a> {
    iter: hash_map::Drain<'a, K, ()>,
}

impl<K> Iterator for Drain<'_, K> {
    type Item = K;

    #[inline]
    fn next(&mut self) -> Option<K> {
        self.iter.next().map(|(k, _)| k)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<K> ExactSizeIterator for Drain<'_, K> {
    #[inline]
    fn len(&self) -> usize {
        self.iter.len()
    }
}

impl<K> FusedIterator for Drain<'_, K> {}

/// A lazy iterator producing elements in the difference of `HashSet`s.
pub struct Difference<'a, T: 'a, S: 'a> {
    iter: Iter<'a, T>,
    other: &'a HashSet<T, S>,
}

impl<T, S> Clone for Difference<'_, T, S> {
    #[inline]
    fn clone(&self) -> Self {
        Difference {
            iter: self.iter.clone(),
            other: self.other,
        }
    }
}

impl<'a, T, S> Iterator for Difference<'a, T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        self.iter.by_ref().find(|&elt| !self.other.contains(elt))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (_, upper) = self.iter.size_hint();
        (0, upper)
    }
}

impl<T: Eq + Hash, S: BuildHasher> FusedIterator for Difference<'_, T, S> {}

/// A lazy iterator producing elements in the intersection of `HashSet`s.
pub struct Intersection<'a, T: 'a, S: 'a> {
    iter: Iter<'a, T>,
    other: &'a HashSet<T, S>,
}

impl<T, S> Clone for Intersection<'_, T, S> {
    #[inline]
    fn clone(&self) -> Self {
        Intersection {
            iter: self.iter.clone(),
            other: self.other,
        }
    }
}

impl<'a, T, S> Iterator for Intersection<'a, T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        self.iter.by_ref().find(|&elt| self.other.contains(elt))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (_, upper) = self.iter.size_hint();
        (0, upper)
    }
}

impl<T: Eq + Hash, S: BuildHasher> FusedIterator for Intersection<'_, T, S> {}

/// A lazy iterator producing elements in the symmetric difference of `HashSet`s.
pub struct SymmetricDifference<'a, T: 'a, S: 'a> {
    iter: Chain<Difference<'a, T, S>, Difference<'a, T, S>>,
}

impl<T, S> Clone for SymmetricDifference<'_, T, S> {
    #[inline]
    fn clone(&self) -> Self {
        SymmetricDifference {
            iter: self.iter.clone(),
        }
    }
}

impl<'a, T, S> Iterator for SymmetricDifference<'a, T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        self.iter.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<T: Eq + Hash, S: BuildHasher> FusedIterator for SymmetricDifference<'_, T, S> {}

/// A lazy iterator producing elements in the union of `HashSet`s.
pub struct Union<'a, T: 'a, S: 'a> {
    iter: Chain<Iter<'a, T>, Difference<'a, T, S>>,
}

impl<T, S> Clone for Union<'_, T, S> {
    #[inline]
    fn clone(&self) -> Self {
        Union {
            iter: self.iter.clone(),
        }
    }
}

impl<'a, T, S> Iterator for Union<'a, T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        self.iter.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<T: Eq + Hash, S: BuildHasher> FusedIterator for Union<'_, T, S> {}

impl<T: Clone, S: Clone> Clone for HashSet<T, S> {
    fn clone(&self) -> HashSet<T, S> {
        HashSet {
            map: self.map.clone(),
        }
    }
}

impl<T, S: Default> Default for HashSet<T, S> {
    #[inline]
    fn default() -> HashSet<T, S> {
        HashSet {
            map: HashMap::default(),
        }
    }
}

impl<T: fmt::Debug, S> fmt::Debug for HashSet<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<T, S> PartialEq for HashSet<T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
    fn eq(&self, other: &HashSet<T, S>) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.iter().all(|key| other.contains(key))
    }
}

impl<T, S> Eq for HashSet<T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
}

impl<T, S> IntoIterator for HashSet<T, S> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    #[inline]
    fn into_iter(self) -> IntoIter<T> {
        IntoIter {
            iter: self.map.into_keys(),
        }
    }
}

impl<'a, T, S> IntoIterator for &'a HashSet<T, S> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    #[inline]
    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<T, S> FromIterator<T> for HashSet<T, S>
where
    T: Eq + Hash,
    S: BuildHasher + Default,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> HashSet<T, S> {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        let mut set = HashSet::with_capacity_and_hasher(lower, S::default());
        for value in iter {
            set.insert(value);
        }
        set
    }
}

impl<T, const N: usize> From<[T; N]> for HashSet<T, RandomState>
where
    T: Eq + Hash,
{
    fn from(arr: [T; N]) -> Self {
        Self::from_iter(arr)
    }
}

impl<T, S> Extend<T> for HashSet<T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for value in iter {
            self.insert(value);
        }
    }
}

impl<'a, T, S> Extend<&'a T> for HashSet<T, S>
where
    T: Eq + Hash + Copy + 'a,
    S: BuildHasher,
{
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, iter: I) {
        for &value in iter {
            self.insert(value);
        }
    }
}

impl<T, S> BitOr<&HashSet<T, S>> for &HashSet<T, S>
where
    T: Eq + Hash + Clone,
    S: BuildHasher + Default,
{
    type Output = HashSet<T, S>;

    fn bitor(self, rhs: &HashSet<T, S>) -> HashSet<T, S> {
        self.union(rhs).cloned().collect()
    }
}

impl<T, S> BitAnd<&HashSet<T, S>> for &HashSet<T, S>
where
    T: Eq + Hash + Clone,
    S: BuildHasher + Default,
{
    type Output = HashSet<T, S>;

    fn bitand(self, rhs: &HashSet<T, S>) -> HashSet<T, S> {
        self.intersection(rhs).cloned().collect()
    }
}

impl<T, S> BitXor<&HashSet<T, S>> for &HashSet<T, S>
where
    T: Eq + Hash + Clone,
    S: BuildHasher + Default,
{
    type Output = HashSet<T, S>;

    fn bitxor(self, rhs: &HashSet<T, S>) -> HashSet<T, S> {
        self.symmetric_difference(rhs).cloned().collect()
    }
}

impl<T, S> Sub<&HashSet<T, S>> for &HashSet<T, S>
where
    T: Eq + Hash + Clone,
    S: BuildHasher + Default,
{
    type Output = HashSet<T, S>;

    fn sub(self, rhs: &HashSet<T, S>) -> HashSet<T, S> {
        self.difference(rhs).cloned().collect()
    }
}
