// Minimal `core` replacement for the rustc_codegen_js spike.
//
// Adapted from rustc_codegen_cranelift's `example/mini_core.rs`. ITEMS ONLY -- the
// crate-level attributes live in every test's fixed header (see CONTRACT.md).
//
// Everything to do with allocation, `Box`, atomics and `va_list` has been removed, and
// the intrinsic surface is cut down to `ptr_metadata`; the operator traits are
// implemented directly on the primitives so rustc lowers them to primitive MIR
// `BinaryOp`/`UnaryOp` rather than to a library call.

// ---------------------------------------------------------------------------
// Sizedness hierarchy
// ---------------------------------------------------------------------------

#[rustc_builtin_macro]
#[rustc_macro_transparency = "semiopaque"]
pub macro stringify($($t:tt)*) {
    /* compiler built-in */
}

#[lang = "pointee_sized"]
pub trait PointeeSized {}

#[lang = "meta_sized"]
pub trait MetaSized: PointeeSized {}

#[lang = "sized"]
pub trait Sized: MetaSized {}

#[lang = "destruct"]
pub trait Destruct {}

#[lang = "tuple_trait"]
pub trait Tuple {}

#[lang = "legacy_receiver"]
pub trait LegacyReceiver {}

impl<T: PointeeSized> LegacyReceiver for &T {}
impl<T: PointeeSized> LegacyReceiver for &mut T {}

// ---------------------------------------------------------------------------
// Marker traits
// ---------------------------------------------------------------------------

#[lang = "copy"]
pub trait Copy {}

#[lang = "bikeshed_guaranteed_no_drop"]
pub trait BikeshedGuaranteedNoDrop {}

impl Copy for bool {}
impl Copy for char {}
impl Copy for u8 {}
impl Copy for u16 {}
impl Copy for u32 {}
impl Copy for usize {}
impl Copy for i8 {}
impl Copy for i16 {}
impl Copy for i32 {}
impl Copy for i64 {}
impl Copy for isize {}
impl Copy for u64 {}
impl Copy for f32 {}
impl Copy for f64 {}
impl<'a, T: PointeeSized> Copy for &'a T {}
impl<T: PointeeSized> Copy for *const T {}
impl<T: PointeeSized> Copy for *mut T {}
impl<T: PointeeSized> Copy for PhantomData<T> {}

#[lang = "sync"]
pub unsafe trait Sync {}

unsafe impl Sync for bool {}
unsafe impl Sync for char {}
unsafe impl Sync for u8 {}
unsafe impl Sync for u16 {}
unsafe impl Sync for u32 {}
unsafe impl Sync for usize {}
unsafe impl Sync for i8 {}
unsafe impl Sync for i16 {}
unsafe impl Sync for i32 {}
unsafe impl Sync for i64 {}
unsafe impl Sync for isize {}
unsafe impl Sync for u64 {}
unsafe impl Sync for f32 {}
unsafe impl Sync for f64 {}
unsafe impl Sync for str {}
unsafe impl<'a, T: PointeeSized> Sync for &'a T {}
unsafe impl<T: Sync, const N: usize> Sync for [T; N] {}

#[lang = "freeze"]
pub unsafe auto trait Freeze {}

unsafe impl<T: PointeeSized> Freeze for PhantomData<T> {}
unsafe impl<T: PointeeSized> Freeze for *const T {}
unsafe impl<T: PointeeSized> Freeze for *mut T {}
unsafe impl<T: PointeeSized> Freeze for &T {}
unsafe impl<T: PointeeSized> Freeze for &mut T {}

#[lang = "unpin"]
pub auto trait Unpin {}

#[lang = "structural_peq"]
pub trait StructuralPartialEq {}

#[lang = "phantom_data"]
pub struct PhantomData<T: PointeeSized>;

#[lang = "manually_drop"]
#[repr(transparent)]
pub struct ManuallyDrop<T: ?Sized> {
    pub value: T,
}

