//! A `Vec` and product item defined and constructed in another crate.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
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
            move || keyed_source::rows(changed.get()),
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
