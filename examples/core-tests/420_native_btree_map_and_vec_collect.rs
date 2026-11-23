//! Wave 4 verification: order-unobserved `BTreeMap` (`NativeMap`), fail-closed ordered/dropful
//! `BTreeMap`, optimized `mires_js::collections::{HashMap, HashSet}`, and native-friendly `Vec`
//! `collect`/`from_elem`/`to_vec`/`clone`/`extend`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::sync::atomic::{AtomicU32, Ordering};
use mires_js::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Val {
    Empty,
    Pair(i32, i32),
}

static DROP_ORDER: AtomicU32 = AtomicU32::new(0);
static DROP_COUNT: AtomicU32 = AtomicU32::new(0);

struct DropLog {
    id: u32,
}

impl Drop for DropLog {
    fn drop(&mut self) {
        let cur = DROP_ORDER.load(Ordering::Relaxed);
        DROP_ORDER.store(cur * 10 + self.id, Ordering::Relaxed);
        DROP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline(never)]
fn test_native_btree_map() {
    // 1. Direct primitive key + direct scalar value: Default, insert, overwrite, get, get_mut,
    //    contains_key, remove, len, is_empty, clear.
    let mut m: BTreeMap<u32, i32> = BTreeMap::default();
    print_bool(m.is_empty());
    print_bool(m.insert(10, 100).is_none());
    print_bool(m.insert(20, 200).is_none());
    print_i32(m.insert(10, 150).unwrap_or(-1));
    print_usize(m.len());
    print_bool(m.contains_key(&10));
    print_bool(m.contains_key(&99));
    print_i32(*m.get(&10).unwrap());
    if let Some(v) = m.get_mut(&20) {
        *v += 25;
    }
    print_i32(*m.get(&20).unwrap());
    print_i32(m.remove(&10).unwrap_or(-1));
    print_bool(m.remove(&10).is_none());
    print_usize(m.len());
    m.clear();
    print_bool(m.is_empty());

    // 2. Owned String key + indirect enum value, queried with both &str and &String.
    let mut sm: BTreeMap<String, Val> = BTreeMap::new();
    sm.insert(String::from("alpha"), Val::Pair(1, 2));
    sm.insert(String::from("beta"), Val::Empty);
    let q_str = black_box("alpha");
    let q_owned = String::from("beta");
    print_bool(sm.contains_key(q_str));
    print_bool(sm.contains_key(&q_owned));
    if let Some(Val::Pair(a, b)) = sm.get_mut(q_str) {
        *a += 10;
        *b += 20;
    }
    match sm.get("alpha") {
        Some(Val::Pair(a, b)) => print_i32(*a + *b),
        _ => print_i32(-1),
    }
    match sm.remove(&q_owned) {
        Some(Val::Empty) => print_u32(1),
        _ => print_u32(0),
    }
    print_usize(sm.len());

    // 3. Borrowed &str key, queried with &str and &&str.
    let mut bm: BTreeMap<&str, u32> = BTreeMap::new();
    bm.insert("one", 1);
    bm.insert("two", 2);
    let k_ref: &str = black_box("two");
    print_u32(*bm.get(k_ref).unwrap());
    print_u32(*bm.get(&k_ref).unwrap());

    // 4. Nested NativeVec inside NativeMap.
    let mut nm: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    nm.insert(1, vec![10, 20]);
    if let Some(v) = nm.get_mut(&1) {
        v.push(30);
    }
    let mut sum = 0u32;
    for x in nm.get(&1).unwrap() {
        sum += *x;
    }
    print_u32(sum);
    let popped = nm.remove(&1).unwrap();
    print_usize(popped.len());
}

#[inline(never)]
fn test_fail_closed_btree_map() {
    // 1. Ordered iteration must observe exact ascending Ord order.
    let mut ordered: BTreeMap<u32, u32> = BTreeMap::new();
    ordered.insert(30, 3);
    ordered.insert(10, 1);
    ordered.insert(20, 2);
    let mut acc = 0u32;
    for (&k, &v) in ordered.iter() {
        acc = acc * 100 + k + v;
    }
    print_u32(acc);

    // 2. Dropful value must run Drop in exact ascending Ord key order.
    DROP_ORDER.store(0, Ordering::Relaxed);
    {
        let mut dm: BTreeMap<u32, DropLog> = BTreeMap::new();
        dm.insert(3, DropLog { id: 3 });
        dm.insert(1, DropLog { id: 1 });
        dm.insert(2, DropLog { id: 2 });
    }
    print_u32(DROP_ORDER.load(Ordering::Relaxed));
}

#[inline(never)]
fn test_mires_js_collections() {
    let mut hm: HashMap<String, i32> = HashMap::new();
    print_bool(hm.insert(String::from("a"), 5).is_none());
    print_bool(hm.insert(String::from("b"), 7).is_none());
    print_i32(hm.insert(String::from("a"), 10).unwrap_or(-1));
    if let Some(v) = hm.get_mut("b") {
        *v *= 3;
    }
    print_i32(*hm.get("a").unwrap() + *hm.get("b").unwrap());
    let mut sum = 0i32;
    for (_k, v) in hm.iter() {
        sum += *v;
    }
    print_i32(sum);
    print_i32(hm.remove("a").unwrap_or(-1));
    print_usize(hm.len());

    let mut hs: HashSet<&str> = HashSet::new();
    print_bool(hs.insert("x"));
    print_bool(hs.insert("x"));
    print_bool(hs.insert("y"));
    print_bool(hs.contains("x"));
    print_bool(hs.remove("x"));
    print_bool(hs.contains("x"));
    let mut cnt = 0usize;
    for _k in hs.iter() {
        cnt += 1;
    }
    print_usize(cnt);
}

#[inline(never)]
fn test_native_friendly_vec_ops() {
    // collect, from_elem, to_vec, clone, extend on non-trivial String elements
    let collected: Vec<String> = (0u32..3)
        .map(|i| {
            let mut s = String::from("k");
            s.push((b'0' + i as u8) as char);
            s
        })
        .collect();
    print_usize(collected.len());
    print_str(&collected[2]);

    let mut repeated = vec![String::from("hi"); 3];
    repeated[1].push('!');
    print_str(&repeated[0]);
    print_str(&repeated[1]);

    let copied = repeated[0..2].to_vec();
    let mut cloned = collected.clone();
    cloned.extend(copied);
    print_usize(cloned.len());
    print_str(&cloned[4]);

    // Verify custom Drop count on Vec<DropLog>
    DROP_COUNT.store(0, Ordering::Relaxed);
    {
        let mut v: Vec<DropLog> = (1u32..=3).map(|id| DropLog { id }).collect();
        v.extend((4u32..=5).map(|id| DropLog { id }));
        print_usize(v.len());
    }
    print_u32(DROP_COUNT.load(Ordering::Relaxed));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    test_native_btree_map();
    test_fail_closed_btree_map();
    test_mires_js_collections();
    test_native_friendly_vec_ops();
}
