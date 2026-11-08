//! Hydration of an idiomatic keyed loop scoped to one parent element.

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
    dom_view! {
        <section>
            <button @click=$(move |_event| changed.set(true))>"update"</button>
            <ul>
                for row in if changed.get() { &AFTER[..] } else { &BEFORE[..] } key (row.id) {
                    <li @click=$(|_event| row.label)>
                        <input value="seed">
                        <span>$(row.label)</span>
                    </li>
                }
            </ul>
            <aside>"outside"</aside>
        </section>
    }
}
