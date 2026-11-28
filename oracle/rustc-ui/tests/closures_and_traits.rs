//! `rustc/tests/ui`-style tests for closures, traits, `dyn Trait`, `Any`, `Error`, and GATs.

use std::any::{Any, TypeId};
use std::error::Error;
use std::fmt;

#[test]
fn ui_closure_fnonce_fnmut_fn_captures() {
    let base = 10;
    let add_base = |x: i32| x + base;
    assert_eq!(add_base(5), 15);
    assert_eq!(add_base(20), 30);

    let mut acc = 0;
    let mut bump = |delta: i32| {
        acc += delta;
        acc
    };
    assert_eq!(bump(3), 3);
    assert_eq!(bump(7), 10);
    drop(bump);
    assert_eq!(acc, 10);

    let owned = String::from("moved");
    let consume = move |suffix: &str| format!("{owned}-{suffix}");
    assert_eq!(consume("ok"), "moved-ok");
}

#[test]
fn ui_higher_order_fn_traits_and_boxed_closures() {
    fn apply_twice<F: FnMut(i32) -> i32>(mut f: F, x: i32) -> i32 {
        let y = f(x);
        f(y)
    }

    let mut mult = 2;
    let out = apply_twice(
        |v| {
            mult += 1;
            v * mult
        },
        4,
    );
    // 4 * 3 = 12; 12 * 4 = 48
    assert_eq!(out, 48);

    let ops: Vec<Box<dyn Fn(i32) -> i32>> = vec![
        Box::new(|x| x + 1),
        Box::new(|x| x * 3),
        Box::new(|x| x - 2),
    ];
    let folded = ops.iter().fold(5, |acc, f| f(acc));
    assert_eq!(folded, 16);
}

trait Shape {
    fn area(&self) -> f64;
    fn label(&self) -> &'static str;
    fn describe(&self) -> String {
        format!("{}:{:.1}", self.label(), self.area())
    }
}

struct Rect {
    w: f64,
    h: f64,
}
impl Shape for Rect {
    fn area(&self) -> f64 {
        self.w * self.h
    }
    fn label(&self) -> &'static str {
        "rect"
    }
}

struct Tri {
    b: f64,
    h: f64,
}
impl Shape for Tri {
    fn area(&self) -> f64 {
        0.5 * self.b * self.h
    }
    fn label(&self) -> &'static str {
        "tri"
    }
}

#[test]
fn ui_dyn_trait_dispatch_and_slice_of_trait_objects() {
    let r = Rect { w: 3.0, h: 4.0 };
    let t = Tri { b: 6.0, h: 5.0 };
    let shapes: [&dyn Shape; 2] = [&r, &t];
    let total: f64 = shapes.iter().map(|s| s.area()).sum();
    assert_eq!(total, 27.0);
    assert_eq!(shapes[0].describe(), "rect:12.0");
    assert_eq!(shapes[1].describe(), "tri:15.0");

    let boxed: Vec<Box<dyn Shape>> = vec![
        Box::new(Rect { w: 2.0, h: 5.0 }),
        Box::new(Tri { b: 4.0, h: 3.0 }),
    ];
    assert_eq!(boxed.iter().map(|s| s.area()).sum::<f64>(), 16.0);
}

#[test]
fn ui_any_downcast_and_type_id() {
    assert_ne!(TypeId::of::<u32>(), TypeId::of::<i32>());
    assert_eq!(TypeId::of::<String>(), TypeId::of::<std::string::String>());

    #[derive(Debug, PartialEq, Eq)]
    struct Tag {
        code: u16,
        name: String,
    }

    let mut items: Vec<Box<dyn Any>> = vec![
        Box::new(42_i32),
        Box::new(String::from("mires")),
        Box::new(vec![1_u8, 2, 3, 4]),
        Box::new(Tag {
            code: 7,
            name: String::from("ok"),
        }),
    ];

    assert_eq!(items[0].downcast_ref::<i32>(), Some(&42));
    assert_eq!(items[0].downcast_ref::<u32>(), None);
    if let Some(s) = items[1].downcast_mut::<String>() {
        s.push('!');
    }
    assert_eq!(
        items[1].downcast_ref::<String>().map(String::as_str),
        Some("mires!")
    );
    assert_eq!(items[2].downcast_ref::<Vec<u8>>(), Some(&vec![1, 2, 3, 4]));
    assert_eq!(
        items[3].downcast_ref::<Tag>(),
        Some(&Tag {
            code: 7,
            name: String::from("ok"),
        })
    );
    drop(items);
}

