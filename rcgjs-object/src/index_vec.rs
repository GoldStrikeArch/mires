//! Strongly typed vector indexed by a newtype index ([`Idx`]).
//!
//! Prevents accidentally indexing a block table with a [`crate::jir::repr::ValueId`] or a value
//! table with a [`crate::jir::repr::CellId`] while keeping the exact memory layout and postcard
//! wire representation of a plain `Vec<T>`.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A dense `usize`-backed newtype index usable with [`IndexVec`].
pub trait Idx: Copy + Eq {
    fn new(idx: usize) -> Self;
    fn index(self) -> usize;
}

/// A `Vec<T>` indexed only by keys of type `I: Idx`.
pub struct IndexVec<I, T> {
    raw: Vec<T>,
    _marker: PhantomData<fn(&I)>,
}

impl<I: Idx, T> IndexVec<I, T> {
    /// Creates an empty `IndexVec`.
    pub const fn new() -> Self {
        Self {
            raw: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Creates an empty `IndexVec` with at least the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            raw: Vec::with_capacity(capacity),
            _marker: PhantomData,
        }
    }

    /// Creates an `IndexVec` of length `len` filled with clones of `elem`.
    pub fn from_elem(elem: T, len: usize) -> Self
    where
        T: Clone,
    {
        Self {
            raw: vec![elem; len],
            _marker: PhantomData,
        }
    }

    /// Creates an `IndexVec` of length `len` by invoking `f` with each index in order.
    pub fn from_fn(len: usize, mut f: impl FnMut(I) -> T) -> Self {
        let mut raw = Vec::with_capacity(len);
        for idx in 0..len {
            raw.push(f(I::new(idx)));
        }
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    /// Appends `value` and returns its newly assigned index.
    pub fn push(&mut self, value: T) -> I {
        let idx = I::new(self.raw.len());
        self.raw.push(value);
        idx
    }

    /// Removes and returns the last element, if any.
    pub fn pop(&mut self) -> Option<T> {
        self.raw.pop()
    }

    /// The index that the next [`IndexVec::push`] will assign.
    pub fn next_idx(&self) -> I {
        I::new(self.raw.len())
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        self.raw.len()
    }

    /// Whether the vector contains no elements.
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Returns a reference to the element at `index`, or `None` if out of bounds.
    pub fn get(&self, index: I) -> Option<&T> {
        self.raw.get(index.index())
    }

    /// Returns a mutable reference to the element at `index`, or `None` if out of bounds.
    pub fn get_mut(&mut self, index: I) -> Option<&mut T> {
        self.raw.get_mut(index.index())
    }

    /// Iterates over elements by shared reference.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.raw.iter()
    }

    /// Iterates over elements by mutable reference.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.raw.iter_mut()
    }

    /// Iterates over `(I, &T)` pairs in index order.
    pub fn iter_enumerated(&self) -> impl DoubleEndedIterator<Item = (I, &T)> + ExactSizeIterator {
        self.raw
            .iter()
            .enumerate()
            .map(|(idx, value)| (I::new(idx), value))
    }

    /// Iterates over `(I, &mut T)` pairs in index order.
    pub fn iter_enumerated_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = (I, &mut T)> + ExactSizeIterator {
        self.raw
            .iter_mut()
            .enumerate()
            .map(|(idx, value)| (I::new(idx), value))
    }

    /// Iterates over all valid indices `0..self.len()`.
    pub fn indices(&self) -> impl DoubleEndedIterator<Item = I> + ExactSizeIterator {
        (0..self.raw.len()).map(I::new)
    }

    /// Borrows the underlying slice.
    pub fn as_slice(&self) -> &[T] {
        &self.raw
    }

    /// Mutably borrows the underlying slice.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.raw
    }

    /// Unwraps into the underlying `Vec<T>`.
    pub fn into_vec(self) -> Vec<T> {
        self.raw
    }

    /// Removes all elements without freeing capacity.
    pub fn clear(&mut self) {
        self.raw.clear();
    }

    /// Reserves capacity for at least `additional` more elements.
    pub fn reserve(&mut self, additional: usize) {
        self.raw.reserve(additional);
    }

    /// Resizes the vector in-place so that `len` is equal to `new_len`.
    pub fn resize(&mut self, new_len: usize, value: T)
    where
        T: Clone,
    {
        self.raw.resize(new_len, value);
    }

    /// Truncates the vector, keeping the first `len` elements.
    pub fn truncate(&mut self, len: usize) {
        self.raw.truncate(len);
    }
}

