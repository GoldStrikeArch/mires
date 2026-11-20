# marked 18.1.1: Kotlin/JS port

A faithful port of the synchronous Markdown-to-HTML pipeline in `libraries/marked/CONTRACT.md` to
Kotlin/JS 2.4.10 (the harness's Gradle production lane: whole-program ES module, ES2015, production
DCE and member minimisation, followed by 2-pass Terser). It mirrors `marked@18.1.1`'s exact
architecture (`rules`, `helpers`, `Tokenizer`, `Lexer`, `Renderer`, `TextRenderer`, `Parser`,
`Marked`) across all three grammar modes (`normal`, `gfm`/`breaks`, `pedantic`) and both entry
points (`parse`, `parseInline`), with zero `js("…")`, zero `eval`, zero host JS, and zero
module-state lint findings. It reaches exact character-for-character parity on all 80 corpus cases
in Node, Chrome, and Firefox (both `dist/index.min.js` and `dist/index.js`), and passes 2,787
targeted adversarial and seeded randomized differential checks (5,574 `min` + `readable` comparisons)
against `reference.mjs` across all 8 `(gfm, breaks, pedantic)` combinations.

| File | What it is |
| --- | --- |
| `src/JsInterop.kt` | Typed `external` bindings for host `String` methods (`JsString`, `JsStringStatic.fromCodePoint`), `JsArray<T>`, `JsMap<K, V>`, `encodeURI`, `parseInt`, and zero-overhead `RegExp`/`RegExpMatch` inline helpers |
| `src/Options.kt` | `MarkedOptions`, `LinkDef`, `TableCell`, `InlineQueueItem`, `LexerState`, and the `Token` sealed class hierarchy |
| `src/Rules.kt` | `RuleEditor` (`edit`), `supportsLookbehind`, `OtherRules` (with eagerly prebuilt 4-entry indent-clamped regex tables), `BlockRules`, `InlineRules`, and immutable top-level rule sets (`RULES_NORMAL`, `RULES_GFM`, `RULES_BREAKS`, `RULES_PEDANTIC`) |
| `src/Helpers.kt` | `escapeHtmlEntities`, `decodeNumericCharacterReferences`, `cleanUrl`, `splitCells`, `rtrim`, `trimTrailingBlankLines`, `normalizeLabel`, `findClosingBracket`, `expandTabs`, `codePointCount`, `firstCodePointLength` |
| `src/Tokenizer.kt` | `Tokenizer` with `outputLink`, `indentCodeCompensation`, `isLabelEndInsideToken`, and all 13 block + 11 inline tokenizers (including GFM tables, nested blockquote/list continuations, task checkboxes, emphasis/strong/del delimiter matching, and GFM URL `_backpedal`) |
| `src/Lexer.kt` | `Lexer` (`lex`, `blockTokens`, `inline`, `linkInText`, `maskReflink`, `inlineTokens` with `linkParenPossible` guard and 3-pass `maskedSrc` pre-scan, `infiniteLoopError`) |
| `src/Renderer.kt` | `InlineRenderer` interface, `Renderer`, and `TextRenderer` (image `alt` flattening) |
| `src/Parser.kt` | `Parser` (`parse`, `parseInline`) and `Marked` entry facade (`parse`, `parseInline`) |
| `src/Main.kt` | Boundary (`OptionsJson`, `InputJson` external interfaces) and the single `@JsExport` entry `run(input: InputJson): String` |

## Architecture and interop decisions

