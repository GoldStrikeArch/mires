#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::hint::black_box;
use core::num::NonZeroU32;
use core::pin::Pin;
use core::ptr::NonNull;

trait Speak { fn n(&self) -> u32; }
impl Speak for u32 { fn n(&self) -> u32 { *self } }

// The niche of each of these is found by walking the standard library's own layout: Box/Unique/
// NonNull, RawVec's capacity, Rc's pointer. `None` of each is a register-sized or in-memory
// constant, and `Option<T>` values are built and matched at run time.
macro_rules! probe {
    ($id:expr, $T:ty, $mk:expr, |$v:ident| $read:expr) => {{
        #[inline(never)] fn mk() -> $T { $mk }
        #[inline(never)] fn pick(some: bool) -> Option<$T> { if some { Some(mk()) } else { None } }
        #[inline(never)] fn show(o: &Option<$T>) -> u32 { match o { Some($v) => $read, None => 0 } }
        const N: Option<$T> = None;
        print_i32(-$id);
        print_u32(show(&N));
        print_u32(show(&black_box(N)));
        print_u32(show(&pick(black_box(true))));
        print_u32(show(&pick(black_box(false))));
        let mut slot: Option<$T> = None;
        print_bool(slot.is_none());
        slot = Some(mk());
        print_bool(slot.is_some());
        print_u32(show(&slot));
        let taken = slot.take();
        print_bool(slot.is_none()); print_u32(show(&taken));
        let nested: Option<Option<$T>> = Some(None);
        print_u32(match black_box(&nested) { Some(Some(_)) => 2, Some(None) => 1, None => 0 });
        let nested2: Option<Option<$T>> = black_box(Some(Some(mk())));
        print_u32(match &nested2 { Some(Some($v)) => $read, Some(None) => 1, None => 0 });
        let r: Result<$T, ()> = if black_box(true) { Ok(mk()) } else { Err(()) };
        print_u32(match &r { Ok($v) => $read, Err(()) => 0 });
        let r2: Result<$T, ()> = Err(());
        print_bool(black_box(&r2).is_err());
    }};
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    probe!(1, Box<u32>, Box::new(11), |v| **v);
    probe!(2, Box<[u32]>, Box::new([1, 2, 3]), |v| v.len() as u32 + v[2]);
    probe!(3, Box<dyn Speak>, Box::new(13u32), |v| v.n());
    probe!(5, Rc<u32>, Rc::new(15), |v| **v);
    probe!(6, Rc<[u32]>, Rc::from([1u32, 2]), |v| v.len() as u32);
    probe!(7, Arc<u32>, Arc::new(17), |v| **v);
    probe!(8, Vec<u32>, alloc::vec![1, 2, 3, 4], |v| v.len() as u32 + v[3]);
    probe!(9, String, String::from("hello"), |v| v.len() as u32);
    probe!(10, &'static mut u32, Box::leak(Box::new(20)), |v| **v);
    probe!(11, NonNull<u32>, NonNull::from(Box::leak(Box::new(21u32))), |v| unsafe { *v.as_ptr() });
    probe!(12, NonNull<[u32]>, NonNull::from(Box::leak(Box::new([1u32, 2, 3]) as Box<[u32]>)), |v| v.len() as u32);
    probe!(13, Pin<Box<u32>>, Box::pin(23), |v| **v);
    probe!(14, BTreeMap<u32, u32>, { let mut m = BTreeMap::new(); m.insert(1, 24); m }, |v| v[&1]);
    probe!(15, VecDeque<u32>, { let mut d = VecDeque::new(); d.push_back(25); d }, |v| v[0]);
    probe!(16, Cow<'static, str>, Cow::Borrowed("cow"), |v| v.len() as u32);
    probe!(17, Cow<'static, str>, Cow::Owned(String::from("owned")), |v| v.len() as u32);
    probe!(19, RefCell<Option<Box<u32>>>, RefCell::new(Some(Box::new(29))), |v| v.borrow().as_deref().copied().unwrap_or(0));
    probe!(20, (Box<u32>, NonZeroU32), (Box::new(30), NonZeroU32::new(1).unwrap()), |v| *v.0 + v.1.get());
    probe!(21, Vec<Option<Box<u32>>>, alloc::vec![None, Some(Box::new(31))], |v| v.iter().flatten().map(|b| **b).sum::<u32>());
    probe!(22, fn(u32) -> u32, |x| x + 32, |v| v(0));
    probe!(23, Box<dyn Fn(u32) -> u32>, Box::new(|x| x + 33), |v| v(0));
    probe!(24, alloc::vec::IntoIter<u32>, alloc::vec![34].into_iter(), |v| v.as_slice()[0]);
    probe!(25, core::slice::Iter<'static, u32>, [35u32, 1].iter(), |v| v.as_slice()[0]);
    probe!(26, core::str::Chars<'static>, "$".chars(), |v| v.as_str().len() as u32 + 35);
    probe!(27, Option<Rc<Cell<u32>>>, Some(Rc::new(Cell::new(37))), |v| v.as_ref().map_or(0, |c| c.get()));
}
