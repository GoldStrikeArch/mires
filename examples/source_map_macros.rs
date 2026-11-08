//! Macro debug locations follow rustc's collapse policy and stop at function bodies.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::js_export;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

#[collapse_debuginfo(yes)]
macro_rules! collapsed {
    ($value:expr) => {
        if $value == 1 {
            1
        } else {
            $value + 11
        }
    };
}

#[collapse_debuginfo(no)]
macro_rules! expanded {
    ($value:expr) => {
        if $value == 2 {
            // expanded condition
            2
        } else {
            $value + 22
        }
    };
}

#[collapse_debuginfo(yes)]
macro_rules! generated {
    () => {
        #[js_export]
        pub fn generated_function(value: i32) -> i32 {
            if value == 3 {
                // generated body condition
                3
            } else {
                value + 33
            }
        }
    };
}

generated!();

#[js_export]
pub fn collapsed_function(value: i32) -> i32 {
    collapsed!(value) // collapsed invocation
}

#[js_export]
pub fn expanded_function(value: i32) -> i32 {
    expanded!(value)
}

#[js_export]
pub fn panic_function(value: bool) -> i32 {
    if value {
        panic!("macro source location"); // panic invocation
    }
    99
}
