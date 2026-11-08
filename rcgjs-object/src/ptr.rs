//! The shape of a JavaScript pointer value: the field names and the constructors that build one.
//!
//! The rustc-free half of the backend's `ptr.rs` — the part that only builds JavaScript. Every
//! reader and writer that has to consult a `tcx` layout stays there; what lives here is the record
//! shape itself, which the link-time lowerings in `jsc-link` also have to build.

use crate::jsast::{self, Expr};

/// The buffer half of a slot.
pub const BUF: &str = "buf";
/// The offset half of a slot, counted in units of the pointee type.
pub const OFF: &str = "off";
/// The element size a scaled slot's byte offset is measured against.
pub const SCALE: &str = "sc";
/// The element count a window slot spans.
pub const WINDOW: &str = "w";
/// The length of a fat slice pointer.
pub const LEN: &str = "len";
/// The data half of a fat trait object pointer.
pub const PTR: &str = "ptr";
/// The vtable half of a fat trait object pointer.
pub const META: &str = "meta";

// ---------------------------------------------------------------------------------------------
// Building and taking apart the records
// ---------------------------------------------------------------------------------------------

/// `{ buf, off }`: a pointer to `buf[off]`.
pub fn slot(buf: Expr, off: Expr) -> Expr {
    jsast::object_of(vec![(BUF, buf), (OFF, off)])
}

/// `{ buf, off, sc }`: a byte offset view of a buffer whose elements are `scale` bytes wide.
///
/// This is what `p as *const u8` produces for a pointee larger than a byte. `off` is a *byte*
/// offset, so the element it names is `buf[off / sc]` and only an offset that is a multiple of
/// `sc` names one at all. A scaled slot is not dereferenceable: it is meant to be offset and then
/// cast back, which is what the `__rt.unscale` helper does.
pub fn scaled_slot(buf: Expr, off: Expr, scale: u64) -> Expr {
    jsast::object_of(vec![
        (BUF, buf),
        (OFF, off),
        (SCALE, jsast::num(scale as f64)),
    ])
}

/// `{ buf, off, w }`: a pointer to the `width` elements starting at `buf[off]`.
///
/// This is what `p as *const [E; N]` produces. The window is how a pointer to an array of
/// elements stays inside the buffer those elements live in, rather than copying them out.
pub fn window_slot(buf: Expr, off: Expr, width: u64) -> Expr {
    jsast::object_of(vec![
        (BUF, buf),
        (OFF, off),
        (WINDOW, jsast::num(width as f64)),
    ])
}

/// `{ buf, off, len }`: a fat pointer to `len` elements starting at `buf[off]`.
pub fn fat_slice(buf: Expr, off: Expr, len: Expr) -> Expr {
    jsast::object_of(vec![(BUF, buf), (OFF, off), (LEN, len)])
}

/// `{ ptr, meta }`: a fat pointer to a trait object.
pub fn fat_dyn(data: Expr, meta: Expr) -> Expr {
    jsast::object_of(vec![(PTR, data), (META, meta)])
}

/// `p.buf`.
pub fn slot_buf(pointer: Expr) -> Expr {
    jsast::member(pointer, BUF)
}

/// `p.off`.
pub fn slot_off(pointer: Expr) -> Expr {
    jsast::member(pointer, OFF)
}

/// `p.sc`, the element size a scaled slot's offset is measured against.
pub fn slot_scale(pointer: Expr) -> Expr {
    jsast::member(pointer, SCALE)
}

/// `p.w`, the element count a window slot spans.
pub fn slot_window(pointer: Expr) -> Expr {
    jsast::member(pointer, WINDOW)
}

/// `p.len`, the length of a fat slice pointer.
pub fn slot_len(pointer: Expr) -> Expr {
    jsast::member(pointer, LEN)
}

/// `p.buf[p.off]`: the place a slot points at, as an assignable JavaScript expression.
///
/// `pointer` is evaluated twice, so a caller holding anything but a local or a field hoists it
/// into a temporary first (the producer's `FnCx::temp`).
pub fn slot_element(pointer: Expr) -> Expr {
    jsast::index(slot_buf(pointer.clone()), slot_off(pointer))
}
