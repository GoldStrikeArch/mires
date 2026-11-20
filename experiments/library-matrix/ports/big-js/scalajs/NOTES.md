# big.js 7.0.1: Scala.js port

A hand-written, faithful port of the subset in `libraries/big-js/CONTRACT.md` to Scala 3.8.4 on
Scala.js 1.22.0 (`fullLinkJS`, ES modules). It follows `node_modules/big.js/big.mjs` function by
function: `parse`, `round`, `stringify`, `cmp`, `div`, `minus`, `plus`, `pow`, `P.round`, `sqrt`,
`times`, `toExponential`, `toFixed`, `toPrecision` and `toString`, with upstream's loops, order of
checks and error messages. It replaces a port that a Python generator had emitted from a shared
core.

| File | What it is |
| --- | --- |
| `src/Big.scala` | the decimal type `Big`, its companion's `parse`, and upstream's package-level `round` and `stringify` |
| `src/Main.scala` | the boundary, doing what `libraries/big-js/reference.mjs` does; `run` is exported with `@JSExportTopLevel` |

## Boundary

- **Input.** `@js.native` facade traits view the input and its cases.
- **Options.** `dp`, `rm` and `repeat` are `js.UndefOr[Double | Null]`/`js.UndefOr[Int | Null]`,
  matched with `case d: Double`, so `undefined` and `null` both take the default (20, 1, 1), as
  the adapter's `??` does.
- **Optional arguments.** `n` and `mode` are read like the options, turned into `Option`s for the
  methods (upstream's optional arguments); an absent `mode` is `Big.RM`.
- **Operands.** `x` and `y` are matched as `String` or `Double`. A number goes through `toString`
  (Scala.js prints doubles as JS does), except that −0 becomes `"-0"`, as in big.js's
  constructor; anything else goes through `String.valueOf`.
- **Output.** The results join with newlines; a `BigError` contributes its message.

## Representation and numeric semantics

- **Coefficient.** An `ArrayBuffer[Int]`, mutated where upstream mutates its array:
  - `xc.length = sd` is `takeInPlace(sd)`;
  - `shift`/`pop`/`unshift` are `remove(0)`/`dropRightInPlace(1)`/`prepend`;
  - upstream's reverse-push-reverse prepending of zeros is `prependAll`.
- **Exponent and options.** The exponent is a `Double`, as upstream's is a JS number: past int32
  and 2^53 every increment, decrement and sum rounds as in JS, and Infinity and NaN exponents keep
  upstream's comparison chains. `dp`, `rm`, `n` and `mode` stay `Double`s until upstream validates
  them:
  - `isIntIn` is `n !== ~~n || n < lo || n > hi`, negated;
  - a rounding mode is checked by equality with 0 to 3.

  So values past int32 or 2^53 fail rather than wrap.
- **JS-isms, spelt out:**
  - `if (a = xe - ye)` tests JS truthiness (`truthy`);
  - `t.c.slice(0, e)` with a negative `e` is `head`;
  - `| 0` and `&` on exponents use `toInt32` (ECMAScript ToInt32).
- **Host number conversions:**
  - JS `Number()` is a typed global facade (`@JSGlobal("Number")`), upstream's own `+s`
    coercion;
  - `String()` is Scala.js `Double.toString`;
  - `toExponential()` is `js.JSNumberOps`.
- **NUMERIC.** Upstream's pattern and `i` flag in a `js.RegExp`. Java's regex `$` would also match
  before a final line terminator, and Scala.js's `java.util.regex` emulation tripled the bundle.

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

- **Loops.** Scala has no prefix `--`, `break` or `do … while`. So `r[--rl]` is a decrement
  followed by the read, and do-while loops are `while { body; condition } do ()`. Upstream's digit
  loop `for (n = 0; n < 10; n++) { compare; if (cmp < 0) subtract; else break }` is "compare, then
  subtract while the divisor is smaller", which cannot run ten times (the remainder stays below ten
  divisors).
- **Borrows.** A borrow walks left from the digit before the one that needs it while digits are
  zero. Upstream's `for (; ri && !r[--ri];) r[ri] = 9` reaches the same digit whenever a borrow is
  possible.
- **Remainder.** Upstream's remainder `[a[ai]]` past the dividend's end is `[undefined]`; here it
  is the empty buffer, and `r[0] !== undefined` is `nonEmpty`.
- **`k--`.** `k--` is "`k > 0`, then decrement". Both are false exactly at 0 and NaN.
- **Copies.** Each binary method copies its argument where upstream's `new Big(y)` does, and
  `times` builds a fresh result instead of reusing `y`.
- **Out of domain.** JSON `null` for `n` or `mode` is outside the domain (CONTRACT.md); this port
  treats it as absent.

## Performance accommodations

None for speed. Two choices are for size:

- the arithmetic loops are `while` loops rather than `for` over `Range`s;
- the boundary collects results in an `ArrayBuffer` rather than with `for … yield`.

`Range` pulls Scala.js's `RuntimeLong` and immutable `Vector` into the bundle, and these two
choices took the minified bundle from 156 KB to 75 KB.
