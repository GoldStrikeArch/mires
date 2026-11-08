//! Nested keyed lists hydrating into independently owned row subtrees.

#![no_std]
#![no_main]

use view_dom_macro::dom_view;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Child {
    id: u32,
    label: u32,
}

struct Group {
    id: u32,
    children: &'static [Child],
}

static ONE_BEFORE: [Child; 2] = [Child { id: 11, label: 1 }, Child { id: 12, label: 2 }];
static ONE_AFTER: [Child; 3] = [
    Child { id: 12, label: 20 },
    Child { id: 11, label: 10 },
    Child { id: 13, label: 30 },
];
static TWO: [Child; 1] = [Child { id: 21, label: 3 }];
static BEFORE: [Group; 2] = [
    Group {
        id: 1,
        children: &ONE_BEFORE,
    },
    Group {
        id: 2,
        children: &TWO,
    },
];
static AFTER: [Group; 2] = [
    Group {
        id: 2,
        children: &TWO,
    },
    Group {
        id: 1,
        children: &ONE_AFTER,
    },
];

#[unsafe(no_mangle)]
pub fn rust_entry() -> view_abi::Node {
    let changed = view_abi::signal(0, false);
    dom_view! {
        <section>
            <button @click=$(move |_event| changed.set(true))>"update"</button>
            <main>
                for group in if changed.get() { &AFTER[..] } else { &BEFORE[..] } key (group.id) {
                    <article>
                        <h2>$(group.id)</h2>
                        <ul>
                            for child in group.children key (child.id) {
                                <li>$(child.label)</li>
                            }
                        </ul>
                    </article>
                }
            </main>
            <aside>"outside"</aside>
        </section>
    }
}
