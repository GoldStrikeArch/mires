//! Standard library macros (`println!`, `print!`, `eprintln!`, `eprint!`,
//! `dbg!`, `panic!`, `thread_local!`).

/// Panics the current thread.
#[macro_export]
macro_rules! panic {
    () => {
        $crate::rt::panic_static("explicit panic")
    };
    ($($arg:tt)+) => {
        $crate::rt::panic_fmt_rt($crate::format_args!($($arg)+))
    };
}

/// Prints to the standard output.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::io::_print($crate::format_args!($($arg)*));
    }};
}

/// Prints to the standard output, with a newline.
#[macro_export]
macro_rules! println {
    () => {{
        $crate::io::_println($crate::format_args!(""));
    }};
    ($($arg:tt)*) => {{
        $crate::io::_println($crate::format_args!($($arg)*));
    }};
}

/// Prints to the standard error.
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {{
        $crate::io::_eprint($crate::format_args!($($arg)*));
    }};
}

/// Prints to the standard error, with a newline.
#[macro_export]
macro_rules! eprintln {
    () => {{
        $crate::io::_eprintln($crate::format_args!(""));
    }};
    ($($arg:tt)*) => {{
        $crate::io::_eprintln($crate::format_args!($($arg)*));
    }};
}

/// Prints and returns the value of a given expression for quick and dirty
/// debugging.
#[macro_export]
macro_rules! dbg {
    () => {
        $crate::eprintln!("[{}:{}:{}]", $crate::file!(), $crate::line!(), $crate::column!())
    };
    ($val:expr $(,)?) => {
        match $val {
            tmp => {
                $crate::eprintln!(
                    "[{}:{}:{}] {} = {:#?}",
                    $crate::file!(),
                    $crate::line!(),
                    $crate::column!(),
                    $crate::stringify!($val),
                    &tmp
                );
                tmp
            }
        }
    };
    ($($val:expr),+ $(,)?) => {
        ($($crate::dbg!($val)),+,)
    };
}

/// Declares a new thread-local storage key of type [`crate::thread::LocalKey`].
///
/// On single-threaded JavaScript targets, each key is backed by a static
/// single-threaded cell initialized lazily (or directly for `const { ... }`).
#[macro_export]
macro_rules! thread_local {
    () => {};

    // Const initialization form with following items.
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const $init:block; $($rest:tt)*) => (
        $crate::__thread_local_inner!($(#[$attr])* $vis $name, $t, const $init);
        $crate::thread_local!($($rest)*);
    );

    // Const initialization form (final item, optional trailing semicolon).
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const $init:block) => (
        $crate::__thread_local_inner!($(#[$attr])* $vis $name, $t, const $init);
    );

    // Lazy initialization form with following items.
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => (
        $crate::__thread_local_inner!($(#[$attr])* $vis $name, $t, $init);
        $crate::thread_local!($($rest)*);
    );

    // Lazy initialization form (final item, optional trailing semicolon).
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => (
        $crate::__thread_local_inner!($(#[$attr])* $vis $name, $t, $init);
    );
}

#[doc(hidden)]
#[macro_export]
macro_rules! __thread_local_inner {
    ($(#[$attr:meta])* $vis:vis $name:ident, $t:ty, const $init:block) => {
        $(#[$attr])*
        $vis static $name: $crate::thread::LocalKey<$t> = {
            static __KEY: $crate::thread::__ConstLocalStorage<$t> =
                $crate::thread::__ConstLocalStorage::new(const $init);
            fn __get() -> &'static $t {
                __KEY.get()
            }
            $crate::thread::LocalKey::__new(__get)
        };
    };
    ($(#[$attr:meta])* $vis:vis $name:ident, $t:ty, $init:expr) => {
        $(#[$attr])*
        $vis static $name: $crate::thread::LocalKey<$t> = {
            static __KEY: $crate::thread::__LazyLocalStorage<$t> =
                $crate::thread::__LazyLocalStorage::new();
            fn __init() -> $t {
                $init
            }
            fn __get() -> &'static $t {
                __KEY.get_or_init(__init)
            }
            $crate::thread::LocalKey::__new(__get)
        };
    };
}
