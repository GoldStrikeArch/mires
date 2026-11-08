//! A retained-row update whose templates claim server-rendered nodes during hydration.

#![no_std]
#![no_main]

use view_dom_macro::dom_view;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Row {
    id: u32,
    label: u32,
}

static BEFORE: [Row; 3] = [
    Row { id: 1, label: 1 },
    Row { id: 2, label: 2 },
    Row { id: 3, label: 3 },
];
static AFTER: [Row; 3] = [
    Row { id: 3, label: 30 },
    Row { id: 1, label: 10 },
    Row { id: 2, label: 20 },
];

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    let changed = view_abi::signal(0, false);
    let output = view_abi::list();
    let control = dom_view! {
        <button @click=$(move |_event| changed.set(true))>"update"</button>
    };
    view_abi::push(&output, control);
    view_abi::push(
        &output,
        view_abi::keyed_list_v1(
            || {
                if changed.get() {
                    &AFTER[..]
                } else {
                    &BEFORE[..]
                }
            },
            |row| row.id,
            |row, _index| {
                dom_view! {
                    <button @click=$(|_event| row.label)>$(row.label)</button>
                }
            },
        ),
    );
    output
}
