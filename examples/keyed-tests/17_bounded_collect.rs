//! A bounded iterator pipeline materialized into the keyed `Vec` source lane.

#![no_std]
#![no_main]

extern crate alloc;

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
            move || {
                let rows = if changed.get() {
                    [
                        Row { id: 3, label: 30 },
                        Row { id: 1, label: 10 },
                        Row { id: 2, label: 20 },
                    ]
                } else {
                    [
                        Row { id: 1, label: 1 },
                        Row { id: 2, label: 2 },
                        Row { id: 3, label: 3 },
                    ]
                };
                rows.into_iter()
                    .enumerate()
                    .map(|(index, row)| Row {
                        id: row.id,
                        label: row.label + index as u32,
                    })
                    .take(3)
                    .collect::<Vec<_>>()
            },
            |row| row.id,
            |row, _index| {
                dom_view_client_only! {
                    <button title=(row.label)>(row.id)</button>
                }
            },
        ),
    );
    output
}
