//! MiniSearch's `SearchableMap`: a radix tree over UTF-16 keys.
//!
//! A node is what MiniSearch's node is, an insertion-ordered map: edge labels to child nodes, and
//! the empty label (`LEAF`) to the value of the key that ends at the node. The order of a node's
//! entries is observable (iteration, prefix and fuzzy results), so every operation keeps exactly
//! the order a JavaScript `Map` would: `set` of a new key appends, `delete` removes in place, and
//! replacing an edge (a split in `create_path`, a merge after a removal) is `set` of the new edge
//! followed by `delete` of the old one, which moves it to the end.
//!
//! Labels are compared and split on code units, so an edge may end inside a surrogate pair.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

pub struct SearchableMap<T> {
    root: Node<T>,
}

struct Node<T> {
    entries: Vec<Entry<T>>,
}

enum Entry<T> {
    /// The `''` key: the value of the key ending here.
    Leaf(T),
    /// A non-empty label and the subtree below it. No two edges of a node share a first unit.
    /// Labels never change once made (a split or a merge makes new ones), hence a boxed slice.
    Edge { label: Box<[u16]>, child: Node<T> },
}

impl<T> Node<T> {
    const fn new() -> Self {
        Node { entries: Vec::new() }
    }

    fn leaf_index(&self) -> Option<usize> {
        self.entries.iter().position(|entry| matches!(entry, Entry::Leaf(_)))
    }

    fn leaf(&self) -> Option<&T> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::Leaf(value) => Some(value),
            Entry::Edge { .. } => None,
        })
    }

    fn leaf_mut(&mut self) -> Option<&mut T> {
        self.entries.iter_mut().find_map(|entry| match entry {
            Entry::Leaf(value) => Some(value),
            Entry::Edge { .. } => None,
        })
    }

    /// The edge whose label starts with `unit`.
    fn edge_starting_with(&self, unit: u16) -> Option<usize> {
        self.entries.iter().position(|entry| matches!(entry, Entry::Edge { label, .. } if label[0] == unit))
    }

    /// The edge whose label is a prefix of `key` (`key.startsWith(k)`), with its label length.
    fn edge_prefix_of(&self, key: &[u16]) -> Option<(usize, &Node<T>)> {
        let index = self.edge_starting_with(*key.first()?)?;
        match &self.entries[index] {
            Entry::Edge { label, child } if key.starts_with(label) => Some((label.len(), child)),
            _ => None,
        }
    }
}

impl<T> SearchableMap<T> {
    pub const fn new() -> Self {
        SearchableMap { root: Node::new() }
    }

    pub fn get(&self, key: &[u16]) -> Option<&T> {
        lookup(&self.root, key).and_then(Node::leaf)
    }

    pub fn get_mut(&mut self, key: &[u16]) -> Option<&mut T> {
        lookup_mut(&mut self.root, key).and_then(Node::leaf_mut)
    }

    pub fn has(&self, key: &[u16]) -> bool {
        self.get(key).is_some()
    }

    /// The value at `key`, inserting `initial()` first when there is none (`fetch`).
    pub fn fetch(&mut self, key: &[u16], initial: impl FnOnce() -> T) -> &mut T {
        let node = create_path(&mut self.root, key);
        let index = match node.leaf_index() {
            Some(index) => index,
            None => {
                node.entries.push(Entry::Leaf(initial()));
                node.entries.len() - 1
            }
        };
        match &mut node.entries[index] {
            Entry::Leaf(value) => value,
            Entry::Edge { .. } => unreachable!(),
        }
    }

    /// Removes `key` (`delete`), cleaning up and merging nodes on the way back up.
    pub fn delete(&mut self, key: &[u16]) {
        // The root has no parent: what it would ask of one is nothing (`cleanup([])`, `merge([])`).
        let _ = remove(&mut self.root, key);
    }

    /// The number of keys (`size`).
    pub fn size(&self) -> usize {
        fn count<T>(node: &Node<T>) -> usize {
            node.entries.iter().map(|entry| match entry {
                Entry::Leaf(_) => 1,
                Entry::Edge { child, .. } => count(child),
            }).sum()
        }
        count(&self.root)
    }

    /// Every entry in `TreeIterator` order: depth first, each node's entries from the most recently
    /// inserted to the oldest (the iterator pops its snapshot of the keys from the end).
    pub fn for_each<'a>(&'a self, mut visit: impl FnMut(&[u16], &'a T)) {
        walk(&self.root, &mut Vec::new(), &mut visit);
    }

