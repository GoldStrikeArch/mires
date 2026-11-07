//! `mires-vanilla`: the krausest js-framework-benchmark row store, compiled from Rust and
//! driving the DOM itself.
//!
//! There is no framework here and no client runtime. The whole benchmark app is this one
//! `#![no_std]` crate: it owns the row data in a `Vec`, it owns the `<tr>` elements in a second
//! `Vec`, and it reconciles the two with [`App::sync_view`]. What ships to a browser is this
//! crate compiled to JavaScript, the subset of the runtime shim it reads, and a bootstrap that
//! wires six buttons and one delegated click to the exported entry points. Every decision the
//! app makes is made in Rust.
//!
//! # The algorithm
//!
//! Non-keyed recycling, the same shape as the benchmark's own `vanillajs` non-keyed entry. A
//! `<tr>` belongs to a position in the table, not to a row of data: replacing the data rewrites
//! the text of the elements already there, a longer list appends clones of a template, and a
//! shorter one drops the elements off the END. So `run` twice clones nothing, deleting row two
//! shifts every later row's text up by one and removes the last element, and swapping rows one
//! and 998 writes four strings and moves no nodes. Those three are exactly what the harness's
//! `isKeyed` check looks at, and all three answer "not keyed".
//!
//! A row carries a dirty flag so that a sync writes only the cells whose contents changed: an
//! update of every tenth row is a hundred writes, not two thousand. The flag is set by whatever
//! changed the row and cleared by the write, which keeps `sync_view` the only place that touches
//! the table.
//!
//! # Crossing to the host
//!
//! [`JsValue`] is an opaque host value: one machine word so MIR keeps it, `repr(transparent)` so
//! it is the value rather than an object holding one. Every DOM operation is a `#[js_extern]`
//! declaration rooted at the global scope or at its first argument, so the emitted program
//! imports nothing and loads as a plain script.
//!
//! A `&str` crosses as a JavaScript string, and the `f64` overload of `textContent` crosses as a
//! number, so the id cell needs no formatting on either side.

#![no_std]
#![no_main]
#![allow(improper_ctypes_definitions)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use js_extern_macro::js_extern;

/// The panic handler `core` is owed. Nothing here panics: every index is guarded and every
/// fallible read is an `Option` that is matched. A loop rather than a message keeps the panic
/// path out of the payload.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// An opaque JavaScript value: a DOM node, in every use here.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsValue {
    /// Never read by Rust code; see the module docs.
    #[allow(dead_code)]
    handle: u32,
}

// ---------------------------------------------------------------------------
// The host surface
// ---------------------------------------------------------------------------
//
// Every declaration is rooted at the global scope or at its first argument, so none of them
// imports anything: the emitted program is a script, not a module.

#[js_extern]
unsafe extern "C" {
    #[js(global, call = "document.getElementById")]
    fn get_element_by_id(id: &str) -> JsValue;

    #[js(global, call = "document.createElement")]
    fn create_element(tag: &str) -> JsValue;

    #[js(global, call = "document.createDocumentFragment")]
    fn create_document_fragment() -> JsValue;

    #[js(method = "appendChild")]
    fn append_child(parent: JsValue, child: JsValue);

    #[js(method = "removeChild")]
    fn remove_child(parent: JsValue, child: JsValue);

    #[js(method = "cloneNode")]
    fn clone_node(node: JsValue, deep: bool) -> JsValue;

    /// The template's children are built by this crate, so the walk from a `<tr>` to its cells
    /// never meets a null and needs no `Option` (and so no unwrap, and so no panic path).
    #[js(get = "firstChild")]
    fn first_child(node: JsValue) -> JsValue;

    #[js(get = "nextSibling")]
    fn next_sibling(node: JsValue) -> JsValue;

    #[js(set = "textContent")]
    fn set_text(node: JsValue, text: &str);

    /// The id cell: a number crosses as a number, so there is no integer to format.
    #[js(set = "textContent")]
    fn set_number(node: JsValue, value: f64);

    #[js(set = "className")]
    fn set_class_name(node: JsValue, value: &str);

    #[js(set = "innerHTML")]
    fn set_inner_html(node: JsValue, html: &str);

    /// Null for a click that landed outside the table body.
    #[js(method = "closest", nullable)]
    fn closest(node: JsValue, selector: &str) -> Option<JsValue>;

    #[js(method = "matches")]
    fn matches(node: JsValue, selector: &str) -> bool;

    /// A `<tr>`'s position in its section, which is a non-keyed row's whole identity.
    #[js(get = "sectionRowIndex")]
    fn section_row_index(row: JsValue) -> f64;

    #[js(global, call = "Math.random")]
    fn random() -> f64;

    #[js(global, call = "Math.round")]
    fn round(value: f64) -> f64;
}

