//! mires bug repro (MIRES-022): `Vec<String>::sort_unstable()` on 21 or more strings throws
//! "`copy_nonoverlapping` over 1 elements at N reaches past the end of a buffer of 1" in core's
//! small sort (`sort4_stable`). Up to 20 elements, core sorts by insertion and the call succeeds.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use mires_js::{js_bindings, js_export, throw_value, JsValue};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("panic"))
}

/// The smallest length that leaves `sort_unstable`'s insertion sort for its small sort.
const WORDS: usize = 21;

/// `n` two-letter words in an unsorted order, sorted with `sort_unstable`.
#[inline(never)]
fn sorted_words(n: usize) -> Vec<String> {
    let mut words: Vec<String> = (0..n)
        .map(|i| {
            let mut word = String::new();
            word.push(char::from(b'a' + ((i * 5) % 26) as u8));
            word.push(char::from(b'a' + ((i * 3) % 26) as u8));
            word
        })
        .collect();
    words.sort_unstable();
    words
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    if sorted_words(WORDS).windows(2).any(|pair| pair[0] > pair[1]) {
        return JsValue::erase("MISCOMPILED: sort_unstable left the words out of order");
    }
    JsValue::erase(text)
}
