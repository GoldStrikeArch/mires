// Decimal coefficient arithmetic from big.js 7.0.1. Digits are most significant first.
use alloc::{string::String, vec, vec::Vec};

pub const NUMBER: &str = "[big.js] Invalid number";
pub const PLACES: &str = "[big.js] Invalid decimal places";
pub const MODE: &str = "[big.js] Invalid rounding mode";
pub const PRECISION: &str = "[big.js] Invalid precision";
pub const POWER: &str = "[big.js] Invalid exponent";
pub const ZERO_DIV: &str = "[big.js] Division by zero";
pub const NO_ROOT: &str = "[big.js] No square root";

// Upstream's `n !== ~~n || n < lo || n > hi`, negated: an integer in the range (all ranges lie
// within int32, so the cast is exact once the range holds).
fn is_int_in(n: f64, lo: f64, hi: f64) -> bool {
    n >= lo && n <= hi && n == (n as i32) as f64
}

// ECMAScript ToInt32, for upstream's `| 0` and `&` on exponents: `%` keeps the sign and the
// fraction, the cast truncates (NaN and infinities become 0), and i64 to i32 keeps the low bits.
fn to_int32(v: f64) -> i32 {
    (v % 4294967296.0) as i64 as i32
}

// `Array.prototype.slice(0, end)`, where a negative end counts from the back.
fn head(digits: &[u8], end: f64) -> &[u8] {
    let n = digits.len() as f64;
    let end = if end < 0.0 { (n + end).max(0.0) } else { end.min(n) };
    &digits[..end as usize]
}

#[derive(Clone)]
pub struct Decimal {
    pub sign: i32,
    pub exponent: f64,
    pub digits: Vec<u8>,
}

