# big.js 7.0.1: Dart port

A hand-written, faithful port of the subset in `libraries/big-js/CONTRACT.md` to Dart 3, compiled
by dart2js (`-O2`, the harness's setting). It follows `node_modules/big.js/big.mjs` function by
function: `parse`, `round`, `stringify`, `cmp`, `div`, `minus`, `plus`, `pow`, `P.round`, `sqrt`,
`times`, `toExponential`, `toFixed`, `toPrecision` and `toString`, with upstream's loops, order of
checks and error messages. It replaces a port that a Python generator had emitted from a shared
core.

| File | What it is |
| --- | --- |
| `src/big.dart` | the decimal type `Big` and upstream's functions |
| `src/main.dart` | the boundary, doing what `libraries/big-js/reference.mjs` does; `main()` publishes `run` for the harness facade |

## Boundary

- **Input.** `dart:js_interop` extension types view the input and its cases.
- **Options.** `dp`, `rm` and `repeat` are nullable (`double?`, `int?`); JS `undefined` and `null`
  are both Dart `null`, so `??` gives the defaults 20, 1 and 1, as the adapter does.
- **Optional arguments.** `n` and `mode` stay nullable and reach the methods as upstream's
  optional arguments; an absent `mode` is `Big.RM`.
- **Operands.** `x` and `y` go through the host's `String()` (`@JS('String')`), except that a
  number −0 becomes `'-0'`, as in big.js's constructor.
- **Output.** The results join with newlines; a `BigError` contributes its message.

## Representation and numeric semantics

- **Coefficient.** A growable `List<int>`, mutated where upstream mutates its array:
  - `xc.length = sd` is the same assignment;
  - `shift`/`pop`/`unshift` are `removeAt(0)`/`removeLast()`/`insert(0, …)`;
  - upstream's reverse-push-reverse prepending of zeros is `insertAll(0, …)`.
- **Exponent and options.** The exponent is a `double`, as upstream's is a JS number: past int32
  and 2^53 every `++`, `--` and sum rounds as in JS, and Infinity and NaN exponents keep upstream's
  comparison chains. The exponent's digits are one `double.parse`, a single correctly rounded parse
  as upstream's `+n.slice(i + 1)` is. This is review finding b8 (`decimal.dart:90`): the generated
  port summed the digits in a double.

  `dp`, `rm`, `n` and `mode` stay `double`s until upstream validates them:
  - `_isIntIn` is `n !== ~~n || n < lo || n > hi`, negated;
  - a rounding mode is checked by equality with 0 to 3.

  So values past int32 or 2^53 fail rather than wrap.
- **JS-isms, spelt out:**
  - `if (a = xe - ye)` tests JS truthiness (`_truthy`);
  - `t.c.slice(0, e)` with a negative `e` is `_head`;
  - `| 0` and `&` on exponents use `_toInt32`, whose `%` is Dart's Euclidean modulo, the spec's
    own.
- **Host number conversions:**
  - JS `Number()` is `double.tryParse(s) ?? double.nan`, which is `Number()` for every string
    upstream passes it, NaN included;
  - `String()` is dart2js `double.toString()` and interpolation (`'' + n`, except for −0, which no
    exponent or estimate is);
  - `toExponential()` is `toStringAsExponential()`.
- **NUMERIC.** A Dart `RegExp` (a JS `RegExp`) with upstream's pattern and `i` flag.

## Square root

`sqrt` runs upstream's iteration exactly:

- the host's binary64 estimate, with its underflow/overflow re-estimate. The exponent is ToInt32 of
  `(e + 1) / 2`, then a subtraction in doubles, as upstream's `((e + 1) / 2 | 0) - (e < 0 || e & 1)`
  is, so it reaches −2^31 − 1 at x.e = −4294967297 and −4294967298 (re-review re-big-js);
- `Big.DP += 4`, passed to `div`, which validates it, so `dp` ≤ −5 or a non-integer `dp` throws
  `Invalid decimal places`;
- the stop test on the first `r.e + DP + 4` digits of successive estimates.

There is no shortcut for small operands. Where upstream's loop never stops (CONTRACT.md: mostly
tiny operands near the last requested place), this port does not stop either. Those inputs are
outside the domain, because the reference gives no answer to compare against. The choice keeps the
port's results and termination identical to upstream's wherever upstream answers.

## Deviations from upstream

- **Remainder.** In `div`, upstream's remainder `[a[ai]]` past the dividend's end is
  `[undefined]`; here it is the empty list, and `r[0] !== undefined` is `isNotEmpty`.
- **`k--`.** In `div`, `k--` is `k-- > 0`. Both are false exactly at 0 and NaN.
- **Copies.** Each binary method copies its argument where upstream's `new Big(y)` does, and
  `times` builds a fresh result instead of reusing `y`.
- **Out of domain.** JSON `null` for `n` or `mode` is outside the domain (CONTRACT.md); this port
  treats it as absent.

## Performance accommodations

None.