impl<I: Idx, T> Default for IndexVec<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I, T: Clone> Clone for IndexVec<I, T> {
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            _marker: PhantomData,
        }
    }
}

impl<I, T: fmt::Debug> fmt::Debug for IndexVec<I, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.raw.fmt(f)
    }
}

impl<I, T: PartialEq> PartialEq for IndexVec<I, T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<I, T: Eq> Eq for IndexVec<I, T> {}

impl<I: Idx, T> Index<I> for IndexVec<I, T> {
    type Output = T;

    fn index(&self, index: I) -> &Self::Output {
        &self.raw[index.index()]
    }
}

impl<I: Idx, T> IndexMut<I> for IndexVec<I, T> {
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        &mut self.raw[index.index()]
    }
}

impl<I: Idx, T> From<Vec<T>> for IndexVec<I, T> {
    fn from(raw: Vec<T>) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }
}

impl<I, T> From<IndexVec<I, T>> for Vec<T> {
    fn from(vec: IndexVec<I, T>) -> Self {
        vec.raw
    }
}

impl<I: Idx, T> FromIterator<T> for IndexVec<I, T> {
    fn from_iter<Iter: IntoIterator<Item = T>>(iter: Iter) -> Self {
        Self::from(Vec::from_iter(iter))
    }
}

impl<I: Idx, T> Extend<T> for IndexVec<I, T> {
    fn extend<Iter: IntoIterator<Item = T>>(&mut self, iter: Iter) {
        self.raw.extend(iter);
    }
}

impl<I: Idx, T> IntoIterator for IndexVec<I, T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.raw.into_iter()
    }
}

impl<'a, I: Idx, T> IntoIterator for &'a IndexVec<I, T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.raw.iter()
    }
}

impl<'a, I: Idx, T> IntoIterator for &'a mut IndexVec<I, T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.raw.iter_mut()
    }
}

impl<I, T: Serialize> Serialize for IndexVec<I, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(serializer)
    }
}

impl<'de, I: Idx, T: Deserialize<'de>> Deserialize<'de> for IndexVec<I, T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<T>::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::repr::{CellId, RBlockId, ValueId};
    use crate::jir::semantic::{BlockId, LocalId};

    #[test]
    fn basic_operations_and_iteration() {
        let mut values: IndexVec<ValueId, &str> = IndexVec::with_capacity(4);
        assert!(values.is_empty());
        let v0 = values.push("zero");
        let v1 = values.push("one");
        assert_eq!(v0, ValueId(0));
        assert_eq!(v1, ValueId(1));
        assert_eq!(values.len(), 2);
        assert_eq!(values[v0], "zero");
        assert_eq!(values.get(v1), Some(&"one"));
        assert_eq!(values.get(ValueId(2)), None);

        *values.get_mut(v1).unwrap() = "ONE";
        assert_eq!(values[v1], "ONE");

        let pairs: Vec<_> = values.iter_enumerated().map(|(i, s)| (i, *s)).collect();
        assert_eq!(pairs, vec![(ValueId(0), "zero"), (ValueId(1), "ONE")]);
        assert_eq!(values.indices().collect::<Vec<_>>(), vec![v0, v1]);

        for (idx, slot) in values.iter_enumerated_mut() {
            if idx == v0 {
                *slot = "ZERO";
            }
        }
        assert_eq!(values.as_slice(), &["ZERO", "ONE"]);
    }

    #[test]
    fn from_elem_and_serde_match_vec_wire_format() {
        let cells: IndexVec<CellId, u32> = IndexVec::from_elem(42, 3);
        assert_eq!(cells.as_slice(), &[42, 42, 42]);

        let raw = vec![10u32, 20, 30];
        let indexed: IndexVec<RBlockId, u32> = IndexVec::from(raw.clone());
        let raw_bytes = postcard::to_allocvec(&raw).unwrap();
        let indexed_bytes = postcard::to_allocvec(&indexed).unwrap();
        assert_eq!(raw_bytes, indexed_bytes);
        let round_tripped: IndexVec<RBlockId, u32> = postcard::from_bytes(&indexed_bytes).unwrap();
        assert_eq!(round_tripped, indexed);

        let locals: IndexVec<LocalId, u32> = IndexVec::from_fn(2, |id: LocalId| id.0 * 10);
        let blocks: IndexVec<BlockId, u32> = IndexVec::from_fn(2, |id: BlockId| id.0 + 1);
        assert_eq!(locals[LocalId(1)], 10);
        assert_eq!(blocks[BlockId(1)], 2);
    }
}
