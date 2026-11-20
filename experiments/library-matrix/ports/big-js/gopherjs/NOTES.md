# big.js 7.0.1: Go port (GopherJS)

A hand-written, faithful port of the subset in `libraries/big-js/CONTRACT.md` to Go 1.21, compiled
by GopherJS 1.21.0 (`gopherjs build -m`). It follows `node_modules/big.js/big.mjs` function by
function: `parse`, `round`, `stringify`, `cmp`, `div`, `minus`, `plus`, `pow`, `P.round`, `sqrt`,
`times`, `toExponential`, `toFixed`, `toPrecision` and `toString`, with upstream's loops, order of
checks and error messages. It replaces a port that a Python generator had emitted from a shared
core.

| File | What it is |
| --- | --- |
| `src/big.go` | the decimal type `Big`, NUMERIC, and upstream's functions as functions and methods |
| `src/host.go` | the three JS number conversions upstream relies on, from the host |
| `src/main.go` | the boundary, doing what `libraries/big-js/reference.mjs` does; publishes `run` on `js.Module.exports` |

## Boundary

- **Input.** Read through `*js.Object`.
- **Options.** `option` reads a number field as `*float64`, nil when it is `undefined` or `null`
  (GopherJS gives a nil `*js.Object` for null). So `dp`, `rm` and `repeat` take the defaults 20, 1
  and 1, as the adapter's `??` does.
- **Optional arguments.** `n` and `mode` stay optional (`*float64`) and reach the methods as
  upstream's optional arguments; an absent `mode` is `Big.RM`.
- **Operands.** `x` and `y` are switched on `Interface()`:
  - a string is used as it is;
  - a number goes through the host's `String()`, except that −0 becomes `"-0"`, as in big.js's
    constructor;
  - anything else goes through `String()`.
- **Output.** The results join with newlines; an error contributes its message.

## Representation and numeric semantics

- **Errors.** Upstream throws; this port returns errors (`bigError`, a string type) from every
  function that can fail, in upstream's order of checks.
- **Coefficient.** A `[]int`; upstream's array mutations are slice operations:
  - `shift` is `r = r[1:]`;
  - `pop` and `length = sd` reslice;
  - `unshift` and prepended zeros append onto a new slice;
  - where upstream copies (`slice()`), so does the port.
- **Exponent and options.** The exponent is a `float64`, as upstream's is a JS number: past int32
  and 2^53 every `++`, `--` and sum rounds as in JS, and Infinity and NaN exponents keep upstream's
  comparison chains. `dp`, `rm`, `n` and `mode` stay `float64`s until upstream validates them:
  - `isIntIn` is `n !== ~~n || n < lo || n > hi`, negated;
  - a rounding mode is checked by equality with 0 to 3.

  So values past int32 or 2^53 fail rather than wrap.
- **JS-isms, spelt out:**
  - `if (a = xe - ye)` tests JS truthiness (`truthy`);
  - `t.c.slice(0, e)` with a negative `e` is `head`;
  - `| 0` and `&` on exponents use `toInt32` (ECMAScript ToInt32). It truncates with `math.Floor`
    and `math.Ceil`, because GopherJS's `math.Trunc` goes through a 32-bit `int`
    (`compiler/natives/src/math`). `math.Mod` is JS `%`.
- **Host number conversions (`host.go`).** `Number(s)` (NaN where `s` is not a number),
  `String(n)` and `n.toExponential()` come from the host through the `js` package, as upstream uses
  them: to parse an exponent and the estimate's text, and to print exponents. This also keeps
  `strconv` out of the bundle.
- **NUMERIC.** A hand-written scanner over the bytes. GopherJS strings are UTF-8, so any non-ASCII
  character is a non-digit byte and the operand is invalid, as in big.js.

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

- **Borrows.** A borrow walks left from the digit before the one that needs it while digits are
  zero. Upstream's `for (; ri && !r[--ri];) r[ri] = 9` reaches the same digit whenever a borrow is
  possible.
- **Remainder.** Upstream's remainder `[a[ai]]` past the dividend's end is `[undefined]`; here it
  is the empty slice, and `r[0] !== undefined` is `len(r) > 0`.
- **`k--`.** `k--` is "`k > 0`, then decrement". Both are false exactly at 0 and NaN.
- **Copies.** `times` builds a fresh result instead of reusing `y`.
- **Out of domain.** JSON `null` for `n` or `mode` is outside the domain (CONTRACT.md); this port
  treats it as absent.

## Performance accommodations

None.
