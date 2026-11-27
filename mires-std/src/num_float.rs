//! Standard library floating-point math methods on `f32` and `f64`, backed by
//! `core::intrinsics` which `mires` lowers directly to JS `Math.*`.

use core_crate::intrinsics;

impl f32 {
    /// Returns the largest integer less than or equal to `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn floor(self) -> f32 {
        intrinsics::floorf32(self)
    }

    /// Returns the smallest integer greater than or equal to `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn ceil(self) -> f32 {
        intrinsics::ceilf32(self)
    }

    /// Returns the nearest integer to `self`, rounding half-way cases away from `0.0`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn round(self) -> f32 {
        intrinsics::roundf32(self)
    }

    /// Returns the nearest integer to `self`, rounding ties to even.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn round_ties_even(self) -> f32 {
        intrinsics::round_ties_even_f32(self)
    }

    /// Returns the integer part of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn trunc(self) -> f32 {
        intrinsics::truncf32(self)
    }

    /// Returns the fractional part of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn fract(self) -> f32 {
        self - self.trunc()
    }

    /// Fused multiply-add: computes `(self * a) + b`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn mul_add(self, a: f32, b: f32) -> f32 {
        intrinsics::fmaf32(self, a, b)
    }

    /// Calculates Euclidean division.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn div_euclid(self, rhs: f32) -> f32 {
        let q = (self / rhs).trunc();
        if self % rhs < 0.0 {
            return if rhs > 0.0 { q - 1.0 } else { q + 1.0 };
        }
        q
    }

    /// Calculates the least nonnegative remainder of `self (mod rhs)`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn rem_euclid(self, rhs: f32) -> f32 {
        let r = self % rhs;
        if r < 0.0 { r + rhs.abs() } else { r }
    }

    /// Raises `self` to an integer power.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn powi(self, n: i32) -> f32 {
        intrinsics::powif32(self, n)
    }

    /// Raises `self` to a floating-point power.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn powf(self, n: f32) -> f32 {
        intrinsics::powf32(self, n)
    }

    /// Returns the square root of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn sqrt(self) -> f32 {
        if self < 0.0 {
            f32::NAN
        } else {
            intrinsics::sqrtf32(self)
        }
    }

    /// Returns `e^(self)`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn exp(self) -> f32 {
        intrinsics::expf32(self)
    }

    /// Returns `2^(self)`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn exp2(self) -> f32 {
        intrinsics::exp2f32(self)
    }

    /// Returns the natural logarithm of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn ln(self) -> f32 {
        intrinsics::logf32(self)
    }

    /// Returns the logarithm of `self` with respect to an arbitrary base.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn log(self, base: f32) -> f32 {
        self.ln() / base.ln()
    }

    /// Returns the base-2 logarithm of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn log2(self) -> f32 {
        intrinsics::log2f32(self)
    }

    /// Returns the base-10 logarithm of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn log10(self) -> f32 {
        intrinsics::log10f32(self)
    }

    /// Computes the sine of `self` (in radians).
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn sin(self) -> f32 {
        intrinsics::sinf32(self)
    }

    /// Computes the cosine of `self` (in radians).
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn cos(self) -> f32 {
        intrinsics::cosf32(self)
    }

    /// Simultaneously computes the sine and cosine of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn sin_cos(self) -> (f32, f32) {
        (self.sin(), self.cos())
    }

    /// Computes the tangent of `self` (in radians).
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn tan(self) -> f32 {
        self.sin() / self.cos()
    }

    /// Calculates the Euclidean distance `sqrt(self^2 + other^2)`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn hypot(self, other: f32) -> f32 {
        (self as f64).hypot(other as f64) as f32
    }
}

impl f64 {
    /// Returns the largest integer less than or equal to `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn floor(self) -> f64 {
        intrinsics::floorf64(self)
    }

    /// Returns the smallest integer greater than or equal to `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn ceil(self) -> f64 {
        intrinsics::ceilf64(self)
    }

    /// Returns the nearest integer to `self`, rounding half-way cases away from `0.0`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn round(self) -> f64 {
        intrinsics::roundf64(self)
    }

    /// Returns the nearest integer to `self`, rounding ties to even.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn round_ties_even(self) -> f64 {
        intrinsics::round_ties_even_f64(self)
    }

    /// Returns the integer part of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn trunc(self) -> f64 {
        intrinsics::truncf64(self)
    }

    /// Returns the fractional part of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn fract(self) -> f64 {
        self - self.trunc()
    }

    /// Fused multiply-add: computes `(self * a) + b`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn mul_add(self, a: f64, b: f64) -> f64 {
        intrinsics::fmaf64(self, a, b)
    }

    /// Calculates Euclidean division.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn div_euclid(self, rhs: f64) -> f64 {
        let q = (self / rhs).trunc();
        if self % rhs < 0.0 {
            return if rhs > 0.0 { q - 1.0 } else { q + 1.0 };
        }
        q
    }

    /// Calculates the least nonnegative remainder of `self (mod rhs)`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn rem_euclid(self, rhs: f64) -> f64 {
        let r = self % rhs;
        if r < 0.0 { r + rhs.abs() } else { r }
    }

    /// Raises `self` to an integer power.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn powi(self, n: i32) -> f64 {
        intrinsics::powif64(self, n)
    }

    /// Raises `self` to a floating-point power.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn powf(self, n: f64) -> f64 {
        intrinsics::powf64(self, n)
    }

    /// Returns the square root of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn sqrt(self) -> f64 {
        if self < 0.0 {
            f64::NAN
        } else {
            intrinsics::sqrtf64(self)
        }
    }

    /// Returns `e^(self)`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn exp(self) -> f64 {
        intrinsics::expf64(self)
    }

    /// Returns `2^(self)`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn exp2(self) -> f64 {
        intrinsics::exp2f64(self)
    }

    /// Returns the natural logarithm of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn ln(self) -> f64 {
        intrinsics::logf64(self)
    }

    /// Returns the logarithm of `self` with respect to an arbitrary base.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn log(self, base: f64) -> f64 {
        self.ln() / base.ln()
    }

    /// Returns the base-2 logarithm of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn log2(self) -> f64 {
        intrinsics::log2f64(self)
    }

    /// Returns the base-10 logarithm of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn log10(self) -> f64 {
        intrinsics::log10f64(self)
    }

    /// Computes the sine of `self` (in radians).
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn sin(self) -> f64 {
        intrinsics::sinf64(self)
    }

    /// Computes the cosine of `self` (in radians).
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn cos(self) -> f64 {
        intrinsics::cosf64(self)
    }

    /// Simultaneously computes the sine and cosine of `self`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn sin_cos(self) -> (f64, f64) {
        (self.sin(), self.cos())
    }

    /// Computes the tangent of `self` (in radians).
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn tan(self) -> f64 {
        self.sin() / self.cos()
    }

    /// Calculates the Euclidean distance `sqrt(self^2 + other^2)`.
    #[rustc_allow_incoherent_impl]
    #[must_use]
    #[inline]
    pub fn hypot(self, other: f64) -> f64 {
        let x = self.abs();
        let y = other.abs();
        if x.is_infinite() || y.is_infinite() {
            return f64::INFINITY;
        }
        if x.is_nan() || y.is_nan() {
            return f64::NAN;
        }
        let (hi, lo) = if x >= y { (x, y) } else { (y, x) };
        if hi == 0.0 {
            return 0.0;
        }
        let r = lo / hi;
        hi * (1.0 + r * r).sqrt()
    }
}