- **Native `kotlin.js.RegExp` throughout.** `Rules.kt` compiles `marked@18.1.1`'s exact grammar via
  `kotlin.js.RegExp` rather than `kotlin.text.Regex`, preserving exact ECMAScript `u` vs non-`u`
  UTF-16 semantics, lookbehind (`(?<!`)`), named backreferences (`\k<a>`, `\k<b>`), and avoiding
  `kotlin.text.Regex` / `MatchResult` / `Sequence` stdlib overhead. Every shared `/g` regex resets
  or saves/restores `lastIndex` around `exec` loops (`linkInText`, `emStrong`, `del`), and the 6
  indent-clamped list regex families are prebuilt eagerly as immutable 4-element `Array<RegExp>`
  tables at module init (`moduleState: 0`).
- **Exact ECMA-262 string and whitespace semantics.** Kotlin's `String.trim()` follows Java
  `Char.isWhitespace()`, which diverges from JS `String.prototype.trim()` on `U+00A0` (NBSP),
  `U+FEFF` (BOM), and `U+001C..U+001F`. Binding native JS `String.prototype` methods (`trim`,
  `trimStart`, `trimEnd`, `toLowerCase`, `toUpperCase`, `replace`, `search`, `split`, `slice`,
  `substring`, `includes`, `repeat`, `indexOf`, `lastIndexOf`) through an `external interface
  JsString` preserves exact ECMA-262 whitespace trimming, Unicode case folding (`normalizeLabel`),
  negative-index slicing, and out-of-bounds `substring` clamping with zero runtime wrapper cost.
- **Unicode code-point helpers without spread allocations.** `expandTabs`, `codePointCount`, and
  `firstCodePointLength` scan UTF-16 surrogate pairs directly, matching JS `for (const char of line)`,
  `[...s].length`, and `[...s][0].length` without allocating intermediate character arrays.
- **Prototype-safe `JsMap` and native `JsArray`.** `Lexer.links` uses a per-call host `Map`
  (`@JsName("Map") external class JsMap<K, V>`), making `Object.prototype` link labels
  (`constructor`, `toString`, `__proto__`, `hasOwnProperty`) safe by construction while avoiding
  Kotlin's `InternalHashMap` / `LinkedHashMap` and `ArrayList` stdlib footprint.
- **Zero `lateinit` or stdlib runtime bloat.** `Tokenizer` receives `(options, rules, lexer)` and
  `Renderer` receives `(options, parser)` via primary constructors, eliminating `lateinit` getter
  checks (`throwUninitializedPropertyAccessException`). String emptiness checks use `.length == 0` /
  `.length > 0` instead of `isEmpty()` / `isNotEmpty()` so Kotlin/JS does not route every check
  through `charSequenceLength`. Non-capturing replacer lambdas are top-level `val`s rather than
  callable references (`::fn`). Neither `Long`, `HashMap`, `ArrayList`, nor `kotlin.text.Regex` is
  pulled into `dist/index.js`.
- **Zero cross-call mutable state.** Only `marked`'s immutable compiled regex rules and stateless
  replacer lambdas live at module scope (`moduleState: 0`). Every `run` call instantiates fresh
  `Lexer`/`Tokenizer`/`Parser`/`Renderer` state and returns a native JS `String` directly.

## Kotlin/JS 2.4.10 findings

1. **`kotlin.js.RegExp` omits `.source` in its stdlib declaration.** Added an inline extension
   property (`RegExp.source`) in `JsInterop.kt` so `RuleEditor.replace` can compose patterns from
   existing `RegExp` instances exactly like `marked`'s `edit()`.
2. **Gradle 9.4.1 macOS FSEvents deadlock.** Prefixing Gradle builds with
   `GRADLE_OPTS="-Dorg.gradle.vfs.watch=false"` avoids Gradle 9.4.1's macOS FSEvents deadlock under
   `--no-daemon` without modifying any build script or compiler flag.
3. **Reproducibility.** Clean builds are byte-identical (`reproducibility.identical === true`) under
   the harness template's in-process `-XX:hashCode=2` JVM setting (~11 s per clean build).

## Size

- `dist/index.min.js`: **51,818 B** raw, **15,401 B** gzip-9, **13,319 B** brotli-11 (JS reference
  for scale: 46,922 B raw / 13,794 B gzip-9 / 12,495 B brotli-11; Kotlin runtime floor: 46 B).
- Pre-Terser `dist/index.js`: **184,066 B** (zero `Long`, `HashMap`, `ArrayList`, `Regex`,
  `charSequenceLength`, or `throwUninitializedPropertyAccessException`).
- Lints: `lines: 2566`, `embeddedJs: 0`, `evalRefs: 0`, `hostJs: 0`, `moduleState: 0`.

## Quick Node performance check

Measured with `tools/bench.mjs --environments node --samples 6` (triplet design, bench variant;
isolated `dev-marked-kotlin` receipts; not a performance claim — read ratios next to A/A and load):

| Case | Node port p50 | JS ref p50 | port/js | paired | A/A |
| --- | --- | --- | --- | --- | --- |
| `bench-parse` | ~2.40 ms | ~2.29 ms | ~1.05 | ~1.01 | ~1.03 |
| `bench-gfm` | ~2.29 ms | ~2.33 ms | ~0.98 | ~0.96 | ~1.01 |

Because the port uses native `kotlin.js.RegExp`, host `String`/`Array`/`Map` methods, and direct
constructor injection with zero boundary conversion overhead, it runs at parity with `marked@18.1.1`
in V8 on both `bench-parse` and `bench-gfm`.