// ---------------------------------------------------------------------------
// Unsizing coercions and pointer metadata
//
// `Unsize` is implemented by the compiler ([T; N] -> [T], Concrete -> dyn Trait);
// these items only give it somewhere to hang the coercion. Shapes copied verbatim
// from rustc_codegen_cranelift's example/mini_core.rs, minus everything about `Box`
// and raw-pointer unsizing (out of scope for the spike).
// ---------------------------------------------------------------------------

#[lang = "unsize"]
pub trait Unsize<T: PointeeSized>: PointeeSized {}

#[lang = "coerce_unsized"]
pub trait CoerceUnsized<T> {}

impl<'a, 'b: 'a, T: PointeeSized + Unsize<U>, U: PointeeSized> CoerceUnsized<&'a U> for &'b T {}
impl<'a, T: PointeeSized + Unsize<U>, U: PointeeSized> CoerceUnsized<&'a mut U> for &'a mut T {}

#[lang = "dispatch_from_dyn"]
pub trait DispatchFromDyn<T> {}

impl<'a, T: PointeeSized + Unsize<U>, U: PointeeSized> DispatchFromDyn<&'a U> for &'a T {}
impl<'a, T: PointeeSized + Unsize<U>, U: PointeeSized> DispatchFromDyn<&'a mut U> for &'a mut T {}

#[lang = "pointee_trait"]
pub trait Pointee: PointeeSized {
    #[lang = "metadata_type"]
    type Metadata: Copy + Sync + Unpin + Freeze;
}

// `<dyn Trait as Pointee>::Metadata`. The field layout is never observed by the
// tests -- it exists so that asking for the metadata type of a `dyn` pointee does
// not demand the `dyn_metadata` lang item that `pointee_trait` implies.
#[lang = "dyn_metadata"]
pub struct DynMetadata<Dyn: PointeeSized> {
    _vtable_ptr: *const (),
    _phantom: PhantomData<Dyn>,
}

impl<Dyn: PointeeSized> Copy for DynMetadata<Dyn> {}
unsafe impl<Dyn: PointeeSized> Sync for DynMetadata<Dyn> {}

pub mod intrinsics {
    use super::{Pointee, PointeeSized};

    #[rustc_intrinsic]
    pub const fn ptr_metadata<P: Pointee<Metadata = M> + PointeeSized, M>(ptr: *const P) -> M;

    #[rustc_intrinsic]
    pub const unsafe fn transmute<Src, Dst>(src: Src) -> Dst;
}

/// The length of a slice. `<[T]>::len` lives in `core`, so the tests reach for the
/// metadata of the fat pointer directly.
pub fn slice_len<T>(s: &[T]) -> usize {
    intrinsics::ptr_metadata(s as *const [T])
}

/// The UTF-8 byte length of a `&str`, which is its pointer metadata. `str::len` lives in
/// `core`, so this asks the same way [`slice_len`] does.
pub fn str_len(s: &str) -> usize {
    intrinsics::ptr_metadata(s as *const str)
}

// ---------------------------------------------------------------------------
// Indexing
//
// Indexing is resolved through this trait even for arrays and slices; MIR building
// then rewrites `Index<usize>` on a builtin indexable back into a place projection,
// which is why the impl bodies below are not infinitely recursive.
// ---------------------------------------------------------------------------

#[lang = "index"]
pub trait Index<Idx: ?Sized> {
    type Output: ?Sized;
    fn index(&self, index: Idx) -> &Self::Output;
}

impl<T, const N: usize> Index<usize> for [T; N] {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<T> Index<usize> for [T] {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        &self[index]
    }
}

#[lang = "index_mut"]
pub trait IndexMut<Idx: ?Sized>: Index<Idx> {
    fn index_mut(&mut self, index: Idx) -> &mut Self::Output;
}

impl<T, const N: usize> IndexMut<usize> for [T; N] {
    fn index_mut(&mut self, index: usize) -> &mut T {
        &mut self[index]
    }
}

impl<T> IndexMut<usize> for [T] {
    fn index_mut(&mut self, index: usize) -> &mut T {
        &mut self[index]
    }
}

