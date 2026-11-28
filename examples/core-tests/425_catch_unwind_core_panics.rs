//! Seam 4.3 regression: `js_catch_unwind` catches `#[rustc_nounwind]` core panics raised
//! through `#[panic_handler]` -> `js_panic_error` and preserves `Drop` landing pads across
//! `select_unpredictable` and `String::retain` even under `js-opt=on` (`prune_invoke`).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::cell::Cell;
use core::hint::{black_box, select_unpredictable};

#[repr(C)]
struct JsCatchResult {
    ok: bool,
    value: i32,
}

#[allow(improper_ctypes)]
unsafe extern "C-unwind" {
    fn js_log_bool(x: bool);
    fn js_log_i32(x: i32);
    fn js_panic_error(msg: &str) -> !;
    fn js_catch_unwind(f: extern "C-unwind" fn(*mut u8) -> i32, arg: *mut u8) -> JsCatchResult;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    // SAFETY: mirrors `mires-std`'s unwind-mode panic handler.
    unsafe { js_panic_error("core-panic") }
}

fn catch<F: FnOnce() -> i32>(f: F) -> Result<i32, ()> {
    struct State<F> {
        f: Option<F>,
    }
    extern "C-unwind" fn trampoline<F: FnOnce() -> i32>(raw: *mut u8) -> i32 {
        // SAFETY: `raw` points to a live `State<F>` on the caller stack.
        let state = unsafe { &mut *(raw.cast::<State<F>>()) };
        if let Some(func) = state.f.take() {
            func()
        } else {
            0
        }
    }
    let mut state = State { f: Some(f) };
    let res = unsafe {
        js_catch_unwind(
            trampoline::<F>,
            (&mut state as *mut State<F>).cast::<u8>(),
        )
    };
    if res.ok { Ok(res.value) } else { Err(()) }
}

struct Guard<'a> {
    cell: &'a Cell<u16>,
    expect: u16,
    write: u16,
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        let value = self.cell.get();
        self.cell.set(self.write);
        assert_eq!(value, self.expect);
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Core `#[rustc_nounwind]` panics: `i8::MIN.isqrt()`, `Option::unwrap`, OOB index
    let r_isqrt = catch(|| i8::MIN.isqrt() as i32);
    let r_unwrap = catch(|| Option::<i32>::None.unwrap());
    let r_oob = catch(|| {
        let a = [1_i32, 2, 3];
        a[black_box(10)]
    });
    let r_ok = catch(|| 42);

    // SAFETY: runtime logging shim.
    unsafe {
        js_log_bool(r_isqrt.is_err());
        js_log_bool(r_unwrap.is_err());
        js_log_bool(r_oob.is_err());
        js_log_i32(r_ok.unwrap_or(-1));
    }

    // 2. `select_unpredictable` drop-on-panic must run remaining `Drop` guards in reverse order
    let cell = Cell::new(0_u16);
    let r_sel = catch(|| {
        let _armed = Guard {
            cell: &cell,
            expect: 0xdead,
            write: 0,
        };
        let selected = Guard {
            cell: &cell,
            write: 0xdead,
            expect: 1,
        };
        let unselected = Guard {
            cell: &cell,
            write: 1,
            expect: 0xff,
        };
        let _chosen = select_unpredictable(true, selected, unselected);
        0
    });
    // SAFETY: runtime logging shim.
    unsafe {
        js_log_bool(r_sel.is_err());
        js_log_i32(cell.get() as i32);
    }

    // 3. `String::retain` panic safety leaves valid UTF-8
    let mut s = String::from("0è0");
    let r_ret = catch(|| {
        let mut count = 0;
        s.retain(|_| {
            count += 1;
            match count {
                1 => false,
                2 => true,
                _ => panic!("stop"),
            }
        });
        0
    });
    // SAFETY: runtime logging shim.
    unsafe {
        js_log_bool(r_ret.is_err());
        js_log_bool(core::str::from_utf8(s.as_bytes()).is_ok());
    }
}