    /// Every entry of `atPrefix(prefix)`, in `TreeIterator` order. When the prefix ends inside an
    /// edge, the view is a node holding only the rest of that edge.
    pub fn for_each_with_prefix<'a>(&'a self, prefix: &[u16], mut visit: impl FnMut(&[u16], &'a T)) {
        let mut node = &self.root;
        let mut rest = prefix;
        let mut key = prefix.to_vec();
        while !rest.is_empty() {
            match node.edge_prefix_of(rest) {
                Some((length, child)) => {
                    node = child;
                    rest = &rest[length..];
                }
                None => {
                    if let Some(index) = node.edge_starting_with(rest[0]) {
                        if let Entry::Edge { label, child } = &node.entries[index] {
                            if label.starts_with(rest) {
                                key.extend_from_slice(&label[rest.len()..]);
                                walk(child, &mut key, &mut visit);
                            }
                        }
                    }
                    return;
                }
            }
        }
        walk(node, &mut key, &mut visit);
    }

    /// Every key within `max_distance` (Levenshtein, over code units) of `query`, with its
    /// distance, in the order the walk meets them: depth first, each node's entries oldest first.
    pub fn fuzzy_get(&self, query: &[u16], max_distance: usize) -> Vec<(Vec<u16>, u8)> {
        // The Levenshtein matrix: one column per query prefix, one row per key prefix up to the
        // longest key that can still match. MiniSearch keeps it in a Uint8Array, so every store
        // is reduced modulo 256 (inside the parity domain no value reaches 256).
        let columns = query.len() + 1;
        let rows = columns + max_distance;
        let mut matrix = vec![(max_distance + 1) as u8; rows * columns];
        for column in 0..columns {
            matrix[column] = column as u8;
        }
        for row in 1..rows {
            matrix[row * columns] = row as u8;
        }
        let mut fuzzy = Fuzzy { query, max_distance, columns, rows, matrix, prefix: Vec::new(), results: Vec::new() };
        fuzzy.recurse(&self.root, 1);
        fuzzy.results
    }
}

fn lookup<'a, T>(node: &'a Node<T>, key: &[u16]) -> Option<&'a Node<T>> {
    if key.is_empty() {
        return Some(node);
    }
    let (length, child) = node.edge_prefix_of(key)?;
    lookup(child, &key[length..])
}

fn lookup_mut<'a, T>(node: &'a mut Node<T>, key: &[u16]) -> Option<&'a mut Node<T>> {
    if key.is_empty() {
        return Some(node);
    }
    let index = node.edge_starting_with(key[0])?;
    match &mut node.entries[index] {
        Entry::Edge { label, child } if key.starts_with(label) => {
            let length = label.len();
            lookup_mut(child, &key[length..])
        }
        _ => None,
    }
}

/// `createPath`: the node `key` ends at, created (splitting an edge where `key` leaves it) when
/// it does not exist.
fn create_path<'a, T>(node: &'a mut Node<T>, key: &[u16]) -> &'a mut Node<T> {
    if key.is_empty() {
        return node;
    }
    let Some(index) = node.edge_starting_with(key[0]) else {
        node.entries.push(Entry::Edge { label: boxed(key), child: Node::new() });
        return last_child(node);
    };
    let Entry::Edge { label, .. } = &node.entries[index] else { unreachable!() };
    let limit = key.len().min(label.len());
    let mut offset = 1;
    while offset < limit && key[offset] == label[offset] {
        offset += 1;
    }
    if offset < label.len() {
        // Partial match: an intermediate node holds the rest of the old edge. It is set under the
        // shared part and the old edge deleted, so it moves to the end of the node.
        let Entry::Edge { label, child } = node.entries.remove(index) else { unreachable!() };
        let intermediate = Node { entries: vec![Entry::Edge { label: boxed(&label[offset..]), child }] };
        node.entries.push(Entry::Edge { label: boxed(&label[..offset]), child: intermediate });
        return create_path(last_child(node), &key[offset..]);
    }
    match &mut node.entries[index] {
        Entry::Edge { child, .. } => create_path(child, &key[offset..]),
        Entry::Leaf(_) => unreachable!(),
    }
}

/// A label of these units. Through a `Vec`: a `Box<[u16]>` made by `From<&[u16]>` cannot be freed
/// under mires (NOTES.md, bug 4).
fn boxed(units: &[u16]) -> Box<[u16]> {
    units.to_vec().into_boxed_slice()
}

fn last_child<T>(node: &mut Node<T>) -> &mut Node<T> {
    match node.entries.last_mut() {
        Some(Entry::Edge { child, .. }) => child,
        _ => unreachable!(),
    }
}