// ---------------------------------------------------------------------------
// Drop
// ---------------------------------------------------------------------------

#[lang = "drop"]
pub trait Drop {
    fn drop(&mut self);
}

#[lang = "drop_glue"]
pub unsafe fn drop_glue<T: ?Sized>(_to_drop: &mut T) {
    // Replaced by the real drop glue by the compiler.
}

#[lang = "deref"]
pub trait Deref {
    type Target: ?Sized;

    fn deref(&self) -> &Self::Target;
}

// ---------------------------------------------------------------------------
// Arithmetic / bitwise / comparison operator traits
// ---------------------------------------------------------------------------

#[lang = "add"]
pub trait Add<RHS = Self> {
    type Output;
    fn add(self, rhs: RHS) -> Self::Output;
}

#[lang = "sub"]
pub trait Sub<RHS = Self> {
    type Output;
    fn sub(self, rhs: RHS) -> Self::Output;
}

#[lang = "mul"]
pub trait Mul<RHS = Self> {
    type Output;
    fn mul(self, rhs: RHS) -> Self::Output;
}

#[lang = "div"]
pub trait Div<RHS = Self> {
    type Output;
    fn div(self, rhs: RHS) -> Self::Output;
}

#[lang = "rem"]
pub trait Rem<RHS = Self> {
    type Output;
    fn rem(self, rhs: RHS) -> Self::Output;
}

#[lang = "neg"]
pub trait Neg {
    type Output;
    fn neg(self) -> Self::Output;
}

#[lang = "not"]
pub trait Not {
    type Output;
    fn not(self) -> Self::Output;
}

#[lang = "bitand"]
pub trait BitAnd<RHS = Self> {
    type Output;
    fn bitand(self, rhs: RHS) -> Self::Output;
}

#[lang = "bitor"]
pub trait BitOr<RHS = Self> {
    type Output;
    fn bitor(self, rhs: RHS) -> Self::Output;
}

#[lang = "bitxor"]
pub trait BitXor<RHS = Self> {
    type Output;
    fn bitxor(self, rhs: RHS) -> Self::Output;
}

#[lang = "shl"]
pub trait Shl<RHS = Self> {
    type Output;
    fn shl(self, rhs: RHS) -> Self::Output;
}

#[lang = "shr"]
pub trait Shr<RHS = Self> {
    type Output;
    fn shr(self, rhs: RHS) -> Self::Output;
}

#[lang = "add_assign"]
pub trait AddAssign<RHS = Self> {
    fn add_assign(&mut self, rhs: RHS);
}

#[lang = "sub_assign"]
pub trait SubAssign<RHS = Self> {
    fn sub_assign(&mut self, rhs: RHS);
}

#[lang = "mul_assign"]
pub trait MulAssign<RHS = Self> {
    fn mul_assign(&mut self, rhs: RHS);
}

#[lang = "div_assign"]
pub trait DivAssign<RHS = Self> {
    fn div_assign(&mut self, rhs: RHS);
}

#[lang = "rem_assign"]
pub trait RemAssign<RHS = Self> {
    fn rem_assign(&mut self, rhs: RHS);
}

#[lang = "eq"]
pub trait PartialEq<Rhs: ?Sized = Self> {
    fn eq(&self, other: &Rhs) -> bool;
    fn ne(&self, other: &Rhs) -> bool;
}

#[lang = "partial_ord"]
pub trait PartialOrd<Rhs: ?Sized = Self>: PartialEq<Rhs> {
    fn lt(&self, other: &Rhs) -> bool;
    fn le(&self, other: &Rhs) -> bool;
    fn gt(&self, other: &Rhs) -> bool;
    fn ge(&self, other: &Rhs) -> bool;
}

// The bodies below all operate on primitives, so rustc lowers them straight to
// primitive MIR operations instead of recursing back into these impls.

