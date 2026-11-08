//! A borrowed `Vec` source adapted from its proven canonical layout.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[derive(Clone, Copy)]
struct Row {
    id: u32,
    label: u32,
}

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    let before: &'static Vec<Row> = Box::leak(Box::new(Vec::from([
        Row { id: 1, label: 1 },
        Row { id: 2, label: 2 },
        Row { id: 3, label: 3 },
    ])));
    let after: &'static Vec<Row> = Box::leak(Box::new(Vec::from([
        Row { id: 3, label: 30 },
        Row { id: 1, label: 10 },
        Row { id: 2, label: 20 },
    ])));
    let changed = view_abi::signal(0, false);
    let output = view_abi::list();
    view_abi::push(
        &output,
        dom_view_client_only! {
            <button @click=$(move |_event| changed.set(true))>"update"</button>
        },
    );
    view_abi::push(
        &output,
        view_abi::keyed_list_v1(
            move || if changed.get() { after } else { before },
            |row| row.id,
            |row, _index| {
                dom_view_client_only! {
                    <button @click=$(|_event| row.label)>$(row.label)</button>
                }
            },
        ),
    );
    output
}
