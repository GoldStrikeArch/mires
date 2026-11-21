# big.js 7.0.1: Elm port

A hand-written Elm port of the subset in `libraries/big-js/CONTRACT.md`.

- **Representation.** Coefficients are immutable `List Int`s of digits. The arithmetic is folds
  and self tail calls, so 20,000-digit operands do not grow the stack.
- **Host.** The JS host (`js/index.js`) only sends the input through a port and returns the reply.

| File | What it is |
| --- | --- |
| `src/Decimal.elm` | the decimal record and big.js's parsing, arithmetic, rounding, square root and formatting |
| `src/Main.elm` | the boundary (what `libraries/big-js/reference.mjs` does), with `Json.Decode` |
| `js/index.js` | the host: `run(input)` sends the input to the `request` port and returns the `response` |

## Boundary

- **Options.** A number option is `Maybe Float`. An absent, `undefined` or `null` field fails
  `D.float`, so `dp`, `rm` and `repeat` take the defaults 20, 1 and 1, as the adapter's `??` does.
- **Optional arguments.** `n` and `mode` stay `Maybe` and reach `Decimal` as upstream's optional
  arguments.
- **Validation.** The options stay `Float`s until upstream would validate them: `intIn` is
  `n !== ~~n || n < lo || n > hi`, negated, and a rounding mode is checked by equality with 0 to 3.
  So non-integers and values past int32 fail where upstream's checks fail, without the former
  out-of-range integer stand-ins.
- **Operands.** A number goes through `String.fromFloat` (JS `String()`), except that −0 becomes
  `"-0"`, as in big.js's constructor.

## Numeric semantics

**JS numbers underneath.** Elm's `Int` is a JS number, so exponents past int32 and 2^53 round as
upstream's do. The places where that matters:

- **Parse (review b8, `Decimal.elm:58`).** The exponent is one `String.toFloat` of its signed
  digits (JS `+s`), and the result's exponent is `((dot + exp) - lead) - 1`, as upstream computes
  it. The old `String.toInt` summed digits in a double.
- **Rounding of exponents.**
  - Sums: a carry increments the larger exponent once.
  - Differences: decrement it once per leading zero, as upstream's `--ye` loop does.
  - Products and rounding: increment only on a final carry.

  The former `(e + 1) - shift` rounded differently past 2^53.
- **Infinity and NaN exponents.**
  - Zeros are prepended only when the exponent difference is JS-truthy (neither 0 nor NaN).
  - `stringify` keeps upstream's chain `e < 0`, `e > 0`, `n > 1`, which a NaN exponent fails.
  - Fixed-notation padding with a NaN width pads nothing.
  - Elm compiles a comparison of two non-literal numbers to `_Utils_cmp`, which orders NaN above
    everything, and `== 0` to a falsiness test, which NaN passes. Where a value can be NaN, the
    code tests `isNaN` first.
  - The builder's Terser pass runs with `unsafe_comps`, which assumes no NaN and rewrites
    `!(p >= 1)` as `p < 1`, so the code never negates a comparison that may see NaN.

## Square root

`sqrt` runs upstream's iteration exactly (review b8):

- the binary64 estimate, with the underflow/overflow re-estimate and upstream's exponent
  `((e + 1) / 2 | 0) - (e < 0 || e & 1)`:
  - Elm's `truncate` is `| 0` and `Bitwise.and` is `&`;
  - the subtraction is on JS numbers, so it reaches −2^31 − 1 where upstream does;
- `Big.DP += 4`, passed to the division, which validates it, so `dp` ≤ −5 or a non-integer `dp`
  gives `Invalid decimal places`;
- the stop test on the first `r.e + DP + 4` digits of successive estimates.

The former shortcut for tiny operands and the changed stop test are gone. Where upstream's loop
never stops (CONTRACT.md: mostly tiny operands near the last requested place), this port does not
stop either. Those inputs are outside the domain, because the reference gives no answer to compare
against. The choice keeps results and termination identical to upstream's wherever upstream
answers.

**The estimate's digits.** Upstream parses the digits of `toExponential()` before its `e`. This
port parses `String.fromFloat` of the estimate and sets the exponent. Both print the shortest
round-trip digits of the same double.

## Deviations from upstream

- **Division.** The decimal long division is restructured over lists: it drops a leading zero
  eagerly (exponent − 1) and stops once it has one digit past the precision. That matches
  upstream's digit count and rounding when the precision `p = dp + q.e + 1` is at least 1.

  When `p` is below 1 or NaN, upstream's `k--` stops after the first digit, and the port follows
  upstream's order exactly:
  - it keeps that digit even when it is zero, at the undecremented `q.e`;
  - it rounds it to `p`, or for a NaN `p` (an Infinity − Infinity exponent) not at all.

  Past 2^53 the exponent sums round, so dropping the zero first gave a different power of ten under
  rounding mode 3. Re-review re-big-js found 12 such quotients in 4,000, for example
  `771e-9007199254740991 / 9717e2` at `dp` 1; the corpus case `div-precision-below-one` holds them.
- **Addition.** Addition aligns both coefficients at the larger exponent and combines them in one
  right-to-left fold.
- **Out of domain.** JSON `null` for `n` or `mode` is outside the domain (CONTRACT.md); this port
  treats it as absent.

## Performance accommodations

None.
