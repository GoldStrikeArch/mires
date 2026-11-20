# big.js 7.0.1: ReScript port

A hand-written, faithful port of the subset in `libraries/big-js/CONTRACT.md` to ReScript 12.3.0
(ES module emit with `@rescript/runtime`). It follows `node_modules/big.js/big.mjs` function by
function: `parse`, `round` (here `roundTo`), `stringify`, `cmp`, `div`, `minus`, `plus`, `pow`,
`P.round` (`round`), `sqrt`, `times`, `toExponential`, `toFixed`, `toPrecision` and `toString`,
with upstream's loops, order of checks and error messages. It replaces a port that a Python
generator had emitted from a shared core.

| File | What it is |
| --- | --- |
| `src/Big.res` | the decimal record `t`, `BigError`, and upstream's functions |
| `src/Index.res` | the boundary, doing what `libraries/big-js/reference.mjs` does; `run` is the module's export |

## Boundary

- **Input.** The input and its cases are records, which are the JS objects themselves, with no
  conversion.
- **Options.** `dp`, `rm` and `repeat` are `Nullable.t`, so `undefined` and `null` both take the
  default (20, 1, 1), as the adapter's `??` does.
- **Optional arguments.** `n` and `mode` are optional fields (`option<float>`) and reach the
  functions as upstream's optional arguments; an absent `mode` is `Big.RM`.
- **Operands.** An `@unboxed` variant `String(string) | Number(float)`. A number goes through
  `Float.toString` (JS `String()`), except that −0 becomes `"-0"`, as in big.js's constructor; an
  absent `y` is `"undefined"`, as `String(undefined)` is.
- **Output.** The results join with newlines; a `BigError` contributes its message.

## Representation and numeric semantics

- **Coefficient.** A JS array of digits, mutated where upstream mutates its array:
  - `xc.length = sd` uses a typed `@set` external, `setLength`;
  - `push`, `pop`, `shift`, `unshift` and `slice` are `Array`'s bindings;
  - upstream's reverse-push-reverse prepending of zeros is a `concat` onto zeros;
  - reads that upstream lets fall off the end (`xc[1] !== undefined`, `a[ai] || 0`) are ReScript's
    `option`-returning `xc[i]`; other reads are `Array.getUnsafe`.
- **Exponent and options.** The exponent is a `float`, as upstream's is a JS number: past int32
  and 2^53 every increment, decrement and sum rounds as in JS, and Infinity and NaN exponents keep
  upstream's comparison chains. `dp`, `rm`, `n` and `mode` stay `float`s until upstream validates
  them:
  - `isIntIn` is `n !== ~~n || n < lo || n > hi`, negated;
  - a rounding mode is checked by equality with 0 to 3.

  So values past int32 or 2^53 fail rather than wrap.
- **JS-isms, spelt out:**
  - `if (a = xe - ye)` tests JS truthiness (`truthy`);
  - `t.c.slice(0, e)` with a negative `e` is `head`;
  - `| 0` and `&` on exponents use `toInt32` (ECMAScript ToInt32).
- **Host number conversions:**
  - JS `Number()` is a typed `@val` external;
  - `String()` is `Float.toString`;
  - `toExponential()` is `Float.toExponential`.
- **NUMERIC and `search(/e/i)`.** Upstream's regular-expression literals (`%re`).

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

- **Loops.** ReScript has no `break` and no early `return`. Upstream's loops that leave early run
  on `ref` flags, and its early returns are `if`/`else` chains. Upstream's digit loop
  `for (n = 0; n < 10; n++) { compare; if (cmp < 0) subtract; else break }` is "compare, then
  subtract while the divisor is smaller", which cannot run ten times (the remainder stays below ten
  divisors).
- **Borrows.** A borrow walks left from the digit before the one that needs it while digits are
  zero. Upstream's `for (; ri && !r[--ri];) r[ri] = 9` reaches the same digit whenever a borrow is
  possible.
- **Remainder.** Upstream's remainder `[a[ai]]` past the dividend's end is `[undefined]`; here it
  is the empty array, and `r[0] !== undefined` is a non-empty test.
- **`k--`.** `k--` is "`k > 0`, then decrement". Both are false exactly at 0 and NaN.
- **Records.** Records are copied where upstream's `new Big(y)` copies, and `times` builds a fresh
  result.
- **Out of domain.** JSON `null` for `n` or `mode` is outside the domain (CONTRACT.md).

## Performance accommodations

None.