/// What a node asks of its parent after a removal below it.
enum Removal {
    Nothing,
    /// The node is empty: delete the edge to it (`cleanup`).
    Cleanup,
    /// The node holds a single edge: replace the edge to it by the two labels joined (`merge`).
    Merge,
}

/// `remove`: `trackDown` to the key's node, delete its leaf, then `cleanup` / `merge` upwards.
fn remove<T>(node: &mut Node<T>, key: &[u16]) -> Removal {
    if key.is_empty() {
        if let Some(index) = node.leaf_index() {
            node.entries.remove(index);
        }
        return match node.entries.len() {
            0 => Removal::Cleanup,
            1 => Removal::Merge,
            _ => Removal::Nothing,
        };
    }
    let Some(index) = node.edge_starting_with(key[0]) else { return Removal::Nothing };
    let Entry::Edge { label, child } = &mut node.entries[index] else { unreachable!() };
    if !key.starts_with(label) {
        return Removal::Nothing;
    }
    let length = label.len();
    match remove(child, &key[length..]) {
        Removal::Nothing => Removal::Nothing,
        Removal::Cleanup => {
            node.entries.remove(index);
            match node.entries.as_slice() {
                [] => Removal::Cleanup,
                [Entry::Edge { .. }] => Removal::Merge,
                _ => Removal::Nothing,
            }
        }
        Removal::Merge => {
            let Entry::Edge { label, mut child } = node.entries.remove(index) else { unreachable!() };
            let Some(Entry::Edge { label: rest, child: grandchild }) = child.entries.pop() else { unreachable!() };
            let mut joined = Vec::with_capacity(label.len() + rest.len());
            joined.extend_from_slice(&label);
            joined.extend_from_slice(&rest);
            node.entries.push(Entry::Edge { label: joined.into_boxed_slice(), child: grandchild });
            Removal::Nothing
        }
    }
}

fn walk<'a, T>(node: &'a Node<T>, key: &mut Vec<u16>, visit: &mut impl FnMut(&[u16], &'a T)) {
    for entry in node.entries.iter().rev() {
        match entry {
            Entry::Leaf(value) => visit(key, value),
            Entry::Edge { label, child } => {
                key.extend_from_slice(label);
                walk(child, key, visit);
                key.truncate(key.len() - label.len());
            }
        }
    }
}

/// The state of MiniSearch's `fuzzySearch` walk.
struct Fuzzy<'q> {
    query: &'q [u16],
    max_distance: usize,
    columns: usize,
    rows: usize,
    matrix: Vec<u8>,
    prefix: Vec<u16>,
    results: Vec<(Vec<u16>, u8)>,
}

impl Fuzzy<'_> {
    /// `recurse`: `row` is the next matrix row to fill, one past the length of `prefix`.
    fn recurse<T>(&mut self, node: &Node<T>, row: usize) {
        let columns = self.columns;
        'entries: for entry in &node.entries {
            let (label, child) = match entry {
                Entry::Leaf(_) => {
                    // A key longer than the matrix has rows reads past it (undefined) and never matches.
                    if let Some(&distance) = self.matrix.get(row * columns - 1) {
                        if usize::from(distance) <= self.max_distance {
                            self.results.push((self.prefix.clone(), distance));
                        }
                    }
                    continue;
                }
                Entry::Edge { label, child } => (label, child),
            };
            let mut i = row;
            for &unit in label {
                if i >= self.rows {
                    // Past the last row no key can match: MiniSearch keeps walking (it reads
                    // undefined, which never prunes and never matches); stopping here finds the same keys.
                    continue 'entries;
                }
                let this_row = columns * i;
                let previous_row = this_row - columns;
                let mut min_distance = u16::from(self.matrix[this_row]);
                let first = i.saturating_sub(self.max_distance + 1);
                let last = (columns - 1).min(i + self.max_distance);
                for j in first..last {
                    let different = u16::from(unit != self.query[j]);
                    let replace = u16::from(self.matrix[previous_row + j]) + different;
                    let delete = u16::from(self.matrix[previous_row + j + 1]) + 1;
                    let insert = u16::from(self.matrix[this_row + j]) + 1;
                    let distance = replace.min(delete).min(insert);
                    self.matrix[this_row + j + 1] = distance as u8;
                    if distance < min_distance {
                        min_distance = distance;
                    }
                }
                // Distances never decrease along a row: no key below can match.
                if usize::from(min_distance) > self.max_distance {
                    continue 'entries;
                }
                i += 1;
            }
            self.prefix.extend_from_slice(label);
            self.recurse(child, i);
            self.prefix.truncate(self.prefix.len() - label.len());
        }
    }
}
