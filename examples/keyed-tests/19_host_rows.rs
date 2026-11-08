//! A host-backed iterator materialized through the proven `Vec` source lane.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

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

struct Rows {
    at: u32,
    len: u32,
}

impl Iterator for Rows {
    type Item = Row;

    fn next(&mut self) -> Option<Row> {
        if self.at == self.len {
            return None;
        }
        let index = self.at;
        self.at += 1;
        Some(Row {
            id: keyed_rows_id(index),
            label: keyed_rows_label(index),
        })
    }
}

fn rows() -> Vec<Row> {
    Rows {
        at: 0,
        len: keyed_rows_len(),
    }
    .collect()
}

::js_extern_macro::js_bindings! {
    module = "../../scripts/keyed-host-shim.mjs";

    #[js(call = "keyed_rows_len")]
    fn keyed_rows_len() -> u32;

    #[js(call = "keyed_rows_id")]
    fn keyed_rows_id(index: u32) -> u32;

    #[js(call = "keyed_rows_label")]
    fn keyed_rows_label(index: u32) -> u32;

    #[js(call = "keyed_rows_flip")]
    fn keyed_rows_flip();
}

#[::js_extern_macro::js_export]
pub fn rust_entry() -> view_abi::Node {
    let changed = view_abi::signal(0, false);
    let output = view_abi::list();
    view_abi::push(
        &output,
        dom_view_client_only! {
            <button @click=$(move |_event| {
                keyed_rows_flip();
                changed.set(!changed.get());
            })>"update"</button>
        },
    );
    view_abi::push(
        &output,
        view_abi::keyed_list_v1(
            move || {
                let _ = changed.get();
                rows()
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
