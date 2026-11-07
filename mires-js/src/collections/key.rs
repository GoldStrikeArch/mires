//! What may be a key, and what a key compares as.

use alloc::string::String;

/// A type a host `Map` can key on.
///
/// Sealed: the integers, `bool`, `char`, `str`, `&str` and `String`, and nothing else ever. A host
/// `Map` compares keys with SameValueZero, which is by value for a number, a string, a boolean and
/// a BigInt and is object *identity* for everything else. An aggregate key would therefore be
/// stored under the object that carried it and never found again by an equal one, so the bound is
/// what turns that into a compile error at the call site.
pub trait MapKey: sealed::Sealed {
    /// What this key compares as.
    ///
    /// `String`, `&str` and `str` all compare as `str`; every other key compares as itself. A
    /// lookup takes the class rather than the key type, which is what lets a `HashMap<String, V>`
    /// be read with a `&str` and never needs a `String` built to ask a question.
    type Class: ?Sized;

    /// How one key appears in the array [`crate::collections::HashMap::iter`] snapshots the keys
    /// into.
    ///
    /// The same as the class for a key that is already a value, and `&'static str` for the string
    /// class, whose class is unsized. The `'static` is the model's own convention for a host
    /// string: a JavaScript string is a value owned by nobody (`CONTRACT.md`, "The string sink").
    type Snapshot: Copy + 'static;

    /// This key, as the class it compares in.
    fn as_class(&self) -> &Self::Class;

    /// One snapshotted key, as the class it compares in.
    fn class_of(snapshot: &Self::Snapshot) -> &Self::Class;
}

/// The integers, `bool` and `char`: each compares as itself and snapshots as itself.
macro_rules! value_keys {
    ($($ty:ty),* $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}

            impl MapKey for $ty {
                type Class = $ty;
                type Snapshot = $ty;

                fn as_class(&self) -> &$ty {
                    self
                }

                fn class_of(snapshot: &$ty) -> &$ty {
                    snapshot
                }
            }
        )*
    };
}

value_keys!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, bool, char);

impl sealed::Sealed for str {}

impl MapKey for str {
    type Class = str;
    type Snapshot = &'static str;

    fn as_class(&self) -> &str {
        self
    }

    fn class_of<'a>(snapshot: &'a &'static str) -> &'a str {
        snapshot
    }
}

impl sealed::Sealed for &str {}

impl MapKey for &str {
    type Class = str;
    type Snapshot = &'static str;

    fn as_class(&self) -> &str {
        self
    }

    fn class_of<'a>(snapshot: &'a &'static str) -> &'a str {
        snapshot
    }
}

impl sealed::Sealed for String {}

impl MapKey for String {
    type Class = str;
    type Snapshot = &'static str;

    /// The host string the bytes decode to, which is one `__rt.bytes_str` per operation. The key
    /// the map holds is that string and not the `String`, so the two go their own ways from the
    /// moment the key goes in: dropping the `String` frees its heap block and leaves the map's key
    /// standing.
    fn as_class(&self) -> &str {
        self.as_str()
    }

    fn class_of<'a>(snapshot: &'a &'static str) -> &'a str {
        snapshot
    }
}

mod sealed {
    /// Closes [`super::MapKey`]. Implementing it outside this crate is impossible, which is what
    /// makes an unsupported key a type error rather than a lookup that never matches.
    pub trait Sealed {}
}