#[test]
fn ui_dyn_supertrait_upcast_and_rc_arc_str() {
    use std::rc::Rc;
    use std::sync::Arc;

    trait BaseA {
        fn a(&self) -> i32;
    }
    trait BaseB {
        fn b(&self) -> i32;
    }
    trait Combined: BaseA + BaseB {
        fn c(&self) -> i32;
    }
    struct Item(i32, i32, i32);
    impl BaseA for Item {
        fn a(&self) -> i32 {
            self.0
        }
    }
    impl BaseB for Item {
        fn b(&self) -> i32 {
            self.1
        }
    }
    impl Combined for Item {
        fn c(&self) -> i32 {
            self.2
        }
    }

    let item = Item(10, 20, 30);
    let sub: &dyn Combined = &item;
    let a: &dyn BaseA = sub;
    let b: &dyn BaseB = sub;
    assert_eq!((a.a(), b.b(), sub.c()), (10, 20, 30));

    let boxed: Box<dyn Combined> = Box::new(Item(1, 2, 3));
    let up_b: Box<dyn BaseB> = boxed;
    assert_eq!(up_b.b(), 2);

    let r: Rc<str> = Rc::from("hello rc");
    let r2 = Rc::clone(&r);
    assert_eq!(&*r2, "hello rc");
    let a_str: Arc<str> = Arc::from("hello arc café");
    let a2 = Arc::clone(&a_str);
    assert_eq!(&*a2, "hello arc café");
}

#[derive(Debug)]
struct InnerErr(&'static str);
impl fmt::Display for InnerErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "inner: {}", self.0)
    }
}
impl Error for InnerErr {}

#[derive(Debug)]
struct OuterErr {
    code: u16,
    source: InnerErr,
}
impl fmt::Display for OuterErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "outer({})", self.code)
    }
}
impl Error for OuterErr {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[test]
fn ui_std_error_trait_object_and_source_chain() {
    let err: Box<dyn Error> = Box::new(OuterErr {
        code: 503,
        source: InnerErr("upstream timeout"),
    });
    assert_eq!(err.to_string(), "outer(503)");
    let src = err.source().expect("source present");
    assert_eq!(src.to_string(), "inner: upstream timeout");
    assert!(src.source().is_none());
}

trait LendingView {
    type Item<'a>
    where
        Self: 'a;
    fn get_view<'a>(&'a self, idx: usize) -> Option<Self::Item<'a>>;
}

struct WindowBuf {
    data: Vec<i32>,
    width: usize,
}

impl LendingView for WindowBuf {
    type Item<'a> = &'a [i32];
    fn get_view<'a>(&'a self, idx: usize) -> Option<&'a [i32]> {
        self.data.get(idx..idx + self.width)
    }
}

#[test]
fn ui_generic_associated_types() {
    let buf = WindowBuf {
        data: vec![10, 20, 30, 40, 50],
        width: 3,
    };
    assert_eq!(buf.get_view(0), Some(&[10, 20, 30][..]));
    assert_eq!(buf.get_view(2), Some(&[30, 40, 50][..]));
    assert_eq!(buf.get_view(3), None);
}

#[test]
fn ui_fn_pointer_coercion_and_dispatch_table() {
    fn inc(x: i32) -> i32 {
        x + 1
    }
    fn dbl(x: i32) -> i32 {
        x * 2
    }
    let table: [fn(i32) -> i32; 3] = [inc, dbl, |x| x - 3];
    let mut v = 10;
    for f in table {
        v = f(v);
    }
    // ((10 + 1) * 2) - 3 = 19
    assert_eq!(v, 19);
}

trait SuperTrait {
    fn base_val(&self) -> i32;
}
trait SubTrait: SuperTrait {
    fn sub_val(&self) -> i32;
}

struct PairVal(i32, i32);
impl SuperTrait for PairVal {
    fn base_val(&self) -> i32 {
        self.0
    }
}
impl SubTrait for PairVal {
    fn sub_val(&self) -> i32 {
        self.1
    }
}

#[test]
fn ui_supertrait_method_dispatch_and_dyn_drop() {
    use std::cell::Cell;
    use std::rc::Rc;

    let p = PairVal(11, 29);
    let sub: &dyn SubTrait = &p;
    assert_eq!(sub.sub_val(), 29);
    assert_eq!(sub.base_val(), 11);

    struct Tracked {
        val: i32,
        dropped: Rc<Cell<bool>>,
    }
    impl SuperTrait for Tracked {
        fn base_val(&self) -> i32 {
            self.val
        }
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.dropped.set(true);
        }
    }

    let dropped = Rc::new(Cell::new(false));
    {
        let obj: Box<dyn SuperTrait> = Box::new(Tracked {
            val: 88,
            dropped: Rc::clone(&dropped),
        });
        assert_eq!(obj.base_val(), 88);
        assert!(!dropped.get());
    }
    assert!(dropped.get());
}
