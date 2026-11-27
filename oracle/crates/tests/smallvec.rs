use smallvec::{smallvec, SmallVec};

#[test]
fn inline_to_heap_spill_and_equality() {
    let mut v: SmallVec<[i32; 4]> = smallvec![10, 20, 30];
    assert!(!v.spilled());
    assert_eq!(v.len(), 3);
    assert_eq!(v.capacity(), 4);

    v.push(40);
    assert!(!v.spilled());
    v.push(50);
    assert!(v.spilled());
    assert_eq!(v.as_slice(), &[10, 20, 30, 40, 50]);

    let cloned = v.clone();
    assert_eq!(cloned, v);
    let as_vec = v.into_vec();
    assert_eq!(as_vec, vec![10, 20, 30, 40, 50]);
}

#[test]
fn insert_remove_retain_and_drain() {
    let mut v: SmallVec<[String; 2]> = SmallVec::new();
    v.push("b".into());
    v.insert(0, "a".into());
    v.push("c".into());
    v.push("d".into());
    assert!(v.spilled());
    assert_eq!(v.remove(1), "b");
    assert_eq!(v.as_slice(), &["a", "c", "d"]);

    v.retain(|s| s != "c");
    assert_eq!(v.as_slice(), &["a", "d"]);

    let mut nums: SmallVec<[i32; 4]> = smallvec![0, 1, 2, 3, 4, 5, 6];
    let drained: Vec<i32> = nums.drain(2..5).collect();
    assert_eq!(drained, vec![2, 3, 4]);
    assert_eq!(nums.as_slice(), &[0, 1, 5, 6]);
}

#[test]
fn repeat_macro_and_swap_remove() {
    let mut v: SmallVec<[u8; 8]> = smallvec![7u8; 5];
    assert!(!v.spilled());
    assert_eq!(v.as_slice(), &[7, 7, 7, 7, 7]);
    v[1] = 1;
    v[4] = 9;
    assert_eq!(v.swap_remove(1), 1);
    assert_eq!(v.as_slice(), &[7, 9, 7, 7]);
    v.truncate(2);
    assert_eq!(v.as_slice(), &[7, 9]);
}
