//! Retained rows whose text, class, attributes, properties, and handlers all read the live item.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Row {
    id: u32,
    label: u32,
    active: bool,
}

struct Classes {
    on: bool,
    off: bool,
}

static BEFORE: [Row; 3] = [
    Row {
        id: 1,
        label: 1,
        active: false,
    },
    Row {
        id: 2,
        label: 2,
        active: true,
    },
    Row {
        id: 3,
        label: 3,
        active: false,
    },
];
static AFTER: [Row; 3] = [
    Row {
        id: 3,
        label: 30,
        active: true,
    },
    Row {
        id: 1,
        label: 10,
        active: true,
    },
    Row {
        id: 2,
        label: 20,
        active: false,
    },
];

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    let changed = view_abi::signal(0, false);
    let output = view_abi::list();
    let control = dom_view_client_only! {
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
                dom_view_client_only! {
                    <button
                        :class=$(Classes { on: row.active, off: !row.active })
                        :title=$(row.label)
                        :disabled=$(row.active)
                        :value=$(row.label)
                        @click=$(|_event| row.label)
                    >$(row.label)</button>
                }
            },
        ),
    );
    output
}
