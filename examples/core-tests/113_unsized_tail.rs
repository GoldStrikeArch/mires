//! Unsized record borrows preserve storage, shared ownership and destruction.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
use alloc::rc::Rc;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::Cell;
use mires_js::{js_bindings, js_export};

js_bindings! {
    #[js(call = "console.log")]
    fn note(value: f64);
}

struct Record<T: ?Sized> {
    header: u32,
    tail: T,
}

#[inline(never)]
fn update(record: &mut Record<[u32]>) {
    record.header += 1;
    for value in &mut record.tail { *value += record.header; }
}

struct Guard<'a>(&'a Cell<u32>, u32);
impl Drop for Guard<'_> {
    fn drop(&mut self) { self.0.set(self.0.get() * 10 + self.1); }
}

#[js_export]
pub fn rust_entry() {
    for length in 0..=17_u32 {
        let mut bytes: Rc<[u8]> = (0..length).map(|n| n as u8).collect::<Vec<_>>().into();
        assert_eq!(bytes.len(), length as usize);
        let weak = Rc::downgrade(&bytes);
        let alias = bytes.clone();
        assert!(Rc::ptr_eq(&bytes, &alias));
        assert!(core::ptr::eq(bytes.as_ptr(), alias.as_ptr()));
        assert_eq!(alias.iter().map(|n| u32::from(*n)).sum::<u32>(), length * length.saturating_sub(1) / 2);
        assert!(Rc::get_mut(&mut bytes).is_none());
        drop(alias);
        drop(weak);
        if length != 0 { Rc::get_mut(&mut bytes).unwrap()[0] = 99; }
        if length != 0 { assert_eq!(bytes[0], 99); }
        let weak = Rc::downgrade(&bytes);
        drop(bytes);
        assert!(weak.upgrade().is_none());

        let values: Rc<[Cell<u32>]> = (0..length).map(Cell::new).collect::<Vec<_>>().into();
        let alias = values.clone();
        for value in alias.iter() { value.set(value.get() + 10); }
        for (i, value) in values.iter().enumerate() { assert_eq!(value.get(), i as u32 + 10); }

        let mut words: Arc<[u32]> = (0..length).collect::<Vec<_>>().into();
        if length != 0 { Arc::get_mut(&mut words).unwrap()[0] = 77; }
        let alias = words.clone();
        assert!(Arc::ptr_eq(&words, &alias));
        assert!(core::ptr::eq(words.as_ptr(), alias.as_ptr()));
        if length != 0 { assert_eq!(alias[0], 77); }
        let weak = Arc::downgrade(&words);
        drop(words);
        assert!(weak.upgrade().is_some());
        drop(alias);
        assert!(weak.upgrade().is_none());
    }
    let mut record = Record { header: 3, tail: [10, 20, 30] };
    update(&mut record);
    assert_eq!(record.header, 4);
    assert_eq!(record.tail, [14, 24, 34]);
    let unsized_ref: &Record<[u32]> = &record;
    let raw: *const Record<[u32]> = unsized_ref;
    let direct: *const Record<[u32]> = &record as *const Record<[u32; 3]>;
    assert!(core::ptr::eq(raw, direct));

    let whole = &record.tail[..] as *const [u32];
    let short = &record.tail[..1] as *const [u32];
    assert!(!core::ptr::eq(whole, short));
    assert!(core::ptr::addr_eq(whole, short));
    assert!(core::ptr::eq(whole, &record.tail[..] as *const [u32]));

    let drops = Cell::new(0);
    let values: Rc<[Guard<'_>]> = (1..=3).map(|n| Guard(&drops, n)).collect::<Vec<_>>().into();
    let alias = values.clone();
    let weak = Rc::downgrade(&values);
    drop(values);
    assert_eq!(drops.get(), 0);
    drop(alias);
    assert_eq!(drops.get(), 123);
    assert!(weak.upgrade().is_none());
    note(1.0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
