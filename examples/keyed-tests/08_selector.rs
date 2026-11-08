//! Selection fan-out through a compiler marker and the production reactive graph.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Row {
    id: u32,
}

struct Classes {
    selected: bool,
}

static ROWS: [Row; 3] = [Row { id: 1 }, Row { id: 2 }, Row { id: 3 }];

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    let selected = view_abi::signal(0, 1_u32);
    let is_selected = view_abi::selector(selected);
    let output = view_abi::list();
    let control = dom_view_client_only! {
        <button @click=$(move |_event| selected.set(2))>"select"</button>
    };
    view_abi::push(&output, control);
    view_abi::push(
        &output,
        view_abi::keyed_list_v1(
            || &ROWS,
            |row| row.id,
            |row, _index| {
                dom_view_client_only! {
                    <button :class=$(Classes { selected: is_selected.is(row.id) })>$(row.id)</button>
                }
            },
        ),
    );
    output
}