// ---------------------------------------------------------------------------
// The label vocabulary
// ---------------------------------------------------------------------------
//
// The benchmark's own three arrays, in its order and with its contents: 25 adjectives, 11
// colours with "brown" in it twice, 13 nouns. A label is one of each, in that order, joined by
// spaces.

static ADJECTIVES: [&str; 25] = [
    "pretty",
    "large",
    "big",
    "small",
    "tall",
    "short",
    "long",
    "handsome",
    "plain",
    "quaint",
    "clean",
    "elegant",
    "easy",
    "angry",
    "crazy",
    "helpful",
    "mushy",
    "odd",
    "unsightly",
    "adorable",
    "important",
    "inexpensive",
    "cheap",
    "expensive",
    "fancy",
];

static COLOURS: [&str; 11] = [
    "red", "yellow", "blue", "green", "pink", "brown", "purple", "brown", "white", "black",
    "orange",
];

static NOUNS: [&str; 13] = [
    "table", "chair", "house", "bbq", "desk", "car", "pony", "cookie", "sandwich", "burger",
    "pizza", "mouse", "keyboard",
];

/// The markup of one row, which every row is a deep clone of.
///
/// `innerHTML` on a detached `<tr>` parses in table-row context, so the cells arrive as cells.
/// The delete anchor carries `remove` so the click handler can tell a delete from a select with
/// one `matches`, and the label anchor carries `lbl` for the harness's own selectors.
static ROW_HTML: &str = concat!(
    "<td class=\"col-md-1\"></td>",
    "<td class=\"col-md-4\"><a class=\"lbl\"></a></td>",
    "<td class=\"col-md-1\"><a class=\"remove\">",
    "<span class=\"glyphicon glyphicon-remove\" aria-hidden=\"true\"></span></a></td>",
    "<td class=\"col-md-6\"></td>",
);

