//! The Rust Standard Library Prelude.

/// The first version of the prelude of the Rust Standard Library.
pub mod v1 {
    // Re-exported core operators and marker traits.
    pub use crate::marker::{Send, Sized, Sync, Unpin};
    pub use crate::ops::{AsyncFn, AsyncFnMut, AsyncFnOnce, Drop, Fn, FnMut, FnOnce};

    // Re-exported functions.
    pub use crate::mem::{align_of, align_of_val, drop, size_of, size_of_val};

    // Re-exported types and traits.
    pub use crate::convert::{AsMut, AsRef, From, Into};
    pub use crate::iter::{DoubleEndedIterator, ExactSizeIterator, Extend, IntoIterator, Iterator};
    pub use crate::option::Option::{self, None, Some};
    pub use crate::result::Result::{self, Err, Ok};

    // Re-exported built-in macros and derive traits from `core`.
    #[allow(deprecated)]
    pub use core_crate::prelude::v1::{
        Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, alloc_error_handler,
        assert, assert_eq, assert_ne, bench, cfg, cfg_select, column, compile_error, concat,
        debug_assert, debug_assert_eq, debug_assert_ne, derive, env, file, format_args,
        global_allocator, include, include_bytes, include_str, line, matches, module_path,
        option_env, stringify, test, test_case, todo, r#try, unimplemented, unreachable, write,
        writeln,
    };

    // Re-exported std macros.
    pub use crate::{dbg, eprint, eprintln, format, print, println, thread_local};

    // Export only the `vec!` and `panic!` macros without exposing the `vec` and `panic` modules.
    mod ambiguous_macros_only {
        #[allow(hidden_glob_reexports)]
        mod vec {}
        #[allow(hidden_glob_reexports)]
        mod panic {}
        pub use crate::*;
    }
    pub use self::ambiguous_macros_only::{panic, vec};

    // Re-exported items from `alloc`.
    pub use crate::borrow::ToOwned;
    pub use crate::boxed::Box;
    pub use crate::string::{String, ToString};
    pub use crate::vec::Vec;
}

/// The 2015 edition prelude.
pub mod rust_2015 {
    pub use super::v1::*;
}

/// The 2018 edition prelude.
pub mod rust_2018 {
    pub use super::v1::*;
}

/// The 2021 edition prelude.
pub mod rust_2021 {
    pub use super::v1::*;
    pub use crate::convert::{TryFrom, TryInto};
    pub use crate::iter::FromIterator;
}

/// The 2024 edition prelude.
pub mod rust_2024 {
    pub use super::v1::*;
    pub use crate::convert::{TryFrom, TryInto};
    pub use crate::future::{Future, IntoFuture};
    pub use crate::iter::FromIterator;
}

/// The future edition prelude.
#[doc(hidden)]
pub mod rust_future {
    pub use super::rust_2024::*;
}
