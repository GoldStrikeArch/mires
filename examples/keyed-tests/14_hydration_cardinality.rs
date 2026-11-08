//! Empty and one-row keyed lists hydrating as independent sole-child ranges.

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

static EMPTY: [Row; 0] = [];
static ADDED: [Row; 2] = [Row { id: 2, label: 2 }, Row { id: 3, label: 3 }];
static ONE: [Row; 1] = [Row { id: 1, label: 1 }];
static ONE_UPDATED: [Row; 1] = [Row { id: 1, label: 10 }];

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    let added = view_abi::signal(0, false);
    let updated = view_abi::signal(1, false);
    dom_view! {
        <section>
            <button class="add" @click=$(move |_event| added.set(true))>"add"</button>
            <ul class="empty">
                for row in if added.get() { &ADDED[..] } else { &EMPTY[..] } key (row.id) {
                    <li>$(row.label)</li>
                }
            </ul>
            <button class="update" @click=$(move |_event| updated.set(true))>"update"</button>
            <ul class="one">
                for row in if updated.get() { &ONE_UPDATED[..] } else { &ONE[..] } key (row.id) {
                    <li>$(row.label)</li>
                }
            </ul>
            <aside>"outside"</aside>
        </section>
    }
}
