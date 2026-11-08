//! A retained keyed row that owns an independently keyed child list.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

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
            |group| group.id,
            |group, _index| {
                view_abi::keyed_list_v1(
                    || group.children,
                    |child| child.id,
                    |child, _index| {
                        dom_view_client_only! {
                            <li>$(child.label)</li>
                        }
                    },
                )
            },
        ),
    );
    output
}
