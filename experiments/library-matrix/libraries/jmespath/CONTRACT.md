# JMESPath 0.16.0

The input carries a JSON document in `data` and a sequence of queries. Each query names the `search` or `compile` operation and supplies an expression. `search` evaluates it against the document. `compile` returns the package's complete Pratt-parser AST, including its property order, slice arguments and expression-reference nodes. Each result has a `value` field on success or an `error` field with the JMESPath error category. One failing expression does not interrupt the remaining queries. The matrix compares the complete result arrays.

The corpus exercises all 26 built-in functions and includes every correctness case in revision `53abcc37901891cf4308fcd910eab287416c4609` of the official compliance suite. Each case runs as both a search and an AST compilation. Its independent expected search result is preserved in the corpus. The reference uses the unmodified CommonJS source from the npm package, wrapped by the harness postinstall step for ES module loading. [Compliance fixtures](https://github.com/jmespath/jmespath.test/tree/53abcc37901891cf4308fcd910eab287416c4609/tests)

The benchmark evaluates 100 expressions over 1,024 records. Both sides parse each expression once during its search call. Version 0.16.0 exports `compile` and `search`, while its public search entry takes an expression string. The variant rotates the records and adds a fixed-width call number to every name and detail. Record counts and queries stay fixed across calls.

A search can return part of the input object directly. Firefox WebDriver supplies those objects from a different JavaScript realm. The harness normaliser copies plain records and arrays into the checking realm and preserves property order, undefined, NaN and signed zero. With prototype-identity checking alone, the npm reference failed its own output check on 46 corpus groups in Firefox.

The fixed suite has 892 search assertions. Nine differ from the pinned JS release. Four cross-type comparisons use JavaScript coercion, the average of an empty list yields NaN, and the two empty min/max-by operations yield undefined. The malformed call `@(foo)` reports an unknown function, and `[:::]` returns null for the fixture's non-array input. The verification script checks the npm behavior at those nine named expressions and uses the official expected values at every other assertion.

Run `node verify.mjs <language>` from this directory after building a port. The verifier also compares 5,600 generated search and compile results and 9,204 cross-type comparisons and function calls, including hexadecimal, binary and octal numeric strings.

## Behaviours of 0.16.0 that ports reproduce

Beyond the nine pinned deviations, these behaviours of the npm source are part of the contract. Each one has a `review-*` corpus case.

- **Filter projections run in two passes.** Every condition is evaluated first, then the projection over the elements that passed. The first error raised is therefore npm's (`[?abs(@) == `1`].nope(@)` on `[1, "1"]` is invalid-type, not unknown-function).
- **`max` and `min` of numbers are `Math.max.apply` and `Math.min.apply`.** NaN wins, `max` prefers +0 to -0 and `min` prefers -0. `max` and `min` of strings use `localeCompare`. `abs` is `Math.abs`, which turns -0 into +0.
- **`type()` of undefined is undefined.** `_getTypeName` has no case for it. An empty `max_by` or `min_by`, or one whose keys never beat ∓∞, gives undefined, which then flows on. `not_null` returns it, `to_string` gives undefined, and projections keep it.
- **`strictDeepEqual` compares own keys.** It compares each own key of the first object against the second object's value for that key, so an undefined value equals a missing key. It then checks that the second object has no key the first lacks.
- **Objects tagged `jmespathType: "Expref"`.** Type checks and `type()` treat them as expression references, while `strictDeepEqual`, `isFalse`, field access and projections treat them as objects.
- **Number tokens are `parseInt` of a run of digits and dashes.** So `-` and `--` are NaN, `1-2` is 1, and `-0` is -0. Index and slice arithmetic is JS arithmetic on those numbers, including NaN and values past int32 and 2^53. `[3:0:-]` on a three-element array is `[undefined]`, because the loop reads `value[3]`.
- **`bindingPower` has no entry for Colon or Literal.** A projection followed by a literal is a syntax error, because `bindingPower[Literal] < 10` is false in `_parseProjectionRHS`. `[a[*] `1`]` is an example.
- **Backtick literals are trimmed with `String.prototype.trimLeft`.** It removes exactly ECMAScript WhiteSpace and LineTerminator: U+0009–U+000D, U+0020, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000 and U+FEFF. It does not remove U+001C–U+001F, U+0085, U+180E or U+200B.
- **`merge` type-checks only its first argument.** For every argument it runs `for (key in current)`, so later strings and arrays contribute their indices, and other primitives and null contribute nothing.
- **`contains` is `indexOf`.** On strings the search value goes through `String()`. On arrays it uses `===`, so only the very same array or object is found, NaN never matches and -0 matches 0.
- **`sort_by` decorates with the index and sorts with the engine's `Array.prototype.sort`.** It checks each key's type inside the comparator. With NaN keys (`&avg(@)` over empty arrays) the comparator is inconsistent, and the result depends on the engine's sort algorithm. npm's own output then differs between V8 and SpiderMonkey, so case `review-sort-by-engine` makes the summary's `referenceStableAcrossEnvironments` false. Ports must sort with the engine.
- **`starts_with` is `lastIndexOf(prefix) === 0`.** So `starts_with('abab', 'ab')` is false. `ends_with` is `indexOf(suffix, length - suffix.length) !== -1`.

## Parity domain

These inputs are outside the parity domain:

- **Names that resolve through `Object.prototype`.** This covers field names (`__proto__`, `constructor` and `toString` on an object that lacks them), function names (`constructor(@)` and `toString(@)` find a function in npm's plain-object table and fail on its missing signature), and input objects whose own keys shadow `Object.prototype` methods that npm calls (`hasOwnProperty`). The reason is that npm's results there come from the JS prototype chain, not from JMESPath. Own data properties named `__proto__` in the input (from `JSON.parse`) are inside the domain. So is assigning a primitive to `__proto__` as a multiselect-hash or `merge` key, which the prototype setter ignores. Both are existing corpus regressions (`property-and-subscript-boundaries`). Assigning an object or null to `__proto__` is outside the domain: the result has a non-standard prototype.
- **Nesting deeper than 500 levels.** Expressions and data may nest at most 500 levels. Each prefix operator, binary-operator or subexpression link, bracket, brace, parenthesis, function call and projection counts one level. So does each array or object level of the document or of a literal. npm's own ceilings were measured through the checker's pipeline (`structuredClone`, run, normaliser, `canonicalText`) in Node 26.8.1, Chrome 158 and Firefox 160. The lowest is 918, for Firefox comparing a nested array with `<`, whose `Array.prototype.join` recursion is the limit. Every other construct reaches at least 1,499 in all three. The bound leaves a margin for engines' stack sizes. Past it npm overflows its stack. Stack exhaustion is not a JMESPath error, so the reference adapter does not report it as a category: the RangeError of V8 and JavaScriptCore or the InternalError of SpiderMonkey propagates, and the check fails. Each port's catch-all lets its own stack exhaustion propagate the same way, so a port that overflows early cannot pass for one that reports "syntax". Case `review-nesting-500` runs every construct at the bound.
- **Objects carrying `jmespathType` used as expression arguments.** When such an object is passed to `map`, `sort_by`, `max_by` or `min_by`, npm reads its other keys as an AST. Comparing, testing and typing such objects is inside the domain.
- **Numeric `max` and `min` over arrays longer than the engine's argument limit.** npm calls `Math.max.apply`, which throws RangeError (Node 26 throws from about 125,000 elements). The corpus stays at 20,000.
- **Error messages.** Only categories are compared.

The Elm port has two narrower exclusions, both listed in its `NOTES.md`. The first concerns `sort_by` keys: when one key raises invalid-value and another has the wrong type, Elm checks keys in element order, not in the engine's comparison order. The second concerns live AST mutation: an expression reference taken as a value and evaluated again later tags its descendants, which npm shows in output serialised afterwards.

## Corpus

`corpus.json` has 247 cases. `generate-corpus.mjs` writes it deterministically from the compliance checkout. The cases are:

- 225 compliance groups, each query as a search and a compilation;
- `property-and-subscript-boundaries`: own `__proto__` data properties, `__proto__` assignment and indices past 2^32;
- `bench-search`: the bench input.

The pre-merge review (2026-11-20) added 20 `review-*` cases:

| Case | Exercises |
| --- | --- |
| `filter-two-pass`, `filter-two-pass-mixed` | J1: condition before projection, error order of filters, projections, `sort_by` and `map` |
| `math-max-min` | J2, J7: `Math.max`/`Math.min`/`Math.abs` with signed zeros and NaN, `ceil`/`floor` of -0.5, string max/min |
| `undefined-results` | J3: `type()` and every consumer of an undefined `max_by`/`min_by` |
| `literal-trim-left` | J4: 21 whitespace and non-whitespace code points before a backtick literal |
| `wide-2000-records` | J6, the review's input |
| `expref-tag` | Tagged objects in equality, truthiness, `type()`, `keys` and `contains`; `&expr` values |
| `slice-numbers`, `slice-numbers-ast` | NaN tokens (`[0:3:-]`, `[-:]`), `[3:0:-]`, tokens past int32 and 2^53, -0 |
| `projection-binding-power` | Literal or Colon after a projection |
| `deep-equal-undefined` | `strictDeepEqual` with undefined values |
| `merge-for-in` | `merge` over strings, arrays, primitives, null and an expression reference |
| `sort-by-engine` | NaN keys (engine-dependent), key type errors, the default `sort` |
| `contains-identity` | `===` identity of input, literal and built values, and `String()` coercion on strings |
| `string-functions` | `starts_with`/`ends_with` corners, UTF-16 length and reverse, `to_number` |
| `numbers-past-2-53` | Hexadecimal, octal and binary strings past 2^53, `1e400`, number printing |
| `lexer`, `parser` | Unterminated literals, a lone `=`, escapes, undefined AST nodes, calls on non-identifiers, optional commas, `@` arguments |
| `nesting-500` | Each construct at the nesting bound, as search and compile, and data nested 500 deep |
| `wide-20000-records` | Projections, filters, sorts, slices and functions over 20,000 records |
