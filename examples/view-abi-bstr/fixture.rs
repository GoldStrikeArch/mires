//! End-to-end `view-abi` + `keyed_list_v1` fixture testing `js-str=bytes` without `view-dom-macro`.
//!
//! Constructs `static view_abi::TemplateData` values directly in Rust and exercises every
//! `view-abi` marker and `keyed_list_v1` lane across ASCII, multi-byte BMP/SMP Unicode
//! (`"é😀"`, `"€"`, `"π"`), and host lone surrogates (`event.target.value` and `on_settled`).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;
use js_extern_macro::js_extern;
use view_abi::{Hole, HoleKind, JsValue, Node, Sig, TemplateData, ABI_VERSION, NO_EFFECT_GROUP};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// Pass `&'static TemplateData` through a `const` item so unoptimized MIR under `JSC_PANIC=abort`
// (`js-cleanup=on`) preserves `Operand::Constant` provenance to the `static TemplateData`.
macro_rules! tmpl {
    ($data:ident) => {{
        const DATA_REF: &'static TemplateData = &$data;
        view_abi::template(DATA_REF)
    }};
}

#[js_extern]
unsafe extern "C" {
    #[js(call = "noteRust")]
    fn note_rust(tag: &str, byte_len: u32, char_len: u32, text: &str);

    #[js(call = "resolvedEmoji")]
    fn resolved_emoji() -> JsValue;

    #[js(call = "resolvedSurrogate")]
    fn resolved_surrogate() -> JsValue;

    #[js(call = "noteCleanup")]
    fn note_cleanup(tag: &str);
}

#[repr(C)]
struct StyleObj {
    content: &'static str,
}

#[repr(C)]
struct ClassObj {
    active: bool,
}

#[repr(C)]
struct SpreadAttrs {
    id: &'static str,
    title: &'static str,
}

struct DisplayKey {
    tag: &'static str,
    n: u32,
}

impl fmt::Display for DisplayKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.tag, self.n)
    }
}