macro_rules! impl_arith {
    ($($t:ty),* $(,)?) => {
        $(
            impl Add for $t {
                type Output = $t;
                fn add(self, rhs: $t) -> $t { self + rhs }
            }
            impl Sub for $t {
                type Output = $t;
                fn sub(self, rhs: $t) -> $t { self - rhs }
            }
            impl Mul for $t {
                type Output = $t;
                fn mul(self, rhs: $t) -> $t { self * rhs }
            }
            impl Div for $t {
                type Output = $t;
                fn div(self, rhs: $t) -> $t { self / rhs }
            }
            impl Rem for $t {
                type Output = $t;
                fn rem(self, rhs: $t) -> $t { self % rhs }
            }
            impl AddAssign for $t {
                fn add_assign(&mut self, rhs: $t) { *self += rhs }
            }
            impl SubAssign for $t {
                fn sub_assign(&mut self, rhs: $t) { *self -= rhs }
            }
            impl MulAssign for $t {
                fn mul_assign(&mut self, rhs: $t) { *self *= rhs }
            }
            impl DivAssign for $t {
                fn div_assign(&mut self, rhs: $t) { *self /= rhs }
            }
            impl RemAssign for $t {
                fn rem_assign(&mut self, rhs: $t) { *self %= rhs }
            }
        )*
    };
}

macro_rules! impl_bitwise {
    ($($t:ty),* $(,)?) => {
        $(
            impl BitAnd for $t {
                type Output = $t;
                fn bitand(self, rhs: $t) -> $t { self & rhs }
            }
            impl BitOr for $t {
                type Output = $t;
                fn bitor(self, rhs: $t) -> $t { self | rhs }
            }
            impl BitXor for $t {
                type Output = $t;
                fn bitxor(self, rhs: $t) -> $t { self ^ rhs }
            }
            impl Not for $t {
                type Output = $t;
                fn not(self) -> $t { !self }
            }
        )*
    };
}

macro_rules! impl_shift {
    ($($t:ty),* $(,)?) => {
        $(
            impl Shl for $t {
                type Output = $t;
                fn shl(self, rhs: $t) -> $t { self << rhs }
            }
            impl Shr for $t {
                type Output = $t;
                fn shr(self, rhs: $t) -> $t { self >> rhs }
            }
        )*
    };
}

macro_rules! impl_neg {
    ($($t:ty),* $(,)?) => {
        $(
            impl Neg for $t {
                type Output = $t;
                fn neg(self) -> $t { -self }
            }
        )*
    };
}

macro_rules! impl_cmp {
    ($($t:ty),* $(,)?) => {
        $(
            impl PartialEq for $t {
                fn eq(&self, other: &$t) -> bool { (*self) == (*other) }
                fn ne(&self, other: &$t) -> bool { (*self) != (*other) }
            }
            impl PartialOrd for $t {
                fn lt(&self, other: &$t) -> bool { (*self) < (*other) }
                fn le(&self, other: &$t) -> bool { (*self) <= (*other) }
                fn gt(&self, other: &$t) -> bool { (*self) > (*other) }
                fn ge(&self, other: &$t) -> bool { (*self) >= (*other) }
            }
        )*
    };
}

// Shifting by a differently-typed integer is common enough (`x << n` with `n: u32`)
// that the wide types carry the extra impl.
macro_rules! impl_shift_by_u32 {
    ($($t:ty),* $(,)?) => {
        $(
            impl Shl<u32> for $t {
                type Output = $t;
                fn shl(self, rhs: u32) -> $t { self << rhs }
            }
            impl Shr<u32> for $t {
                type Output = $t;
                fn shr(self, rhs: u32) -> $t { self >> rhs }
            }
        )*
    };
}

impl_arith!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f32, f64);
impl_bitwise!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
impl_shift!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
impl_shift_by_u32!(i64, u64);
impl_neg!(i8, i16, i32, i64, isize, f32, f64);
impl_cmp!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f32, f64, char, bool);

impl BitAnd for bool {
    type Output = bool;
    fn bitand(self, rhs: bool) -> bool {
        self & rhs
    }
}

impl BitOr for bool {
    type Output = bool;
    fn bitor(self, rhs: bool) -> bool {
        self | rhs
    }
}

