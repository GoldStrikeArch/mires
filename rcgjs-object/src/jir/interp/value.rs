//! The abstract values both interpreters compute with.
//!
//! Values are Rust values, not JavaScript ones: an integer knows its width and signedness, an
//! aggregate its type and active variant, a pointer the heap location it names. Nothing here
//! says how a value is spelled in JavaScript; that is the carrier's business.

use std::fmt;

use crate::jir::interns::StrId;
use crate::jir::semantic::TypeId;

/// A heap object: an index into the interpreter's [`super::heap::Heap`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HeapId(pub u32);

/// One step down into an aggregate: a field of a struct, tuple, or enum payload, or an element of
/// an array.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Step {
    Field(u32),
    Index(u64),
}

/// A place: a heap object and the path of steps into it.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Location {
    pub heap: HeapId,
    pub path: Vec<Step>,
}

impl Location {
    pub fn root(heap: HeapId) -> Self {
        Self {
            heap,
            path: Vec::new(),
        }
    }

    pub fn child(&self, step: Step) -> Self {
        let mut path = self.path.clone();
        path.push(step);
        Self {
            heap: self.heap,
            path,
        }
    }
}

impl Default for HeapId {
    fn default() -> Self {
        Self(u32::MAX)
    }
}

/// An abstract value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Storage that was never written (a fresh local or cell).
    Undef,
    Unit,
    Bool(bool),
    /// An integer of the given width and signedness, held sign- or zero-extended in `value` so
    /// equal integers compare equal.
    Int {
        value: i128,
        bits: u32,
        signed: bool,
    },
    /// An `f32` (`bits == 32`, already rounded to single precision) or `f64`.
    Float {
        value: f64,
        bits: u32,
    },
    Char(char),
    /// A `&str`: the text itself (string data is immutable and has no identity worth keeping).
    Str(String),
    /// A struct, tuple, array, closure environment, or enum value. `variant` is `0` for a
    /// non-enum; `fields` are the fields of the active variant (or the elements of an array).
    Aggregate {
        ty: TypeId,
        variant: u32,
        fields: Vec<Value>,
    },
    /// A reference or raw pointer to one place.
    Ptr(Location),
    /// A slice reference or raw slice pointer: `len` elements of the array at `buf` starting at
    /// element `off`.
    Slice {
        buf: Location,
        off: u64,
        len: u64,
    },
    /// A function value by its link name (a function item, function pointer, or closure that
    /// erases to one).
    Fn(StrId),
    /// A value the interpreter cannot inspect, tagged with why. Comparing two opaque values is
    /// unsupported.
    Opaque(&'static str),
}

impl Value {
    /// An integer of type `(signed, bits)` from raw bits, normalized (sign-extended when signed,
    /// zero-extended otherwise).
    pub fn int(bits_value: u128, bits: u32, signed: bool) -> Self {
        Self::Int {
            value: normalize_int(bits_value as i128, bits, signed),
            bits,
            signed,
        }
    }

    /// The `usize` of the interpreter's target: 32 bits (`wasm32-unknown-unknown`).
    pub fn usize(value: u64) -> Self {
        Self::int(value as u128, 32, false)
    }

    /// An `f32` value (rounded to single precision) or `f64` value.
    pub fn float(value: f64, bits: u32) -> Self {
        Self::Float {
            value: if bits == 32 {
                f64::from(value as f32)
            } else {
                value
            },
            bits,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i128> {
        match self {
            Self::Int { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// The raw unsigned bit pattern of an integer, `bool`, or `char`, as MIR's `SwitchInt`
    /// carries case values.
    pub fn as_switch_bits(&self) -> Option<u128> {
        match self {
            Self::Int { value, bits, .. } => Some(truncate_bits(*value as u128, *bits)),
            Self::Bool(b) => Some(u128::from(*b)),
            Self::Char(c) => Some(u128::from(u32::from(*c))),
            _ => None,
        }
    }

    pub fn is_aggregate(&self) -> bool {
        matches!(self, Self::Aggregate { .. })
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undef => f.write_str("undef"),
            Self::Unit => f.write_str("()"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int {
                value,
                bits,
                signed,
            } => write!(f, "{value}{}{bits}", if *signed { "i" } else { "u" }),
            Self::Float { value, bits } => write!(f, "{value}f{bits}"),
            Self::Char(c) => write!(f, "{c:?}"),
            Self::Str(s) => write!(f, "{s:?}"),
            Self::Aggregate {
                variant, fields, ..
            } => {
                write!(f, "#{variant}(")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{field}")?;
                }
                f.write_str(")")
            }
            Self::Ptr(location) => write!(f, "&{location:?}"),
            Self::Slice { buf, off, len } => write!(f, "&{buf:?}[{off}..{}]", off + len),
            Self::Fn(name) => write!(f, "fn#{}", name.0),
            Self::Opaque(why) => write!(f, "opaque({why})"),
        }
    }
}

/// Keeps the low `bits` of `value`.
pub fn truncate_bits(value: u128, bits: u32) -> u128 {
    if bits >= 128 {
        value
    } else {
        value & ((1_u128 << bits) - 1)
    }
}

/// Wraps `value` into the range of an integer of `(signed, bits)`.
pub fn normalize_int(value: i128, bits: u32, signed: bool) -> i128 {
    if bits >= 128 {
        return value;
    }
    let truncated = truncate_bits(value as u128, bits);
    if signed && truncated & (1_u128 << (bits - 1)) != 0 {
        (truncated | !((1_u128 << bits) - 1)) as i128
    } else {
        truncated as i128
    }
}

/// The inclusive range of an integer type.
pub fn int_range(bits: u32, signed: bool) -> (i128, i128) {
    match (signed, bits) {
        (true, 128) => (i128::MIN, i128::MAX),
        (false, 128) => (0, i128::MAX),
        (true, bits) => (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1),
        (false, bits) => (0, (1_i128 << bits) - 1),
    }
}
