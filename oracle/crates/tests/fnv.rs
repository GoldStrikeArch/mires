use std::hash::Hasher;

use fnv::{FnvHashMap, FnvHashSet, FnvHasher};

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h = FnvHasher::default();
    h.write(bytes);
    h.finish()
}

#[test]
fn fnv1a_64_official_spec_vectors() {
    // Official FNV-1a 64-bit reference test vectors.
    assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);

    // Streaming writes match one-shot writes.
    let mut h = FnvHasher::default();
    h.write(b"foo");
    h.write(b"bar");
    assert_eq!(h.finish(), 0x8594_4171_f739_67e8);

    // Custom key changes output deterministically.
    let mut seeded = FnvHasher::with_key(12345);
    seeded.write(b"foobar");
    assert_ne!(seeded.finish(), 0x8594_4171_f739_67e8);
}

#[test]
fn fnv_hashmap_and_hashset_operations() {
    let mut map: FnvHashMap<String, i32> = FnvHashMap::default();
    for i in 0..64 {
        map.insert(format!("key_{i}"), i * i);
    }
    assert_eq!(map.len(), 64);
    assert_eq!(map.get("key_12"), Some(&144));
    *map.entry("key_12".into()).or_insert(0) += 1;
    assert_eq!(map["key_12"], 145);
    assert_eq!(map.remove("key_0"), Some(0));
    assert_eq!(map.len(), 63);

    let mut set: FnvHashSet<u32> = FnvHashSet::default();
    for x in [3, 1, 4, 1, 5, 9, 2, 6, 5] {
        set.insert(x);
    }
    let mut sorted: Vec<u32> = set.into_iter().collect();
    sorted.sort();
    assert_eq!(sorted, vec![1, 2, 3, 4, 5, 6, 9]);
}
