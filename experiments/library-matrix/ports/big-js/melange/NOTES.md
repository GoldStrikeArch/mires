# big.js 7.0.1: Melange port

A hand-written, faithful port of the subset in `libraries/big-js/CONTRACT.md` to OCaml, compiled
by Melange 7.0.1 (`melange.emit`, ES6, the emitted stdlib bundled). It follows
`node_modules/big.js/big.mjs` function by function: `parse`, `round` (here `round_to`),
`stringify`, `cmp`, `div`, `minus`, `plus`, `pow`, `P.round` (`round`), `sqrt`, `times`,
`toExponential`, `toFixed`, `toPrecision` and `toString`, with upstream's loops, order of checks
and error messages. It replaces a port that a Python generator had emitted from a shared core.

| File | What it is |
| --- | --- |
| `src/big.ml` | the decimal record `t`, `Big_error`, upstream's functions, and `operate` (one reference operation) |
| `src/host.mli` | the three JS number conversions the core needs |
| `src/host.ml` | their Melange implementation: externals for `Number`, `String` and `toExponential` |
| `src/index.ml` | the boundary, doing what `libraries/big-js/reference.mjs` does; `run` is the module's export |

**Shared source.** `big.ml` and `host.mli` are byte-identical to `ports/big-js/jsoo/src/`, as the
README allows; only `host.ml` and the entry module differ.

## Boundary

- **Input.** The input and its cases are `Js.t` objects read with `##`, with no conversion.
- **Options.** `dp`, `rm` and `repeat` are `Js.nullable`, so `undefined` and `null` both take the
  default (20, 1, 1), as the adapter's `??` does.
- **Optional arguments.** `n` and `mode` are `Js.undefined`, which become options and reach the
  core as upstream's optional arguments; an absent `mode` is `Big.RM`.
- **Operands.** Classified with `Js.Types.classify`: a string is used as it is; anything else goes
  through JS `String()`, except that a number −0 becomes `"-0"`, as in big.js's constructor.
- **Output.** The results join with newlines; a `Big_error` contributes its message.

## Representation and numeric semantics

- **Values.** A decimal is an immutable record. Its coefficient is an `int array` of digits, most
  significant first, built afresh by each operation:
  - where upstream mutates its own copy in place (borrows, carries, the division remainder), the
    core mutates a fresh copy;
  - where upstream shifts, pops or truncates, it takes `Array.sub`;
  - where upstream unshifts or prepends zeros, it uses `Array.append`.

  The division's quotient digits gather in a list, last first.
- **Exponent and options.** The exponent is a `float`, as upstream's is a JS number: past int32
  and 2^53 every increment, decrement and sum rounds as in JS. `minus` therefore decrements once
  per leading zero, as upstream's `--ye` loop does, rather than subtracting the count. Infinity and
  NaN exponents keep upstream's comparison chains: OCaml's float comparisons are IEEE.

  `dp`, `rm`, `n` and `mode` stay floats until upstream validates them:
  - `is_int_in` is `n !== ~~n || n < lo || n > hi`, negated;
  - a rounding mode is checked by equality with 0 to 3.

  So values past int32 or 2^53 fail rather than wrap.
- **JS-isms, spelt out:**
  - `if (a = xe - ye)` tests JS truthiness (`truthy`);
  - `t.c.slice(0, e)` with a negative `e` is `head`;
  - `| 0` and `&` on exponents use `to_int32` (ECMAScript ToInt32, from `Float.rem` and
    `Float.trunc`).
- **Host number conversions (`Host`).** `Number(s)`, `String(n)` and `n.toExponential()`, as
  upstream uses them: to parse an exponent and the estimate's text, and to print exponents.
- **NUMERIC.** A hand-written scanner (`numeric`), since Melange has no `Str`. It accepts exactly
  upstream's grammar over ASCII.

## Square root

`sqrt` runs upstream's iteration exactly:

- the host's binary64 estimate, with its underflow/overflow re-estimate. The exponent is ToInt32 of
  `(e + 1) / 2`, then a subtraction in doubles, as upstream's `((e + 1) / 2 | 0) - (e < 0 || e & 1)`
  is, so it reaches −2^31 − 1 at x.e = −4294967297 and −4294967298 (re-review re-big-js);
- `Big.DP += 4`, passed to `div`, which validates it, so `dp` ≤ −5 or a non-integer `dp` raises
  `Invalid decimal places`;
- the stop test on the first `r.e + DP + 4` digits of successive estimates.

There is no shortcut for small operands. Where upstream's loop never stops (CONTRACT.md: mostly
tiny operands near the last requested place), this port does not stop either. Those inputs are
outside the domain, because the reference gives no answer to compare against. The choice keeps the
port's results and termination identical to upstream's wherever upstream answers.

## Deviations from upstream

- **Digit loop.** Upstream's `for (n = 0; n < 10; n++) { compare; if (cmp < 0) subtract; else
  break }` is "compare, then subtract while the divisor is smaller", which cannot run ten times
  (the remainder stays below ten divisors).
- **Borrows.** A borrow walks left from the digit before the one that needs it while digits are
  zero. Upstream's `for (; ri && !r[--ri];) r[ri] = 9` reaches the same digit whenever a borrow is
  possible.
- **Remainder.** Upstream's remainder `[a[ai]]` past the dividend's end is `[undefined]`; here it
  is the empty array, and `r[0] !== undefined` is a length test.
- **`k--`.** `k--` is "`k > 0`, then decrement". Both are false exactly at 0 and NaN.
- **Records.** Records are immutable, so upstream's defensive `new Big(...)` copies are not needed
  except for the arrays a method mutates.
- **Out of domain.** JSON `null` for `n` or `mode` is outside the domain (CONTRACT.md).

## Performance accommodations

None.
