//! Family 04 -- event handlers.
//!
//! `@name` picks the sink by consulting the same 22-name delegated set the
//! reference plugin uses: `click` is delegated, `change` is not. Topcoat has no
//! `on:` namespace, so a non-delegated name is the only way to reach
//! addEventListener.

use view_dom_macro::dom_view;

fn delegated(count: u32) -> view_abi::Node {
    dom_view! {
        <button @click=$(|_e| count + 1)>"Click"</button>
    }
}

fn delegated_bound(handler: impl Fn(u32)) -> view_abi::Node {
    dom_view! {
        <button @click=(handler)>"Click"</button>
    }
}

fn non_delegated(value: u32) -> view_abi::Node {
    dom_view! {
        <button @change=$(|_e| value)>"Change"</button>
    }
}

fn custom_event(value: u32) -> view_abi::Node {
    dom_view! {
        <button @custom-event=$(|_e| value)>"Custom"</button>
    }
}

fn many(count: u32, query: &str) -> view_abi::Node {
    dom_view! {
        <div id="main">
            <button @click=$(|_e| count + 1)>"+1"</button>
            <input @input=$(|e| e.target.value) value=(query)>
        </div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = delegated;
    let _ = delegated_bound;
    let _ = non_delegated;
    let _ = custom_event;
    let _ = many;
}