/// One word out of `words`, drawn the way the benchmark draws it:
/// `Math.round(Math.random() * 1000) % words.length`, bit for bit. The draw is biased, and
/// reproducing the bias is the point.
///
/// Nothing here can panic, which is what keeps `core`'s integer formatter out of the payload: an
/// index that could be out of bounds puts the "index out of bounds" message on the call graph,
/// and that message formats two numbers.
fn pick(words: &[&'static str]) -> &'static str {
    let drawn = round(random() * 1000.0) as usize;
    let count = words.len();
    if count == 0 {
        return "";
    }
    words.get(drawn % count).copied().unwrap_or("")
}

/// One row of data.
struct Row {
    /// Numbered from one across the life of the page, never reset.
    id: f64,
    label: String,
    /// Whether the element showing this row is out of date. Set by whatever changed the row,
    /// cleared by the write in [`App::sync_view`].
    dirty: bool,
}

impl Row {
    /// A fresh row: a new id and a label drawn from the three vocabularies.
    fn new(id: f64) -> Row {
        // One allocation, sized for the longest label there is ("inexpensive yellow keyboard"
        // plus room for the update suffix), and three appends. Never a format.
        let mut label = String::with_capacity(32);
        label.push_str(pick(&ADJECTIVES));
        label.push(' ');
        label.push_str(pick(&COLOURS));
        label.push(' ');
        label.push_str(pick(&NOUNS));
        Row { id, label, dirty: true }
    }
}

/// The whole application: the data, the elements showing it, and where the selection is.
///
/// It is a value the bootstrap holds and hands back to every entry point, so there is no mutable
/// static anywhere in the program.
pub struct App {
    data: Vec<Row>,
    /// The `<tr>` elements in the table body, in document order. `rows[i]` shows `data[i]` after
    /// a sync; between a change and the sync the two lengths may differ, which is the whole of
    /// what a sync has to fix.
    rows: Vec<JsValue>,
    tbody: JsValue,
    /// The detached `<tr>` every row is cloned from.
    template: JsValue,
    /// The position of the highlighted row, if any. A non-keyed row IS its position.
    selected: Option<usize>,
    next_id: f64,
}

impl App {
    /// Finds the table body and builds the template row.
    fn new() -> App {
        let template = create_element("tr");
        set_inner_html(template, ROW_HTML);
        App {
            data: Vec::new(),
            rows: Vec::new(),
            tbody: get_element_by_id("tbody"),
            template,
            selected: None,
            next_id: 1.0,
        }
    }

    /// Appends `count` fresh rows to the data. Ids continue from wherever they had reached.
    fn append_rows(&mut self, count: usize) {
        self.data.reserve(count);
        for _ in 0..count {
            let id = self.next_id;
            self.next_id += 1.0;
            self.data.push(Row::new(id));
        }
    }

    /// Replaces every row with `count` new ones.
    fn run(&mut self, count: usize) {
        self.unselect();
        self.data.clear();
        self.append_rows(count);
        self.sync_view();
    }

    /// Appends a thousand rows to whatever is there.
    fn add(&mut self) {
        self.append_rows(1000);
        self.sync_view();
    }

    /// Appends " !!!" to every tenth label, counting from the first row.
    fn update(&mut self) {
        let mut at = 0;
        while let Some(row) = self.data.get_mut(at) {
            row.label.push_str(" !!!");
            row.dirty = true;
            at += 10;
        }
        self.sync_view();
    }

    /// Empties the table.
    fn clear(&mut self) {
        self.unselect();
        self.data.clear();
        self.sync_view();
    }

    /// Exchanges the second row with the 999th, and does nothing at all if there are not that
    /// many rows.
    ///
    /// The split is the guard: a list of 998 rows or fewer leaves the second half empty, and the
    /// two halves are what makes holding both rows at once legal.
    fn swap_rows(&mut self) {
        let Some((head, tail)) = self.data.split_at_mut_checked(998) else {
            return;
        };
        let (Some(second), Some(last)) = (head.get_mut(1), tail.first_mut()) else {
            return;
        };
        core::mem::swap(second, last);
        second.dirty = true;
        last.dirty = true;
        self.sync_view();
    }

    /// Deletes the row at `index`. Every row after it moves up one position, which in a
    /// non-keyed table means their elements show different text and the last element goes.
    fn remove(&mut self, index: usize) {
        let Some(tail) = self.data.get_mut(index..) else {
            return;
        };
        let mut rows = tail.iter_mut();
        let Some(mut hole) = rows.next() else {
            return;
        };
        // Each swap fills the position before it and carries the deleted row along, so the row
        // that ends up last is the deleted one and the pop drops it.
        for row in rows {
            core::mem::swap(&mut *hole, &mut *row);
            hole.dirty = true;
            hole = row;
        }
        self.data.pop();
        self.sync_view();
    }

    /// Highlights the row at `index`, and only it. No sync: a selection changes no data, so
    /// nothing but two class names has to move.
    fn select(&mut self, index: usize) {
        let Some(element) = self.rows.get(index).copied() else {
            return;
        };
        self.unselect();
        set_class_name(element, "danger");
        self.selected = Some(index);
    }

    /// Drops the highlight, if there is one and its element is still in the table.
    fn unselect(&mut self) {
        if let Some(at) = self.selected.take() {
            if let Some(element) = self.rows.get(at) {
                set_class_name(*element, "");
            }
        }
    }

    /// Makes the table show the data: rewrite, append, then drop.
    ///
    /// The three parts are the whole algorithm. Positions both lists have are rewritten in
    /// place, and only where a row said it changed. Positions only the data has get a clone of
    /// the template, collected in a fragment so the table body is touched once however many
    /// rows arrived. Positions only the table has are dropped from the END, which is what makes
    /// this non-keyed: the elements that stay are the leading ones, whatever data they had.
    fn sync_view(&mut self) {
        let data_len = self.data.len();
        let view_len = self.rows.len();

        let shared = if data_len < view_len { data_len } else { view_len };
        for at in 0..shared {
            let (Some(row), Some(element)) = (self.data.get_mut(at), self.rows.get(at)) else {
                break;
            };
            if !row.dirty {
                continue;
            }
            row.dirty = false;
            write_row(*element, row);
        }

        if data_len > view_len {
            let fragment = create_document_fragment();
            self.rows.reserve(data_len - view_len);
            for at in view_len..data_len {
                let Some(row) = self.data.get_mut(at) else {
                    break;
                };
                row.dirty = false;
                let element = clone_node(self.template, true);
                write_row(element, row);
                append_child(fragment, element);
                self.rows.push(element);
            }
            append_child(self.tbody, fragment);
        } else if data_len == 0 {
            // Emptying the table is one write rather than a thousand removals.
            if view_len > 0 {
                set_text(self.tbody, "");
                self.rows.clear();
            }
        } else {
            while self.rows.len() > data_len {
                if let Some(row) = self.rows.pop() {
                    remove_child(self.tbody, row);
                }
            }
        }
    }
}

/// Writes a row's id and label into the cells of the element showing it.
///
/// The walk is `tr.firstChild` for the id cell, its `nextSibling` for the label cell and that
/// cell's `firstChild` for the anchor. Reading it back each time costs three property reads and
/// saves holding two more handles per row.
fn write_row(element: JsValue, row: &Row) {
    let id_cell = first_child(element);
    set_number(id_cell, row.id);
    let label_anchor = first_child(next_sibling(id_cell));
    set_text(label_anchor, &row.label);
}

// ---------------------------------------------------------------------------
// The entry points
// ---------------------------------------------------------------------------
//
// bootstrap.js calls `bench_init` once, holds what it answers, and hands it back to every other
// call. A reference to a struct IS the object in this model, so what crosses is the application
// itself and there is no handle table and no mutable static.

/// Builds the application. Call once, after the document has the table body in it.
#[unsafe(no_mangle)]
pub extern "C" fn bench_init() -> &'static mut App {
    Box::leak(Box::new(App::new()))
}

/// Create 1,000 rows.
#[unsafe(no_mangle)]
pub extern "C" fn bench_run(app: &mut App) {
    app.run(1000);
}

/// Create 10,000 rows.
#[unsafe(no_mangle)]
pub extern "C" fn bench_runlots(app: &mut App) {
    app.run(10000);
}

/// Append 1,000 rows.
#[unsafe(no_mangle)]
pub extern "C" fn bench_add(app: &mut App) {
    app.add();
}

/// Update every 10th row.
#[unsafe(no_mangle)]
pub extern "C" fn bench_update(app: &mut App) {
    app.update();
}

/// Clear.
#[unsafe(no_mangle)]
pub extern "C" fn bench_clear(app: &mut App) {
    app.clear();
}

/// Swap rows.
#[unsafe(no_mangle)]
pub extern "C" fn bench_swaprows(app: &mut App) {
    app.swap_rows();
}

/// A click anywhere in the table body.
///
/// The bootstrap passes the event target and nothing else: which row it was, and whether the
/// click was on the delete anchor or on the row, are decided here.
#[unsafe(no_mangle)]
pub extern "C" fn bench_click(app: &mut App, target: JsValue) {
    let Some(row) = closest(target, "tr") else {
        return;
    };
    let index = section_row_index(row);
    if index < 0.0 {
        return;
    }
    if matches(target, ".remove, .remove *") {
        app.remove(index as usize);
    } else {
        app.select(index as usize);
    }
}
