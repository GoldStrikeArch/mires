//! Collection types: `alloc` collections plus pure-Rust `HashMap` and `HashSet`.

pub mod hash_map;
pub mod hash_set;

pub use alloc_crate::collections::{
    BTreeMap, BTreeSet, BinaryHeap, LinkedList, TryReserveError, TryReserveErrorKind, VecDeque,
    binary_heap, btree_map, btree_set, linked_list, vec_deque,
};

pub use hash_map::HashMap;
pub use hash_set::HashSet;
