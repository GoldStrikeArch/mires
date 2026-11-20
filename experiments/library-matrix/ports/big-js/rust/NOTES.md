# big.js 7.0.1: Rust port (mires)

The Rust port stores normalized decimal digits in `Vec<u8>`, aligns coefficients for addition,
uses schoolbook multiplication and decimal long division, and iterates the square root by Newton's
method.

**MIRES-019.** Parsing the leading sign with a direct byte comparison avoids MIRES-019, where a
promoted `Some(&b'-')` compares incorrectly in generated JavaScript.

| File | What it is |
| --- | --- |
| `src/decimal.rs` | `Decimal` (sign, `f64` exponent, digits) and the arithmetic, rounding and formatting |
| `src/lib.rs` | the boundary (what `libraries/big-js/reference.mjs` does) and the host bindings |

## Boundary

- **Input.** Read with `Reflect.get`.
- **Options.** `number_field` reads a number option as `Option<f64>`, `None` when it is
  `undefined` or `null`. So `dp`, `rm` and `repeat` take the defaults 20, 1 and 1, as the
  adapter's `??` does (review b8 found that a null threw).
- **Optional arguments.** `n` and `mode` stay options and reach the methods as upstream's optional
  arguments.
- **Operands.** `x` and `y` go through the host's `String()`, except that a number −0 becomes
  `"-0"`, as in big.js's constructor.
- **Options in the methods.** `dp`, `rm`, `n` and `mode` stay `f64` until upstream would validate
  them:
  - `is_int_in` is `n !== ~~n || n < lo || n > hi`, negated;
  - a rounding mode is checked by equality with 0 to 3.

  So non-integers and values past int32 or 2^53 fail where upstream's checks fail, without the
  former `i32::MIN` stand-in.

## Square root

`sqrt` runs upstream's iteration exactly (review b8):

- the host's binary64 estimate (`Math.sqrt(Number(...))`), with the underflow/overflow re-estimate
  and upstream's ToInt32 exponent `((e + 1) / 2 | 0) - (e < 0 || e & 1)`;
- `Big.DP += 4`, passed to `div`, which validates it, so `dp` ≤ −5 or a non-integer `dp` throws
  `Invalid decimal places`;
- the stop test on the first `r.e + DP + 4` digits of successive estimates.

The former shortcut for tiny operands, the working precision `max(dp + 4, 5 - r.e)` and the
exponent-equality stop test are gone. Where upstream's loop never stops (CONTRACT.md: mostly tiny
operands near the last requested place), this port does not stop either. Those inputs are outside
the domain, because the reference gives no answer to compare against. The choice keeps results and
termination identical to upstream's wherever upstream answers.

**The estimate's digits.** Upstream parses the digits of `toExponential()` before its `e`, followed
by the computed exponent. This port parses `String()` of the estimate and sets the exponent. Both
print the shortest round-trip digits of the same double.

## Deviations from upstream

- **Division.** The decimal long division is restructured: it drops the leading zero eagerly and
  stops once it has one digit past the precision. On the corpus and on the review's 6,000 random
  cases it produces upstream's digits, termination and rounding. Upstream's NaN precision (an
  Infinity − Infinity quotient exponent) stops after one digit, keeps it even when it is zero, and
  never rounds; the port does the same.
- **Addition.** Addition aligns both coefficients at the larger exponent. The leading zeros of a
  difference decrement the exponent one at a time, as upstream's `--ye` does: past 2^53 a run of
  decrements is not one subtraction.
- **Rounding.** `rounded` returns the value unchanged when `sd` is NaN, as upstream's `sd < 1` and
  `sd < xc.length` both fail.
- **Out of domain.** JSON `null` for `n` or `mode` is outside the domain (CONTRACT.md).

## Performance accommodations

None.
