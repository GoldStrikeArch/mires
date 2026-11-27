//! Transparent map-entry carriers used by `#[js_export]` wrappers for `HashMap` and `BTreeMap`
//! parameters.
//!
//! At run time both wrappers are `#[repr(transparent)]` over [`alloc::vec::Vec`]`<(K, V)>`, while
//! keeping distinct nominal types so the compiler's boundary marshaler and `.d.ts` generator can
//! treat them as JavaScript objects / `Map`s rather than plain tuple arrays.

use alloc::vec::Vec;

/// Transparent entry-list carrier for `HashMap<K, V>` parameters on `#[js_export]` functions.
#[repr(transparent)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsMapEntries<K, V>(pub Vec<(K, V)>);

impl<K, V> JsMapEntries<K, V> {
    /// Consumes this carrier and returns the underlying `(K, V)` entry vector.
    #[must_use]
    pub fn into_vec(self) -> Vec<(K, V)> {
        self.0
    }
}

impl<K, V> Default for JsMapEntries<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K, V> From<Vec<(K, V)>> for JsMapEntries<K, V> {
    fn from(entries: Vec<(K, V)>) -> Self {
        Self(entries)
    }
}

impl<K, V> From<JsMapEntries<K, V>> for Vec<(K, V)> {
    fn from(entries: JsMapEntries<K, V>) -> Self {
        entries.0
    }
}

/// Transparent entry-list carrier for `BTreeMap<K, V>` parameters on `#[js_export]` functions.
#[repr(transparent)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsBTreeMapEntries<K, V>(pub Vec<(K, V)>);

impl<K, V> JsBTreeMapEntries<K, V> {
    /// Consumes this carrier and returns the underlying `(K, V)` entry vector.
    #[must_use]
    pub fn into_vec(self) -> Vec<(K, V)> {
        self.0
    }
}

impl<K, V> Default for JsBTreeMapEntries<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K, V> From<Vec<(K, V)>> for JsBTreeMapEntries<K, V> {
    fn from(entries: Vec<(K, V)>) -> Self {
        Self(entries)
    }
}

impl<K, V> From<JsBTreeMapEntries<K, V>> for Vec<(K, V)> {
    fn from(entries: JsBTreeMapEntries<K, V>) -> Self {
        entries.0
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;
    use alloc::string::String;
    use alloc::vec;

    use super::*;

    #[test]
    fn map_entries_have_transparent_vec_layout() {
        assert_eq!(
            core::mem::size_of::<JsMapEntries<String, i32>>(),
            core::mem::size_of::<Vec<(String, i32)>>()
        );
        assert_eq!(
            core::mem::align_of::<JsMapEntries<String, i32>>(),
            core::mem::align_of::<Vec<(String, i32)>>()
        );
        assert_eq!(
            core::mem::size_of::<JsBTreeMapEntries<String, i32>>(),
            core::mem::size_of::<Vec<(String, i32)>>()
        );
        assert_eq!(
            core::mem::align_of::<JsBTreeMapEntries<String, i32>>(),
            core::mem::align_of::<Vec<(String, i32)>>()
        );
    }

    #[test]
    fn map_entries_round_trip_and_collect_into_btree_map() {
        let entries = JsMapEntries::from(vec![(String::from("a"), 1), (String::from("b"), 2)]);
        assert_eq!(entries.0.len(), 2);
        let raw_vec: Vec<_> = entries.into();
        assert_eq!(
            raw_vec,
            vec![(String::from("a"), 1), (String::from("b"), 2)]
        );

        let btree_entries =
            JsBTreeMapEntries::from(vec![(String::from("z"), 9), (String::from("a"), 3)]);
        let map: BTreeMap<String, i32> = btree_entries.into_vec().into_iter().collect();
        assert_eq!(map.get("a"), Some(&3));
        assert_eq!(map.get("z"), Some(&9));
        assert_eq!(JsMapEntries::<u32, u32>::default().into_vec().len(), 0);
        assert_eq!(JsBTreeMapEntries::<u32, u32>::default().into_vec().len(), 0);
    }
}
