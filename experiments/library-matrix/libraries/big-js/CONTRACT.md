# big.js 7.0.1

Each input supplies a list of independent decimal operations in `cases`, and an optional positive `repeat` count. Each operation constructs a fresh decimal from `x`, a decimal string or a finite JSON number. The sign and coefficient may begin with a minus and include a decimal point and signed exponent. Number inputs use JavaScript's standard number-to-string conversion at the language boundary.

`plus`, `minus`, `times`, `div` and `cmp` take a second decimal in `y`. `pow` takes an integer exponent `n`. `sqrt` and `toString` need only the receiver. `round`, `toFixed`, `toExponential` and `toPrecision` accept an optional integer `n` and an optional rounding `mode`. The constructor's decimal places and rounding mode are configured independently for each case with `dp` and `rm` (defaults 20 and 1). Rounding modes 0 through 3 select round down, half up, half even and round up. Exponential display thresholds are the upstream defaults of −7 and 21.

The result joins all decimal strings, comparisons (-1, 0, 1) and exact upstream error messages with newlines in execution order. The benchmark runs 10,000 mixed operations on 50-digit operands, returning each value. The corpus includes scientific notation with exponents beyond signed 32-bit bounds, signed zeros, negative decimal places, ties, all rounding modes, and generated operands of up to 500 decimal digits.

`upstream-vectors.json` retains selected literal assertions, expected strings and source line numbers from [the upstream v7.0.1 method tests](https://github.com/MikeMcl/big.js/tree/v7.0.1/test/methods). Each method's file has the same name as its operation. The tiny-input square-root cases use the terminating rounding modes of the upstream Newton loop.

The standalone verifier checks the fixed answers in `exponent-vectors.json` and compares arithmetic and rounding against the pinned package around positive and negative powers of two. Benchmark variants vary both 50-digit operands while retaining their leading and final digits.
