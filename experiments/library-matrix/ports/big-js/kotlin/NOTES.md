# big.js 7.0.1: Kotlin/JS port

A hand-written, faithful port of the subset in `libraries/big-js/CONTRACT.md` to Kotlin/JS 2.4.10
(the harness's Gradle production lane). It follows `node_modules/big.js/big.mjs` function by
function: `parse`, `round`, `stringify`, `cmp`, `div`, `minus`, `plus`, `pow`, `P.round`, `sqrt`,
`times`, `toExponential`, `toFixed`, `toPrecision` and `toString`, with upstream's loops, order of
checks and error messages. It replaces a port that a Python generator had emitted from a shared
core.

| File | What it is |
| --- | --- |
| `src/Big.kt` | the decimal type `Big` and upstream's functions |
| `src/Main.kt` | the boundary, doing what `libraries/big-js/reference.mjs` does; `run` is the only export (`@JsExport`) |

## Boundary

- **Input.** External interfaces give typed views of the input and its cases, with no copy.
- **Options.** `dp`, `rm` and `repeat` are `Double?`/`Int?`. Kotlin/JS null tests are loose, so
  JS `undefined` and `null` both take the default (20, 1, 1), as the adapter's `??` does.
- **Optional arguments.** `n` and `mode` stay nullable and reach the methods as upstream's
  optional arguments; an absent `mode` is `Big.RM`.
- **Operands.** `x` and `y` are strings, or numbers, which go through `toString()` (JS `String()`
  on Kotlin/JS), except that −0 becomes `"-0"`, as in big.js's constructor.
- **Output.** The results join with newlines; a `BigError` contributes its message.

## Representation and numeric semantics

- **Coefficient.** A `MutableList<Int>` (an `ArrayList` over a JS array), mutated where upstream
  mutates its array:
  - `xc.length = sd` is `subList(sd, size).clear()`;
  - `shift`/`pop`/`unshift` are `removeFirst`/`removeLast`/`add(0, …)`;
  - upstream's reverse-push-reverse prepending of zeros is `addAll(0, …)`.
- **Exponent and options.** The exponent is a `Double`, as upstream's is a JS number: past int32
  and 2^53 every `++`, `--` and sum rounds as in JS, and Infinity and NaN exponents keep upstream's
  comparison chains. `dp`, `rm`, `n` and `mode` stay `Double`s until upstream validates them:
  - `isIntIn` is `n !== ~~n || n < lo || n > hi`, negated, whose ranges lie inside int32;
  - a rounding mode is checked by equality with 0 to 3.

  So values past int32 or 2^53 fail rather than wrap or saturate.
- **JS-isms, spelt out:**
  - `if (a = xe - ye)` tests JS truthiness (`isTruthy`: neither 0 nor NaN);
  - `t.c.slice(0, e)` with a negative `e` is `head`;
  - `| 0` and `&` on exponents use `toInt32` (ECMAScript ToInt32).
- **Host number conversions:**
  - JS `Number()` is `toDoubleOrNull() ?: NaN` (Kotlin/JS's `+s` with a failure check), which is
    `Number()` for every string upstream passes it, NaN included;
  - `String()` is Kotlin/JS `Double.toString()` and string concatenation;
  - `toExponential()` is a dynamic call on the double (`mantissa`).
- **NUMERIC.** A Kotlin `Regex` (a JS `RegExp`) with upstream's pattern, `[eE]` written out
  because Kotlin/JS adds the `u` flag.

## Square root

`sqrt` runs upstream's iteration exactly:

- the host's binary64 estimate, with its underflow/overflow re-estimate and ToInt32 exponent;
- `Big.DP += 4`, passed to `div`, which validates it, so `dp` ≤ −5 or a non-integer `dp` throws
  `Invalid decimal places`;
- the stop test on the first `r.e + DP + 4` digits of successive estimates.

There is no shortcut for small operands. Where upstream's loop never stops (CONTRACT.md: mostly
tiny operands near the last requested place), this port does not stop either. Those inputs are
outside the domain, because the reference gives no answer to compare against. The choice keeps the
port's results and termination identical to upstream's wherever upstream answers.

## Deviations from upstream

- **Remainder.** In `div`, upstream's remainder `[a[ai]]` past the dividend's end is
  `[undefined]`; here it is the empty list, and `r[0] !== undefined` is `isNotEmpty()`.
- **`k--`.** In `div`, `k--` is `k-- > 0`. Both are false exactly at 0 and NaN.
- **Copies.** Each binary method copies its argument where upstream's `new Big(y)` does, and
  `times` builds a fresh result instead of reusing `y`.
- **Out of domain.** JSON `null` for `n` or `mode` is outside the domain (CONTRACT.md); this port
  treats it as absent.

## Performance accommodations

None.