impl Decimal {
    fn single(sign: i32, exponent: f64, digit: u8) -> Self {
        Self {
            sign,
            exponent,
            digits: vec![digit],
        }
    }
    fn normalize(sign: i32, exponent: f64, digits: Vec<u8>, offset: usize) -> Self {
        let mut start = offset;
        while start < digits.len() && digits[start] == 0 {
            start += 1;
        }
        if start == digits.len() {
            return Self::single(sign, 0.0, 0);
        }
        let mut end = digits.len();
        while digits[end - 1] == 0 {
            end -= 1;
        }
        // One decrement per leading zero, as upstream's `--ye`: past 2^53 a run of decrements
        // rounds differently from one subtraction.
        let mut exponent = exponent;
        for _ in offset..start {
            exponent -= 1.0;
        }
        Self {
            sign,
            exponent,
            digits: digits[start..end].to_vec(),
        }
    }
    pub fn parse(s: &str) -> Result<Self, &'static str> {
        let bytes = s.as_bytes();
        let mut i = 0;
        // WORKAROUND(MIRES-019): compare the byte without a promoted Option<&u8>.
        let sign = if !bytes.is_empty() && bytes[0] == b'-' {
            i += 1;
            -1
        } else {
            1
        };
        let mut digits = Vec::new();
        let mut places = 0;
        let mut scale = 0.0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            digits.push(bytes[i] - b'0');
            places += 1;
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                digits.push(bytes[i] - b'0');
                i += 1;
            }
        }
        if digits.is_empty() {
            return Err(NUMBER);
        }
        if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            i += 1;
            let exponent_start = i;
            if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                i += 1;
            }
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == start {
                return Err(NUMBER);
            }
            scale = crate::number(mires_js::JsValue::erase(&s[exponent_start..i]));
        }
        if i != bytes.len() {
            return Err(NUMBER);
        }
        let start = digits
            .iter()
            .position(|&digit| digit != 0)
            .unwrap_or(digits.len());
        Ok(Self::normalize(
            sign,
            places as f64 + scale - start as f64 - 1.0,
            digits,
            start,
        ))
    }
    fn zero(&self) -> bool {
        self.digits[0] == 0
    }
    pub fn cmp(&self, y: &Self) -> i32 {
        if self.zero() {
            return if y.zero() { 0 } else { -y.sign };
        }
        if y.zero() || self.sign != y.sign {
            return self.sign;
        }
        let c = if self.exponent != y.exponent {
            if self.exponent > y.exponent {
                1
            } else {
                -1
            }
        } else {
            let mut i = 0;
            while i < self.digits.len() && i < y.digits.len() && self.digits[i] == y.digits[i] {
                i += 1;
            }
            if i == self.digits.len() || i == y.digits.len() {
                (self.digits.len() as i32 - y.digits.len() as i32).signum()
            } else {
                (self.digits[i] as i32 - y.digits[i] as i32).signum()
            }
        };
        c * self.sign
    }
    fn coefficient(&self, index: usize, offset: usize) -> i32 {
        if index >= offset && index - offset < self.digits.len() {
            self.digits[index - offset] as i32
        } else {
            0
        }
    }
    pub fn add(&self, y: &Self, subtract: bool) -> Self {
        let mut other = y.clone();
        if subtract {
            other.sign = -other.sign;
        }
        if self.zero() {
            if other.zero() && self.sign != other.sign {
                other.sign = 1;
            }
            return other;
        }
        if other.zero() {
            return self.clone();
        }
        let mut sign = self.sign;
        let same = sign == other.sign;
        other.sign = sign;
        let reverse = !same && self.cmp(&other) * sign < 0;
        if reverse {
            sign = -sign;
        }
        let top = self.exponent.max(other.exponent);
        let x_offset = (top - self.exponent) as usize;
        let y_offset = (top - other.exponent) as usize;
        let width = (x_offset + self.digits.len()).max(y_offset + other.digits.len());
        let mut digits = vec![0; width + 1];
        let mut carry = 0;
        for p in (0..width).rev() {
            let a = self.coefficient(p, x_offset);
            let b = other.coefficient(p, y_offset);
            let sum = carry
                + if same {
                    a + b
                } else if reverse {
                    b - a
                } else {
                    a - b
                };
            carry = if sum < 0 { -1 } else { sum / 10 };
            digits[p + 1] = (sum - carry * 10) as u8;
        }
        digits[0] = carry as u8;
        let mut result = if carry != 0 {
            Self::normalize(sign, top + 1.0, digits, 0)
        } else {
            Self::normalize(sign, top, digits, 1)
        };
        if result.zero() {
            result.sign = 1;
        }
        result
    }
    pub fn times(&self, y: &Self) -> Self {
        let sign = self.sign * y.sign;
        if self.zero() || y.zero() {
            return Self::single(sign, 0.0, 0);
        }
        let mut product = vec![0; self.digits.len() + y.digits.len()];
        for i in (0..self.digits.len()).rev() {
            let mut carry = 0;
            for j in (0..y.digits.len()).rev() {
                let k = i + j + 1;
                let v = self.digits[i] * y.digits[j] + product[k] + carry;
                product[k] = v % 10;
                carry = v / 10;
            }
            product[i] = carry;
        }
        let exponent = self.exponent + y.exponent;
        if product[0] == 0 {
            Self::normalize(sign, exponent, product, 1)
        } else {
            Self::normalize(sign, exponent + 1.0, product, 0)
        }
    }
    fn rounded(&self, sd: f64, mode: f64, more: bool) -> Result<Self, &'static str> {
        if mode != 0.0 && mode != 1.0 && mode != 2.0 && mode != 3.0 {
            return Err(MODE);
        }
        let mode = mode as i32;
        // Upstream rounds only when sd < 1 or sd < xc.length; a NaN sd fails both.
        if !(sd < self.digits.len() as f64) {
            return Ok(self.clone());
        }
        let guard = if sd >= 0.0 {
            self.digits[sd as usize]
        } else {
            0
        };
        let odd = sd > 0.0 && self.digits[(sd - 1.0) as usize] % 2 == 1;
        let sticky = sd + 1.0 < self.digits.len() as f64 || more;
        let up = match mode {
            1 => guard >= 5,
            2 => guard > 5 || guard == 5 && (odd || sticky),
            3 => !self.zero() || more,
            _ => false,
        };
        if sd <= 0.0 {
            return Ok(if up {
                Self::single(self.sign, self.exponent - sd + 1.0, 1)
            } else {
                Self::single(self.sign, 0.0, 0)
            });
        }
        let mut digits = self.digits[..sd as usize].to_vec();
        if up {
            let mut i = digits.len();
            loop {
                if i == 0 {
                    return Ok(Self::single(self.sign, self.exponent + 1.0, 1));
                }
                i -= 1;
                if digits[i] != 9 {
                    digits[i] += 1;
                    break;
                }
                digits[i] = 0;
            }
        }
        Ok(Self::normalize(self.sign, self.exponent, digits, 0))
    }
    pub fn div(&self, y: &Self, dp: f64, mode: f64) -> Result<Self, &'static str> {
        if !is_int_in(dp, 0.0, 1_000_000.0) {
            return Err(PLACES);
        }
        if y.zero() {
            return Err(ZERO_DIV);
        }
        let sign = self.sign * y.sign;
        if self.zero() {
            return Ok(Self::single(sign, 0.0, 0));
        }
        let divisor = &y.digits;
        let mut rest = Vec::new();
        let mut pos = 0;
        while pos < divisor.len() {
            rest.push(if pos < self.digits.len() {
                self.digits[pos]
            } else {
                0
            });
            pos += 1;
        }
        let mut exponent = self.exponent - y.exponent;
        // Upstream's precision dp + q.e + 1. When it is NaN (an Infinity - Infinity exponent),
        // upstream's `k--` stops after the first digit, which it keeps even when it is zero, and
        // `qi > p` never rounds.
        let precision = dp + exponent + 1.0;
        let mut digits = Vec::new();
        loop {
            let mut q = 0;
            while natural_cmp(&rest, divisor) >= 0 {
                let shift = rest.len() - divisor.len();
                let mut borrow = 0;
                for k in (0..rest.len()).rev() {
                    let d = if k >= shift {
                        divisor[k - shift] as i32
                    } else {
                        0
                    };
                    let v = rest[k] as i32 - d - borrow;
                    borrow = if v < 0 { 1 } else { 0 };
                    rest[k] = (v + borrow * 10) as u8;
                }
                let mut start = 0;
                while start < rest.len() && rest[start] == 0 {
                    start += 1;
                }
                if start > 0 {
                    rest = rest[start..].to_vec();
                }
                q += 1;
            }
            if q == 0 && digits.is_empty() {
                exponent -= 1.0;
            } else {
                digits.push(q);
            }
            if precision.is_nan() {
                if digits.is_empty() {
                    digits.push(0);
                }
                return Ok(Self {
                    sign,
                    exponent,
                    digits,
                });
            }
            let more = !rest.is_empty() || pos < self.digits.len();
            if !digits.is_empty() && (!more || digits.len() as f64 > dp + exponent + 1.0) {
                let sd = dp + exponent + 1.0;
                let value = Self {
                    sign,
                    exponent,
                    digits,
                };
                if value.digits.len() as f64 > sd {
                    return value.rounded(sd, mode, more);
                }
                return Ok(value);
            }
            let next = if pos < self.digits.len() {
                self.digits[pos]
            } else {
                0
            };
            if !rest.is_empty() || next != 0 {
                rest.push(next);
            }
            pos += 1;
        }
    }
    pub fn pow(&self, n: Option<f64>, dp: f64, mode: f64) -> Result<Self, &'static str> {
        let n = match n {
            Some(n) if is_int_in(n, -1_000_000.0, 1_000_000.0) => n as i32,
            _ => return Err(POWER),
        };
        let one = Self::single(1, 0.0, 1);
        let mut result = one.clone();
        let mut base = self.clone();
        let mut i = n.abs();
        while i > 0 {
            if i % 2 == 1 {
                result = result.times(&base);
            }
            i /= 2;
            if i > 0 {
                base = base.times(&base);
            }
        }
        if n < 0 {
            one.div(&result, dp, mode)
        } else {
            Ok(result)
        }
    }
    pub fn sqrt(&self, dp: f64, mode: f64) -> Result<Self, &'static str> {
        if self.zero() {
            return Ok(self.clone());
        }
        if self.sign < 0 {
            return Err(NO_ROOT);
        }
        // Upstream's estimate: the host's binary64 square root of the exponential string.
        let mut estimate = crate::root_estimate(&self.format("toExponential", None, mode)?);
        let mut root = if estimate == 0.0 || estimate == f64::INFINITY {
            // Math.sqrt underflow/overflow? Re-estimate from the coefficient as an integer, then
            // set upstream's exponent ((e + 1) / 2 | 0) - (e < 0 || e & 1).
            let e = self.exponent;
            let mut coefficient = String::new();
            for d in &self.digits {
                coefficient.push((b'0' + *d) as char);
            }
            if to_int32(coefficient.len() as f64 + e) & 1 == 0 {
                coefficient.push('0');
            }
            estimate = crate::root_estimate(&coefficient);
            let exponent = to_int32((e + 1.0) / 2.0) - i32::from(e < 0.0 || to_int32(e) & 1 != 0);
            // Upstream parses the digits of toExponential() before its `e`: the shortest digits
            // of the estimate, the ones String() prints, placed at this exponent.
            let mut r = if estimate == f64::INFINITY {
                Self::single(1, 0.0, 5)
            } else {
                Self::parse(crate::number_text(estimate))?
            };
            r.exponent = f64::from(exponent);
            r
        } else {
            // A NaN estimate prints as "NaN", which upstream rejects as an invalid number.
            Self::parse(crate::number_text(estimate))?
        };
        // Big.DP += 4 for the iteration (div validates it), and the stop test compares the first
        // r.e + DP + 4 digits of successive estimates, as joined strings.
        let half = Self::single(1, -1.0, 5);
        let places = dp + 4.0;
        let compared = root.exponent + places;
        loop {
            let next = half.times(&root.add(&self.div(&root, places, mode)?, false));
            let same = head(&root.digits, compared) == head(&next.digits, compared);
            root = next;
            if same {
                break;
            }
        }
        root.rounded(dp + root.exponent + 1.0, mode, false)
    }
    pub fn round(&self, n: Option<f64>, mode: f64) -> Result<Self, &'static str> {
        let n = n.unwrap_or(0.0);
        if !is_int_in(n, -1_000_000.0, 1_000_000.0) {
            return Err(PLACES);
        }
        self.rounded(n + self.exponent + 1.0, mode, false)
    }
    pub fn format(&self, op: &str, n: Option<f64>, mode: f64) -> Result<String, &'static str> {
        let mut value = self.clone();
        if let Some(n) = n {
            let sd = if op == "toPrecision" {
                if !is_int_in(n, 1.0, 1_000_000.0) {
                    return Err(PRECISION);
                }
                n
            } else {
                if !is_int_in(n, 0.0, 1_000_000.0) {
                    return Err(PLACES);
                }
                if op == "toFixed" {
                    n + value.exponent + 1.0
                } else {
                    n + 1.0
                }
            };
            value = value.rounded(sd, mode, false)?;
            let width = if op == "toFixed" {
                n + value.exponent + 1.0
            } else {
                sd
            };
            while (value.digits.len() as f64) < width {
                value.digits.push(0);
            }
        }
        let e = value.exponent;
        let exponential = op == "toExponential"
            || op != "toFixed"
                && (e <= -7.0
                    || e >= 21.0
                    || op == "toPrecision" && n.is_some_and(|n| n <= e));
        let mut s = String::new();
        if self.sign < 0 && !self.zero() {
            s.push('-');
        }
        if exponential {
            s.push((b'0' + value.digits[0]) as char);
            if value.digits.len() > 1 {
                s.push('.');
                for d in &value.digits[1..] {
                    s.push((b'0' + *d) as char);
                }
            }
            s.push('e');
            if e >= 0.0 {
                s.push('+');
            }
            s.push_str(crate::number_text(e));
        } else {
            let point = e as i32;
            let end = (point + 1).max(1) as usize;
            let offset = (-point).max(0) as usize;
            for i in 0..end {
                s.push((b'0' + value.coefficient(i, offset) as u8) as char);
            }
            if end < offset + value.digits.len() {
                s.push('.');
                for i in end..offset + value.digits.len() {
                    s.push((b'0' + value.coefficient(i, offset) as u8) as char);
                }
            }
        }
        Ok(s)
    }
}

fn natural_cmp(x: &[u8], y: &[u8]) -> i32 {
    if x.len() != y.len() {
        return if x.len() > y.len() { 1 } else { -1 };
    }
    for i in 0..x.len() {
        if x[i] != y[i] {
            return if x[i] > y[i] { 1 } else { -1 };
        }
    }
    0
}
