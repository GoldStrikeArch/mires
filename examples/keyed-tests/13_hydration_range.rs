//! Hydration of marker-delimited keyed rows whose identity spans two DOM nodes.

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

fn rows(changed: view_abi::Sig<bool>) -> view_abi::Node {
    view_abi::keyed_list_v1(
        move || {
            if changed.get() {
                &AFTER[..]
            } else {
                &BEFORE[..]
            }
        },
        |row| row.id,
        |row, _index| {
            let output = view_abi::list();
            view_abi::push(
                &output,
                dom_view! {
                    <span class="row-label" @click=$(|_event| row.label)>$(row.label)</span>
                },
            );
            view_abi::push(
                &output,
                dom_view! {
                    <input class="row-input" value="seed">
                },
            );
            output
        },
    )
}

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    let changed = view_abi::signal(0, false);
    dom_view! {
        <section>
            <button @click=$(move |_event| changed.set(true))>"update"</button>
            <div class="range">
                <b>"before"</b>
                (rows(changed))
                <i>"after"</i>
            </div>
            <aside>"outside"</aside>
        </section>
    }
}
