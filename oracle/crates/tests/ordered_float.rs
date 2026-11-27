use std::collections::{HashMap, HashSet};

use ordered_float::{NotNan, OrderedFloat};

#[test]
fn total_ordering_including_nan_and_infinities() {
    let mut vals = vec![
        OrderedFloat(3.5f64),
        OrderedFloat(f64::NAN),
        OrderedFloat(f64::INFINITY),
        OrderedFloat(-0.0),
        OrderedFloat(f64::NEG_INFINITY),
        OrderedFloat(0.0),
        OrderedFloat(-2.25),
    ];
    vals.sort();
    assert_eq!(vals[0], OrderedFloat(f64::NEG_INFINITY));
    assert_eq!(vals[1], OrderedFloat(-2.25));
    assert_eq!(vals[2], OrderedFloat(0.0));
    assert_eq!(vals[3], OrderedFloat(0.0));
    assert_eq!(vals[4], OrderedFloat(3.5));
    assert_eq!(vals[5], OrderedFloat(f64::INFINITY));
    assert!(vals[6].is_nan());
    assert_eq!(OrderedFloat(f64::NAN), OrderedFloat(f64::NAN));
}

#[test]
fn hashing_and_map_keys_consistency() {
    let mut set: HashSet<OrderedFloat<f64>> = HashSet::new();
    set.insert(OrderedFloat(-0.0));
    set.insert(OrderedFloat(0.0));
    set.insert(OrderedFloat(f64::NAN));
    set.insert(OrderedFloat(-f64::NAN));
    set.insert(OrderedFloat(1.5));
    // -0.0 == +0.0 and all NaNs are equal under OrderedFloat.
    assert_eq!(set.len(), 3);

    let mut map: HashMap<OrderedFloat<f32>, &str> = HashMap::new();
    map.insert(OrderedFloat(0.0f32), "zero");
    map.insert(OrderedFloat(f32::NAN), "nan");
    assert_eq!(map.get(&OrderedFloat(-0.0f32)), Some(&"zero"));
    assert_eq!(map.get(&OrderedFloat(f32::NAN)), Some(&"nan"));
}

#[test]
fn not_nan_validation_and_arithmetic() {
    assert!(NotNan::new(f64::NAN).is_err());
    let a = NotNan::new(1.5f64).unwrap();
    let b = NotNan::new(2.5f64).unwrap();
    assert_eq!((a + b).into_inner(), 4.0);
    assert_eq!((b - a).into_inner(), 1.0);
    assert_eq!((a * b).into_inner(), 3.75);
    assert!(a < b);
}