impl BitXor for bool {
    type Output = bool;
    fn bitxor(self, rhs: bool) -> bool {
        self ^ rhs
    }
}

impl Not for bool {
    type Output = bool;
    fn not(self) -> bool {
        !self
    }
}

// ---------------------------------------------------------------------------
// Callable traits
// ---------------------------------------------------------------------------

#[lang = "fn_once"]
#[rustc_paren_sugar]
pub trait FnOnce<Args: Tuple> {
    #[lang = "fn_once_output"]
    type Output;

    extern "rust-call" fn call_once(self, args: Args) -> Self::Output;
}

#[lang = "fn_mut"]
#[rustc_paren_sugar]
pub trait FnMut<Args: Tuple>: FnOnce<Args> {
    extern "rust-call" fn call_mut(&mut self, args: Args) -> Self::Output;
}

#[lang = "fn"]
#[rustc_paren_sugar]
pub trait Fn<Args: Tuple>: FnMut<Args> {
    extern "rust-call" fn call(&self, args: Args) -> Self::Output;
}

// ---------------------------------------------------------------------------
// Print / abort shim -- the only foreign functions in the spike
// ---------------------------------------------------------------------------

#[allow(improper_ctypes)]
extern "C" {
    fn js_log_i32(x: i32);
    fn js_log_i64(x: i64);
    fn js_log_f64(x: f64);
    fn js_log_bool(x: bool);
    fn js_log_str(ptr: &str); // backend passes the JS string value directly
    fn js_abort(msg: &str) -> !;
}

pub fn print_i32(x: i32) {
    unsafe { js_log_i32(x) }
}

pub fn print_i64(x: i64) {
    unsafe { js_log_i64(x) }
}

pub fn print_f64(x: f64) {
    unsafe { js_log_f64(x) }
}

pub fn print_bool(x: bool) {
    unsafe { js_log_bool(x) }
}

pub fn print_str(s: &str) {
    unsafe { js_log_str(s) }
}

// ---------------------------------------------------------------------------
// Panic lang items
// ---------------------------------------------------------------------------

#[lang = "panic_location"]
pub struct PanicLocation {
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
}

#[lang = "panic"]
#[track_caller]
pub fn panic(msg: &'static str) -> ! {
    unsafe { js_abort(msg) }
}

#[lang = "panic_nounwind"]
#[track_caller]
pub fn panic_nounwind(msg: &'static str) -> ! {
    unsafe { js_abort(msg) }
}

macro_rules! panic_const {
    ($($lang:ident = $message:expr,)+) => {
        pub mod panic_const {
            use super::*;

            $(
                #[track_caller]
                #[lang = stringify!($lang)]
                pub fn $lang() -> ! {
                    panic($message);
                }
            )+
        }
    }
}

panic_const! {
    panic_const_add_overflow = "attempt to add with overflow",
    panic_const_sub_overflow = "attempt to subtract with overflow",
    panic_const_mul_overflow = "attempt to multiply with overflow",
    panic_const_div_overflow = "attempt to divide with overflow",
    panic_const_rem_overflow = "attempt to calculate the remainder with overflow",
    panic_const_neg_overflow = "attempt to negate with overflow",
    panic_const_shr_overflow = "attempt to shift right with overflow",
    panic_const_shl_overflow = "attempt to shift left with overflow",
    panic_const_div_by_zero = "attempt to divide by zero",
    panic_const_rem_by_zero = "attempt to calculate the remainder with a divisor of zero",
}

#[lang = "panic_bounds_check"]
#[track_caller]
fn panic_bounds_check(_index: usize, _len: usize) -> ! {
    panic("index out of bounds");
}

#[lang = "panic_cannot_unwind"]
#[track_caller]
fn panic_cannot_unwind() -> ! {
    panic("panic in a function that cannot unwind");
}

#[lang = "panic_in_cleanup"]
fn panic_in_cleanup() -> ! {
    panic("panic in a destructor during cleanup");
}