static TMPL_MAIN: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<div data-static=\"é😀\"><span></span><p><!></p><input></input><section><!></section><a></a><b></b><aside></aside></div>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[
        // 0: static non-ASCII attribute on <span>
        Hole {
            path: "c",
            kind: HoleKind::Attribute,
            name: "data-info",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        // 1: reactive attribute in shared effect group 1 on <span>
        Hole {
            path: "c",
            kind: HoleKind::Attribute,
            name: "data-dyn",
            reactive: true,
            effect_group: 1,
        },
        // 2: reactive property in shared effect group 1 on <span>
        Hole {
            path: "c",
            kind: HoleKind::Property,
            name: "title",
            reactive: true,
            effect_group: 1,
        },
        // 3: static Style object with non-ASCII &str on <span>
        Hole {
            path: "c",
            kind: HoleKind::Style,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        // 4: reactive ClassList on <span>
        Hole {
            path: "c",
            kind: HoleKind::ClassList,
            name: "",
            reactive: true,
            effect_group: NO_EFFECT_GROUP,
        },
        // 5: reactive &str child inside <p><!></p> (direct &str effect return)
        Hole {
            path: "cnc",
            kind: HoleKind::Child,
            name: "",
            reactive: true,
            effect_group: NO_EFFECT_GROUP,
        },
        // 6: delegated input event on <input>
        Hole {
            path: "cnn",
            kind: HoleKind::DelegatedEvent,
            name: "input",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        // 7: direct event listener on <input>
        Hole {
            path: "cnn",
            kind: HoleKind::Event,
            name: "custom",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        // 8: component inside <section><!></section>
        Hole {
            path: "cnnnc",
            kind: HoleKind::Component,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        // 9: static non-ASCII &str child in <a></a>
        Hole {
            path: "cnnnn",
            kind: HoleKind::Child,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        // 10: static ASCII &str child in <b></b> (KnownAscii elision)
        Hole {
            path: "cnnnnn",
            kind: HoleKind::Child,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        // 11: Spread attrs with non-ASCII &str on <aside></aside>
        Hole {
            path: "cnnnnnn",
            kind: HoleKind::Spread,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
    ],
    events: &["input"],
};

static TMPL_SUB: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<article><span></span><span></span><span></span><span></span><span></span></article>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[
        // 0: reactive text(Sig<&str>)
        Hole {
            path: "c",
            kind: HoleKind::Child,
            name: "",
            reactive: true,
            effect_group: NO_EFFECT_GROUP,
        },
        // 1: reactive text(Sig<u32>)
        Hole {
            path: "cn",
            kind: HoleKind::Child,
            name: "",
            reactive: true,
            effect_group: NO_EFFECT_GROUP,
        },
        // 2: static text(DisplayKey) -> display_to_str
        Hole {
            path: "cnn",
            kind: HoleKind::Child,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        // 3: static text(char)
        Hole {
            path: "cnnn",
            kind: HoleKind::Child,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        // 4: static text(&&str)
        Hole {
            path: "cnnnn",
            kind: HoleKind::Child,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
    ],
    events: &[],
};

fn sub_component(msg: Sig<&'static str>, count: Sig<u32>) -> Node {
    let root = tmpl!(TMPL_SUB);
    view_abi::effect(&root, 0, move || view_abi::text(msg.get()));
    view_abi::effect(&root, 1, move || view_abi::text(count.get()));
    view_abi::hole(&root, 2, view_abi::text(DisplayKey { tag: "€", n: 7 }));
    view_abi::hole(&root, 3, view_abi::text('😀'));
    let double_ref: &&str = &"é😀";
    view_abi::hole(&root, 4, view_abi::text(double_ref));
    root
}

#[unsafe(no_mangle)]
pub fn build_main() -> Node {
    let msg = view_abi::signal(0, "init-é😀");
    let count = view_abi::signal(1, 10_u32);
    let active = view_abi::signal(2, true);

    let root = tmpl!(TMPL_MAIN);
    view_abi::hole(&root, 0, "info-é😀-€");
    view_abi::effect(&root, 1, move || msg.get());
    view_abi::effect(&root, 2, move || msg.get());
    view_abi::hole(&root, 3, StyleObj { content: "\"é😀\"" });
    view_abi::effect(&root, 4, move || ClassObj {
        active: active.get(),
    });
    view_abi::effect(&root, 5, move || msg.get());
    view_abi::handler(&root, 6, move |ev| -> &'static str {
        let val = ev.target_value();
        note_rust("etv", val.len() as u32, val.chars().count() as u32, val);
        msg.set(val);
        active.set(val.contains("€"));
        "discarded-non-unit-é😀"
    });
    view_abi::handler(&root, 7, move |ev| {
        ev.prevent_default();
        let val = ev.target_value();
        count.set(val.len() as u32);
    });
    view_abi::component(&root, 8, move || sub_component(msg, count));
    view_abi::hole(&root, 9, "é😀");
    view_abi::hole(&root, 10, "ascii-only");
    view_abi::hole(
        &root,
        11,
        SpreadAttrs {
            id: "spread-id",
            title: "spread-é😀",
        },
    );
    root
}

static TMPL_COND: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<div><!></div>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[Hole {
        path: "c",
        kind: HoleKind::Child,
        name: "",
        reactive: false,
        effect_group: NO_EFFECT_GROUP,
    }],
    events: &[],
};

static TMPL_COND_BTN: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<button>toggle</button>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[Hole {
        path: "",
        kind: HoleKind::DelegatedEvent,
        name: "click",
        reactive: false,
        effect_group: NO_EFFECT_GROUP,
    }],
    events: &["click"],
};

static TMPL_LI: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<li></li>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[Hole {
        path: "",
        kind: HoleKind::Child,
        name: "",
        reactive: false,
        effect_group: NO_EFFECT_GROUP,
    }],
    events: &[],
};

fn make_li(label: &'static str) -> Node {
    let li = tmpl!(TMPL_LI);
    view_abi::hole(&li, 0, label);
    li
}

#[unsafe(no_mangle)]
pub fn build_cond() -> Node {
    let show = view_abi::signal(0, true);
    let out = view_abi::list();

    let btn = tmpl!(TMPL_COND_BTN);
    view_abi::handler(&btn, 0, move |_ev| {
        show.set(!show.get());
    });
    view_abi::push(&out, btn);

    let box1 = tmpl!(TMPL_COND);
    view_abi::hole(
        &box1,
        0,
        view_abi::cond(
            move || show.get(),
            || view_abi::content("cond-é😀"),
            || view_abi::content(()),
        ),
    );
    view_abi::push(&out, box1);

    let box2 = tmpl!(TMPL_COND);
    view_abi::hole(
        &box2,
        0,
        view_abi::cond(
            move || !show.get(),
            || view_abi::content(view_abi::text("€")),
            || view_abi::content(make_li("fallback-π")),
        ),
    );
    view_abi::push(&out, box2);

    out
}

static TMPL_UL: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<ul></ul>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[Hole {
        path: "",
        kind: HoleKind::Child,
        name: "",
        reactive: false,
        effect_group: NO_EFFECT_GROUP,
    }],
    events: &[],
};

#[unsafe(no_mangle)]
pub fn render_push_keyed(reverse: u32) -> Node {
    let str_keys_fwd: [&'static str; 3] = ["é😀", "€", "plain"];
    let str_keys_rev: [&'static str; 3] = ["plain", "€", "é😀"];
    let str_keys = if reverse == 0 {
        &str_keys_fwd[..]
    } else {
        &str_keys_rev[..]
    };

    let str_ul = tmpl!(TMPL_UL);
    let str_list = view_abi::list();
    for &k in str_keys {
        view_abi::push_keyed(&str_list, k, make_li(k));
    }
    view_abi::hole(&str_ul, 0, str_list);

    let disp_fwd: [DisplayKey; 2] = [
        DisplayKey { tag: "é😀", n: 1 },
        DisplayKey { tag: "€", n: 2 },
    ];
    let disp_rev: [DisplayKey; 2] = [
        DisplayKey { tag: "€", n: 2 },
        DisplayKey { tag: "é😀", n: 1 },
    ];
    let disp_keys = if reverse == 0 {
        &disp_fwd[..]
    } else {
        &disp_rev[..]
    };

    let disp_ul = tmpl!(TMPL_UL);
    let disp_list = view_abi::list();
    for k in disp_keys {
        view_abi::push_keyed(&disp_list, k, make_li(k.tag));
    }
    view_abi::hole(&disp_ul, 0, disp_list);

    let pair = view_abi::list();
    view_abi::push(&pair, str_ul);
    view_abi::push(&pair, disp_ul);
    pair
}

static TMPL_SEL_ITEM: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<span></span>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[
        Hole {
            path: "",
            kind: HoleKind::Attribute,
            name: "data-key",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        Hole {
            path: "",
            kind: HoleKind::Attribute,
            name: "data-selected",
            reactive: true,
            effect_group: NO_EFFECT_GROUP,
        },
        Hole {
            path: "",
            kind: HoleKind::Child,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
    ],
    events: &[],
};

static TMPL_SEL_BTN: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<button>select</button>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[Hole {
        path: "",
        kind: HoleKind::DelegatedEvent,
        name: "click",
        reactive: false,
        effect_group: NO_EFFECT_GROUP,
    }],
    events: &["click"],
};

#[inline(never)]
fn selector_row(key: &'static str, is_selected: view_abi::Selector<&'static str>) -> Node {
    let item = tmpl!(TMPL_SEL_ITEM);
    view_abi::hole(&item, 0, key);
    view_abi::effect(&item, 1, move || {
        let hit = is_selected.is(key);
        note_rust("sel-check", key.len() as u32, u32::from(hit), key);
        if hit {
            "yes"
        } else {
            "no"
        }
    });
    view_abi::hole(&item, 2, key);
    item
}

#[unsafe(no_mangle)]
pub fn build_selector() -> Node {
    let selected = view_abi::signal(0, "é😀");
    let is_selected = view_abi::selector(selected);
    let out = view_abi::list();

    let btn = tmpl!(TMPL_SEL_BTN);
    view_abi::handler(&btn, 0, move |ev| {
        selected.set(ev.target_value());
    });
    view_abi::push(&out, btn);

    view_abi::push(&out, selector_row("é😀", is_selected));
    view_abi::push(&out, selector_row("€", is_selected));
    view_abi::push(&out, selector_row("a", is_selected));

    out
}

static TMPL_SETTLED: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<p></p>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[Hole {
        path: "",
        kind: HoleKind::Child,
        name: "",
        reactive: true,
        effect_group: NO_EFFECT_GROUP,
    }],
    events: &[],
};

#[unsafe(no_mangle)]
pub fn build_settled() -> Node {
    let text_sig = view_abi::signal(0, "pending");
    let own = view_abi::owner();
    view_abi::on_cleanup(|| {
        note_cleanup("settled-owner");
    });

    view_abi::microtask(move || {
        note_cleanup("microtask-ran");
        view_abi::with_owner(own, move || {
            view_abi::on_settled::<&'static str, _>(resolved_emoji(), move |val, ok| {
                if ok && val == "é😀" && val.len() == 6 && val.chars().count() == 2 {
                    note_rust(
                        "settled-emoji",
                        val.len() as u32,
                        val.chars().count() as u32,
                        val,
                    );
                    text_sig.set(val);
                }
            });
            view_abi::on_settled::<&'static str, _>(resolved_surrogate(), move |val, ok| {
                let bytes = val.as_bytes();
                if ok
                    && bytes == &[b'x', 0xEF, 0xBF, 0xBD, b'y']
                    && val.len() == 5
                    && val.chars().count() == 3
                {
                    note_rust(
                        "settled-surrogate",
                        val.len() as u32,
                        val.chars().count() as u32,
                        val,
                    );
                }
            });
        });
    });

    let p = tmpl!(TMPL_SETTLED);
    view_abi::effect(&p, 0, move || text_sig.get());
    p
}

