//! `Option<Box<str>>` / `Option<NonNull<str>>` niches, collections (`Vec`, `BTreeMap`),
//! `Rc<Box<str>>` / `Arc<Box<str>>`, constants/`static`s, `MaybeUninit`/`ManuallyDrop`, and
//! `Location::caller().file()` (MIRES-011).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem::{self, ManuallyDrop, MaybeUninit};
use core::panic::Location;
use core::ptr::NonNull;

const OPT_NONE: Option<Box<str>> = None;
const OPT_NN_NONE: Option<NonNull<str>> = None;
const CONST_NN_STR: NonNull<str> = NonNull::from_ref("const-é");
const CONST_NN_EMPTY: NonNull<str> = NonNull::from_ref("");
const OPT_NN_SOME: Option<NonNull<str>> = Some(NonNull::from_ref("opt-const"));
static STATIC_OPT_NONE: Option<Box<str>> = None;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Niche layout checks + Option<Box<str>> / Option<NonNull<str>> / Result<Box<str>, u32>
    print_bool(mem::size_of::<Option<Box<str>>>() == mem::size_of::<Box<str>>());
    print_bool(mem::size_of::<Option<NonNull<str>>>() == mem::size_of::<NonNull<str>>());

    {
        let mut opt_some: Option<Box<str>> = black_box(Some(Box::from("OpT-é")));
        if let Some(s) = opt_some.as_deref_mut() {
            s.make_ascii_lowercase();
        }
        print_str(opt_some.as_deref().unwrap());
        let taken = opt_some.take();
        print_bool(opt_some.is_none());
        print_str(taken.as_deref().unwrap());
        drop(taken);

        let opt_empty: Option<Box<str>> = black_box(Some(Box::<str>::default()));
        print_bool(opt_empty.is_some());
        print_usize(opt_empty.as_deref().unwrap().len());
        drop(opt_empty);

        let opt_none: Option<Box<str>> = black_box(OPT_NONE);
        print_bool(opt_none.is_none());
        print_bool(STATIC_OPT_NONE.is_none());
    }

    {
        let mut backing = String::from(black_box("nN-oPt"));
        let opt_nn: Option<NonNull<str>> = black_box(Some(NonNull::from(backing.as_mut_str())));
        if let Some(mut nn) = opt_nn {
            unsafe {
                nn.as_mut().make_ascii_uppercase();
                print_str(nn.as_ref());
            }
        }
        print_str(backing.as_str());

        let opt_nn_none: Option<NonNull<str>> = black_box(OPT_NN_NONE);
        print_bool(opt_nn_none.is_none());

        let c_nn = black_box(CONST_NN_STR);
        let c_empty = black_box(CONST_NN_EMPTY);
        let c_opt = black_box(OPT_NN_SOME);
        unsafe {
            print_str(c_nn.as_ref());
            print_usize(c_empty.as_ref().len());
            print_str(c_opt.unwrap().as_ref());
        }
    }

    {
        let ok: Result<Box<str>, u32> = black_box(Ok(Box::from("res-ok")));
        let err: Result<Box<str>, u32> = black_box(Err(7));
        print_str(ok.as_deref().unwrap());
        print_u32(err.unwrap_err());
        drop(ok);
    }

    // 2. Vec<Box<str>>: push, mutate elements, sort, pop, clear, drop
    {
        let mut v: Vec<Box<str>> = Vec::new();
        v.push(Box::from(black_box("Charlie")));
        let mut s = String::with_capacity(32);
        s.push_str(black_box("Alpha-é"));
        v.push(s.into_boxed_str());
        v.push(Box::<str>::default());
        v.push(Box::from(black_box("Bravo")));
        for item in v.iter_mut() {
            item.make_ascii_lowercase();
        }
        v.sort();
        for item in &v {
            print_str(item);
        }
        let popped = v.pop().unwrap();
        print_str(&popped);
        drop(popped);
        v.clear();
        print_usize(v.len());
    }

    // 3. BTreeMap<Box<str>, u32>
    {
        let mut map: BTreeMap<Box<str>, u32> = BTreeMap::new();
        map.insert(Box::from(black_box("two")), 2);
        map.insert(Box::from(black_box("one-é")), 1);
        map.insert(Box::from(black_box("three")), 3);
        print_u32(*map.get("one-é").unwrap());
        print_u32(map.insert(Box::from(black_box("two")), 22).unwrap());
        print_u32(map.remove("three").unwrap());
        for (k, val) in &map {
            print_str(k);
            print_u32(*val);
        }
    }

    // 4. Rc<Box<str>> and Arc<Box<str>> (plus Rc<[u8]> / Arc<[u8]> remain unaffected;
    //    bare Rc<str>/Arc<str> stay guarded by `guard_rc_str.rs`)
    {
        let rc1: Rc<Box<str>> = Rc::new(Box::from(black_box("rc-hello-é")));
        let rc2 = Rc::clone(&rc1);
        print_usize(Rc::strong_count(&rc1));
        print_str(&rc2);
        drop(rc2);
        print_usize(Rc::strong_count(&rc1));
        drop(rc1);

        let arc1: Arc<Box<str>> =
            Arc::new(String::from(black_box("arc-world-🚀")).into_boxed_str());
        let arc2 = Arc::clone(&arc1);
        print_usize(Arc::strong_count(&arc1));
        print_str(&arc2);
        drop(arc2);
        drop(arc1);
    }

    // 5. MaybeUninit<Box<str>> and ManuallyDrop<Box<str>>
    {
        let mut mu: MaybeUninit<Box<str>> = MaybeUninit::new(Box::from(black_box("MuBoX")));
        unsafe { mu.assume_init_mut().make_ascii_lowercase() };
        let b = unsafe { mu.assume_init() };
        print_str(&b);
        let md = ManuallyDrop::new(b);
        print_str(&md);
        let unwrapped = ManuallyDrop::into_inner(md);
        drop(unwrapped);
    }

    // 6. Location::caller().file() (Location stores `filename: NonNull<str>` in real `core`)
    {
        #[inline(never)]
        #[track_caller]
        fn get_caller_loc() -> &'static Location<'static> {
            Location::caller()
        }
        let loc = get_caller_loc();
        print_bool(loc.file().contains("379_box_str_collections_niches_consts"));
        print_bool(loc.line() > 0 && Location::caller().file().ends_with(".rs"));
    }
}