static TMPL_KEYED_LIVE_ROW: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<li><span></span><b></b></li>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[
        Hole {
            path: "",
            kind: HoleKind::Attribute,
            name: "data-key",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        Hole {
            path: "c",
            kind: HoleKind::Child,
            name: "",
            reactive: true,
            effect_group: NO_EFFECT_GROUP,
        },
        Hole {
            path: "cn",
            kind: HoleKind::Child,
            name: "",
            reactive: true,
            effect_group: NO_EFFECT_GROUP,
        },
        Hole {
            path: "",
            kind: HoleKind::DelegatedEvent,
            name: "click",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
    ],
    events: &["click"],
};

static TMPL_KEYED_STATIC_ROW: TemplateData = TemplateData {
    abi: ABI_VERSION,
    html: "<li><span></span><b></b></li>",
    hydratable: false,
    is_import_node: false,
    is_svg: false,
    holes: &[
        Hole {
            path: "",
            kind: HoleKind::Attribute,
            name: "data-key",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        Hole {
            path: "c",
            kind: HoleKind::Child,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
        Hole {
            path: "cn",
            kind: HoleKind::Child,
            name: "",
            reactive: false,
            effect_group: NO_EFFECT_GROUP,
        },
    ],
    events: &[],
};

static SLICE_BEFORE: [(&str, u32); 3] = [("é😀", 1), ("€", 2), ("ascii", 3)];
static SLICE_AFTER: [(&str, u32); 3] = [("€", 20), ("π", 4), ("é😀", 10)];

#[unsafe(no_mangle)]
pub fn build_keyed_slice() -> Node {
    let changed = view_abi::signal(0, false);
    let out = view_abi::list();

    let btn = tmpl!(TMPL_COND_BTN);
    view_abi::handler(&btn, 0, move |_ev| {
        changed.set(!changed.get());
    });
    view_abi::push(&out, btn);

    view_abi::push(
        &out,
        view_abi::keyed_list_v1(
            move || {
                if changed.get() {
                    &SLICE_AFTER[..]
                } else {
                    &SLICE_BEFORE[..]
                }
            },
            |row| row.0,
            |row, idx| {
                let li = tmpl!(TMPL_KEYED_LIVE_ROW);
                view_abi::hole(&li, 0, row.0);
                view_abi::effect(&li, 1, move || view_abi::text(row.0));
                view_abi::effect(&li, 2, move || view_abi::text(row.1));
                view_abi::handler(&li, 3, move |_ev| {
                    note_rust("keyed-slice-click", row.1, (idx + 1) as u32, row.0);
                });
                li
            },
        ),
    );
    out
}

#[unsafe(no_mangle)]
pub fn build_keyed_vec_live() -> Node {
    let before: &'static Vec<(&'static str, &'static str)> =
        Box::leak(Box::new(Vec::from([("é😀", "α"), ("€", "β")])));
    let after: &'static Vec<(&'static str, &'static str)> = Box::leak(Box::new(Vec::from([
        ("€", "β-updated-€"),
        ("é😀", "α-updated-😀"),
        ("γ", "new-γ"),
    ])));
    let changed = view_abi::signal(0, false);
    let out = view_abi::list();

    let btn = tmpl!(TMPL_COND_BTN);
    view_abi::handler(&btn, 0, move |_ev| {
        changed.set(!changed.get());
    });
    view_abi::push(&out, btn);

    view_abi::push(
        &out,
        view_abi::keyed_list_v1(
            move || if changed.get() { after } else { before },
            |row| row.0,
            |row, idx| {
                let li = tmpl!(TMPL_KEYED_LIVE_ROW);
                view_abi::hole(&li, 0, row.0);
                view_abi::effect(&li, 1, move || row.0);
                view_abi::effect(&li, 2, move || row.1);
                view_abi::handler(&li, 3, move |_ev| {
                    note_rust(
                        "keyed-vec-click",
                        row.1.len() as u32,
                        (idx + 1) as u32,
                        row.1,
                    );
                });
                li
            },
        ),
    );
    out
}

#[unsafe(no_mangle)]
pub fn build_keyed_vec_static() -> Node {
    let changed = view_abi::signal(0, false);
    let out = view_abi::list();

    let btn = tmpl!(TMPL_COND_BTN);
    view_abi::handler(&btn, 0, move |_ev| {
        changed.set(!changed.get());
    });
    view_abi::push(&out, btn);

    view_abi::push(
        &out,
        view_abi::keyed_list_v1(
            move || {
                if changed.get() {
                    Vec::from([("€", "two-€"), ("é😀", "one-😀"), ("ω", "three-ω")])
                } else {
                    Vec::from([("é😀", "one-😀"), ("€", "two-€")])
                }
            },
            |row| row.0,
            |row, _idx| {
                let li = tmpl!(TMPL_KEYED_STATIC_ROW);
                view_abi::hole(&li, 0, row.0);
                view_abi::hole(&li, 1, view_abi::text(row.0));
                view_abi::hole(&li, 2, view_abi::text(row.1));
                li
            },
        ),
    );
    out
}
