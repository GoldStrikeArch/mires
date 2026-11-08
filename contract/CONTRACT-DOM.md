# CONTRACT-DOM

What the Rust to JS emitter must reproduce to be compatible with the dom-expressions client runtime.

Every point below cites (a) the exact file and line in the pinned upstream tree, (b) the fixture or test that pins it, and (c) a `Topcoat delta:` line saying how our side differs. Points with no known divergence say so explicitly rather than staying silent, so that "no delta" is a recorded finding and not an omission.

Paths starting `dom-expressions/` are the npm tarball at `contract/vendor/upstream/dom-expressions/`. Paths starting `plugin/` are `contract/vendor/upstream/git/packages/babel-plugin-jsx-dom-expressions/`. Paths starting `solid/` are `contract/vendor/upstream/solid-js/`. Run `contract/vendor/fetch.sh` to populate all three.

The npm and git copies of `dom-expressions/src/*.js` are byte identical at these pins, so line numbers are interchangeable between them.

## 0. Provenance

### 0.1 The pins are mutually consistent despite differing gitHeads

`dom-expressions@0.40.8` was published from commit `56bd4054237b031c0cca8c1ec69da162c52ed4a6`, but `babel-plugin-jsx-dom-expressions@0.40.7` was published from the earlier `3c01eb2ac61b193d4164319befe3f1b2634549cb`. That looks like a mismatch, and it is worth stating why it is not one.

Diffing the two commits shows the plugin package directory is byte identical between them. The only changes are TypeScript declaration files, package versions, and lockfiles. No plugin source, and no dom-expressions runtime `.js`, changed. The 0.40.8 release was types only.

So the fixtures at `56bd405` are faithful to the published plugin 0.40.7, and the runtime behaviour pinned here is identical in 0.40.7 and 0.40.8.

- Source: `contract/upstream.lock`, both `gitHead` fields.
- Pinned by: the diff is reproducible with `contract/vendor/fetch.sh` plus a second fetch of `3c01eb2`.
- Topcoat delta: none known.

### 0.2 Reproducing upstream output requires pinning babel's internals, not just the plugin

Byte matching upstream's committed fixture outputs requires `@babel/core@7.20.12` with `@babel/traverse@7.23.2` and `@babel/generator@7.20.7` forced as resolutions. These are the versions in upstream's `pnpm-lock.yaml` at the pinned commit.

This is load bearing. `Scope.generateUid` lives in `@babel/traverse`, and its numbering changed after 7.23. A newer traverse emits `_tmpl$0` and `_el$0` where the committed fixtures have `_tmpl$10` and `_el$10`. The generated code is semantically identical but every identifier and every reference to it differs, so nothing byte matches. Pinning only `@babel/core` is not enough, because yarn hoists a newer traverse that still satisfies core's caret range.

Upstream also formats every fixture output through `prettier@2.8.2` before writing it, using the repo's `.prettierrc`. The committed `output.js` files are prettier artifacts, so reproducing them requires the same formatter.

- Source: upstream `pnpm-lock.yaml` at `56bd405`; `contract/vendor/package.json` resolutions.
- Pinned by: `contract/harness/verify-reference.mjs --all`, 43 of 43 fixtures reproduce.
- Topcoat delta: not applicable. This constrains the reference harness, not the emitter.

### 0.3 Plugin options that produced the fixtures

The four option sets, verbatim from upstream's spec files, are reproduced in `contract/harness/compile-reference.mjs` as `PRESETS`.

The hydratable DOM preset, from `plugin/test/dom-hydratable.spec.js`:

```js
{
  moduleName: "r-dom",
  builtIns: ["For", "Show"],
  generate: "dom",
  hydratable: true,
  contextToCustomElements: true,
  staticMarker: "@once"
}
```

Note what is absent: `wrapConditionals` and `requireImportSource` are set in the non-hydratable DOM preset (`plugin/test/dom.spec.js`) but omitted here. `wrapConditionals` defaults to `true` so the effect is the same, but the asymmetry is real and copying the wrong preset changes the output.

The full option defaults are in `plugin/src/config.ts:1-20`.

- Source: `plugin/test/dom.spec.js`, `dom-hydratable.spec.js`, `ssr.spec.js`, `ssr-hydratable.spec.js`.
- Pinned by: `contract/fixtures/reference-parity.json`.
- Topcoat delta: the emitter has no `moduleName` option to vary; it will always import from the vendored artifact. `builtIns`, `staticMarker`, and `requireImportSource` are JSX concepts with no Rust equivalent.

## 1. Module ABI

### 1.1 The client ABI is 48 exports

`dom-expressions/src/client.js` exports 48 names. The full list, with kind and arity, is in `contract/fixtures/abi.json`, extracted by importing the module rather than by reading it.

Arity is recorded as `Function.length`, which stops counting at the first parameter with a default or a rest parameter. `template(html, isImportNode, isSVG, isMathML)` reports 4, but `delegateEvents(eventNames, document = window.document)` reports 1. Treat arity as a lower bound on the declared parameter count, never as a call signature.

- Source: `dom-expressions/src/client.js:20-47` (re-exports), plus each `export function`.
- Pinned by: `contract/fixtures/abi.json`; `runtime/dom/check-exports.mjs` asserts the built artifact matches it exactly.
- Topcoat delta: none known. The emitter uses a 24 name subset, recorded as `emitterRequired` in the same fixture.

### 1.2 Every ABI name is available from solid-js/web

The plan anticipated that some ABI names might not be re-exportable from solid's public surface and would need aliasing or another source. Measured against `solid/web/dist/web.js`, none do. All 48 are present, including all 24 the emitter needs.

- Source: `solid/web/dist/web.js` export list.
- Pinned by: `abi.json` field `solidWebParity.missingFromSolidWeb`, which is empty; `runtime/dom/gen-index.mjs` refuses to generate an entry point if it ever becomes non-empty.
- Topcoat delta: none known.

### 1.3 Six exports are inert on the client

`useAssets`, `getAssets`, `Assets`, `generateHydrationScript`, `HydrationScript`, and `getRequestEvent` are all bound to a shared no-op `voidFn` in the client build. They exist so that isomorphic code links, and they do nothing.

- Source: `dom-expressions/src/client.js:41-46`.
- Pinned by: `abi.json` field `voidOnClient`.
- Topcoat delta: the emitter must not call these expecting behaviour. If Topcoat needs a hydration script it must emit one server side, not obtain one from the client runtime.

## 2. Template construction

### 2.1 The cloner is a lazy memoised singleton

`template()` returns a function that creates the prototype node once on first call and thereafter deep clones it.

`dom-expressions/src/client.js:67-88`:

```js
export function template(html, isImportNode, isSVG, isMathML) {
  let node;
  const create = () => {
    ...
    t.innerHTML = html;
    return isSVG ? t.content.firstChild.firstChild : isMathML ? t.firstChild : t.content.firstChild;
  };
  const fn = isImportNode
    ? () => untrack(() => document.importNode(node || (node = create()), true))
    : () => (node || (node = create())).cloneNode(true);
  fn.cloneNode = fn;
  return fn;
}
```

The prototype is never returned directly, only clones. `fn.cloneNode = fn` makes the cloner quack like a node so it can be passed where one is expected.

- Source: as quoted.
- Pinned by: `contract/fixtures/reference-traces/dom_fixtures.simpleElements.trace`, which records `template` followed by `template.clone`.
- Topcoat delta: none known. The emitter must hoist one `_$template` call per distinct template string to module scope, so the memoisation is per template and not per render.

### 2.2 The unwrap depth differs by namespace

The return line above unwraps a different number of levels per namespace: SVG unwraps two (`t.content.firstChild.firstChild`), MathML unwraps one from the element itself (`t.firstChild`, with no `.content`, because a namespaced `template` is not an `HTMLTemplateElement`), and the default unwraps one (`t.content.firstChild`).

SVG needs two because the compiler wraps the template string in a literal `<svg>` element first, at `plugin/src/dom/element.js:164-167`.

- Source: `dom-expressions/src/client.js:80`; wrapper at `plugin/src/dom/element.js:72` and `:164-167`.
- Pinned by: fixture family `__dom_fixtures__/SVG`.
- Topcoat delta: none known, but this is a correctness trap. Emitting `isSVG` without also prefixing `<svg>` to the template string, or the reverse, silently yields the wrong root node.

### 2.3 isMathML is detected from the template string by regex

The MathML flag is not carried through the AST. The plugin tests the template string against a fixed list of MathML tag names.

- Source: `plugin/src/dom/template.js:51-54`.
- Pinned by: `abi.json` arity of `template`; no upstream fixture family exercises MathML.
- Topcoat delta: the emitter knows its own element namespaces from the Rust AST and does not need a regex. It must produce the same answer for the same tag set. Because no fixture covers this, it is the weakest pinned point in this section.

### 2.4 The three boolean arguments are omitted when all false

`template(str)` is emitted with one argument unless at least one of the three flags is true.

- Source: `plugin/src/dom/template.js:56-74`.
- Pinned by: every fixture output, which shows bare `_$template(...)` calls.
- Topcoat delta: cosmetic only, since trailing `undefined` behaves identically. Matching upstream keeps emitted output diffable against the reference.

### 2.5 Unclosed tags are intentional and tolerated

Templates are emitted with closing tags omitted wherever the HTML parser can infer them. The whole string is assigned to `innerHTML`, so the browser's fragment parser closes them.

Fixture `__dom_hydratable_fixtures__/simpleElements/output.js` shows this directly, with templates such as `` `<div><span><a></a></span><span>` `` and `` `<div><noscript>` ``.

The `omitLastClosingTag` option defaults to `true`, and `omitNestedClosingTags` defaults to `false`.

- Source: `plugin/src/config.ts`; `plugin/src/dom/element.js:181-182`.
- Pinned by: `__dom_hydratable_fixtures__/simpleElements`; `contract/fixtures/templates.jsonl` case `unclosed-tag-tolerance`.
- Topcoat delta: none known. Omission is a size optimisation, not a semantic requirement, so an emitter may emit fully closed tags and still be correct. It must not omit a closing tag the parser cannot infer.

## 3. Walk semantics

### 3.1 There is exactly one walk form

Dynamic positions are reached by `.firstChild` for the first child and `.nextSibling` chained from the previous sibling thereafter. The compiler never re-walks from the parent.

`plugin/src/dom/element.js:1109-1120`:

```js
      const walk = t.memberExpression(
        t.identifier(tempPath),
        t.identifier(i === 0 ? "firstChild" : "nextSibling")
      );
```

`tempPath` is reassigned to the last emitted child id at `:1128`, which is what makes the chain sequential rather than indexed.

- Source: as quoted.
- Pinned by: `contract/fixtures/templates.jsonl` (`walk` field on all 51 cases); `__dom_fixtures__/textInterpolation/output.js:61-69`.
- Topcoat delta: none known. This is the single most mechanical part of the contract and the emitter must match it exactly, because the walk is computed against the compiler's tree but executed against the parser's.

### 3.2 Only nodes on the path to a dynamic position get an id

Whether a child is walked to at all is decided by `detectExpressions`. Static subtrees are skipped entirely.

- Source: `plugin/src/dom/element.js:1211` onward, called at `:1056`.
- Pinned by: `__dom_hydratable_fixtures__/simpleElements`, where fully static templates produce a single `getNextElement` call and no walk.
- Topcoat delta: none known. Emitting unnecessary walk variables is correct but wasteful; emitting too few is a bug.

### 3.3 Adjacent text nodes merge before walking

Consecutive text children are merged into one template chunk before the walk is computed, so they occupy one node position rather than several.

- Source: `plugin/src/dom/element.js:1060-1064`.
- Pinned by: `__dom_fixtures__/textInterpolation`.
- Topcoat delta: the emitter must apply the same merge. Rust source that produces two adjacent static text fragments must emit one text node, or every subsequent `nextSibling` in that list is off by one.

### 3.4 The html element is walked by tag match, not by position

When hydrating, children of `<html>` are reached with `getNextMatch(walk, tagName)` instead of a bare walk, because browsers inject `<head>` and `<body>` unpredictably.

`dom-expressions/src/client.js:282-285`:

```js
export function getNextMatch(el, nodeName) {
  while (el && el.localName !== nodeName) el = el.nextSibling;
  return el;
}
```

- Source: as quoted; emission at `plugin/src/dom/element.js:1101-1118`.
- Pinned by: `__dom_hydratable_fixtures__/document`.
- Topcoat delta: none known.

### 3.5 The tree oracle records the parser's answer, not the compiler's

`contract/fixtures/templates.jsonl` holds 51 template strings with the tree parse5 builds and the walk path to every node, generated by `contract/harness/gen-tree-oracle.mjs`. Ten are rejected by upstream's own validator and must be refused at compile time rather than walked.

The rejected cases are: implied `<tbody>` insertion, foster parenting out of a table, `<option>` auto-close, `<div>` dropped inside `<select>`, `<p>` auto-close by `<div>` and by `<p>`, `<li>` auto-close, `<dt>`/`<dd>` auto-close, nested `<form>` being ignored, and `<textarea>` containing a bare `<`.

- Source: parse5 7.3.0, the same major the plugin depends on.
- Pinned by: `contract/fixtures/templates.jsonl`.
- Topcoat delta: Topcoat must run an equivalent check. The oracle is the expected answer; see point 5.1 for the mechanism.

## 4. Placeholder and marker guards

### 4.1 Placeholders are emitted on two independent conditions

A `<!>` anchor or a `<!$>`/`<!/>` pair is emitted when hydration is on and the parent has more than one child, or when the expression sits between two text nodes.

`plugin/src/dom/element.js:1133-1136`:

```js
      const multi = checkLength(filteredChildren),
        markers = config.hydratable && multi;
      if (markers || wrappedByText(childNodes, index)) {
```

`checkLength` returns `i > 1` (`plugin/src/shared/utils.js:212-221`).

- Source: as quoted.
- Pinned by: `__dom_hydratable_fixtures__/insertChildren`; `contract/fixtures/markers.md`.
- Topcoat delta: none known.

### 4.2 Adjacent expressions share one anchor, except when hydrating

Consecutive expression children reuse a single `<!>` via the `nextPlaceholder` variable, which is cleared when a non-expression child intervenes. In the hydratable path there is no sharing: each expression gets its own marker pair, because the pair is what delimits its content.

- Source: `plugin/src/dom/element.js:1139-1144`, cleared at `:1129` and `:1171`.
- Pinned by: `__dom_fixtures__/textInterpolation/output.js:8`, template `` `<span> <!> ` `` for source `<span> {greeting}{name} </span>`.
- Topcoat delta: none known. Merging is an optimisation; emitting one anchor per expression is also correct in the non-hydratable case, but diverges from the reference output.

### 4.3 Marker syntax depends on who parses the bytes

`<!$>` and `<!/>` are bogus comments that only work because a parser expands them, so they are valid only inside template strings assigned to `innerHTML`. SSR writes `<!--$-->` and `<!--/-->` literally.

Full treatment, including the depth counting semantics of `getNextMarker` and the unrelated `<!--!$-->` array separator, is in `contract/fixtures/markers.md`.

- Source: `plugin/src/dom/element.js:1180-1181` and `plugin/src/ssr/element.js:500-509`; equivalence asserted at `plugin/src/shared/validate.js:31-33`.
- Pinned by: `contract/fixtures/markers.md`; `__dom_hydratable_fixtures__/insertChildren/output.js:12-13`.
- Topcoat delta: Topcoat renders SSR from Rust, so it must emit the long form server side and the short form in client templates. Getting this backwards produces markup that looks right and hydrates wrong.

## 5. The innerHTML round-trip invariant

### 5.1 Templates are validated by parse and re-serialise

The plugin maintains a second string, `templateWithClosingTags`, purely so it can check that the HTML parser does not restructure the markup. The check parses that string in `<body>` context and compares the serialisation case-insensitively.

`plugin/src/shared/validate.js:10-16` and `:88-95`:

```js
function innerHTML(htmlFragment) {
  const parsedFragment = parse5.parseFragment(bodyElement, htmlFragment);
  return parse5.serialize(parsedFragment);
}
...
  const browser = innerHTML(html);
  if (html.toLowerCase() !== browser.toLowerCase()) {
    return { html, browser };
  }
```

Before comparing, it normalises the short marker forms to long ones, replaces text nodes with `#text`, fixes entity escaping, and wraps orphan table fragments. Attributes are excluded from `templateWithClosingTags` entirely.

Gated by the `validate` option, default `true` (`plugin/src/shared/postprocess.js:22`).

- Source: as quoted; whole file is 96 lines.
- Pinned by: `contract/fixtures/templates.jsonl`, generated by driving this exact function through `contract/harness/load-plugin-validate.mjs`.
- Topcoat delta: Topcoat needs the same guard, implemented against a Rust HTML parser. The oracle fixture is the expected answer for 51 cases and is the intended acceptance test for that implementation. This is a real work item, not a no-op.

### 5.2 The innerHTML prop is never inlined into the template

An `innerHTML` prop is in `ChildProperties` and is always emitted as a runtime assignment, even for a literal value.

- Source: `dom-expressions/src/constants.js:80-85`; `plugin/src/dom/element.js:285-299`.
- Pinned by: `__dom_fixtures__/attributeExpressions/output.js:150`, which emits `_el$8.innerHTML = "<div/>";`.
- Topcoat delta: none known.

## 6. Reactivity wrapping

### 6.1 A single dynamic property gets a bare arrow

One dynamic binding compiles to a single `effect` with no record object. A `_$p` parameter is threaded only for `classList`, `style`, and `style:*`, which need their previous value.

- Source: `plugin/src/dom/template.js:123-155`.
- Pinned by: `__dom_fixtures__/attributeExpressions`.
- Topcoat delta: none known.

### 6.2 Multiple dynamic properties share one effect and a `_p$` record

Two or more dynamic bindings on the same element compile to one effect taking a `_p$` record, with a per key guard that skips the write when the value is unchanged.

`plugin/src/dom/template.js:197-211`:

```js
          t.logicalExpression(
            "&&",
            t.binaryExpression("!==", varIdent, propMember),
            setAttr(path, elem, key, t.assignmentExpression("=", propMember, varIdent), {...})
          )
```

The effect is seeded with an object literal whose every property is `undefined` (`:215-227`). `classList` and `style` differ: they assign the helper's return value into the record rather than the raw value (`:179-194`).

- Source: as quoted.
- Pinned by: `__dom_fixtures__/attributeExpressions`.
- Topcoat delta: none known. This is an optimisation, and a correct emitter could use one effect per binding. Doing so changes update ordering when several properties change in one tick, so matching upstream is safer.

### 6.3 Record keys use a base 54 frequency-ordered alphabet

The `_p$` record's property names come from a counter encoded in a fixed alphabet, not from the property names themselves.

`plugin/src/shared/utils.js:417-431`:

```js
const chars = "etaoinshrdlucwmfygpbTAOISWCBvkxjqzPHFMDRELNGUKVYJQZX_$";
const base = chars.length;

export function getNumberedId(num) {
  let out = "";

  do {
    const digit = num % base;

    num = Math.floor(num / base);
    out = chars[digit] + out;
  } while (num !== 0);

  return out;
}
```

The alphabet is 54 characters, all distinct: 52 letters in rough English frequency order, then `_` and `$`. So the base is 54, ids 0 to 53 are single characters (`e`, `t`, ... `_`, `$`) and id 54 rolls over to `te`. This heading said 53 until it was checked against the pinned source; nothing else depended on the number, but a base is exactly the kind of fact that gets copied.

- Source: as quoted, byte for byte including the blank lines.
- Pinned by: `__dom_fixtures__/attributeExpressions/output.js:183-197`, where the record is initialised as `{ e: undefined, t: undefined, a: undefined }` and read back as `_p$.e`, `_p$.t`, `_p$.a`.
- Topcoat delta: the record is private to one effect, so any unique key scheme is functionally correct. Matching upstream is only needed for output diffing.

### 6.4 Conditionals are wrapped in a memo when both test and branch are dynamic

With `wrapConditionals` on (default `true`), logical and conditional expressions become `memo`-wrapped so the branch is not re-evaluated when the test result is unchanged. Non-binary tests are double negated so the memo compares booleans.

Wrapping is skipped when the test is static or no branch is dynamic.

- Source: decision at `plugin/src/shared/transform.js:159-171`; implementation `plugin/src/shared/utils.js:270-343`.
- Pinned by: `__dom_hydratable_fixtures__/conditionalExpressions`.
- Topcoat delta: `wrapConditionals` is not exposed to the emitter as an option. Rust `if` and `match` in view position map onto this, and the memo is what prevents a whole subtree re-rendering when an unrelated signal changes, so the behaviour is required rather than optional.

## 7. Events

### 7.1 Twenty-two events are delegated

`dom-expressions/src/constants.js:189-212` defines exactly 22 delegated events: `beforeinput`, `click`, `dblclick`, `contextmenu`, `focusin`, `focusout`, `input`, `keydown`, `keyup`, `mousedown`, `mousemove`, `mouseout`, `mouseover`, `mouseup`, `pointerdown`, `pointermove`, `pointerout`, `pointerover`, `pointerup`, `touchend`, `touchmove`, `touchstart`.

- Source: as cited.
- Pinned by: `contract/fixtures/delegated-events.json`, which asserts the list rather than merely recording it; the extractor exits non-zero if upstream adds or removes one.
- Topcoat delta: none known.

### 7.2 Delegated handlers are properties, not listeners

A delegated handler is stored as a `$$name` property on the node, with an optional `$$nameData` companion for the bound-data form. No per node listener is registered.

`dom-expressions/src/client.js:136-146`:

```js
export function addEventListener(node, name, handler, delegate) {
  if (delegate) {
    if (Array.isArray(handler)) {
      node[`$$${name}`] = handler[0];
      node[`$$${name}Data`] = handler[1];
    } else node[`$$${name}`] = handler;
  } else if (Array.isArray(handler)) {
```

- Source: as quoted; registry at `:90-99` keyed by `_$DX_DELEGATE` (`:31`).
- Pinned by: `__dom_hydratable_fixtures__/eventExpressions`; `contract/fixtures/reference-traces/dom_hydratable_fixtures.eventExpressions.trace`.
- Topcoat delta: none known. The emitter must write the same property names, because dispatch looks them up by exactly that string.

### 7.3 Dispatch walks the composed path and retargets

`eventHandler` walks up from `composedPath()[0]`, calling any `$$name` property it finds, faking `currentTarget`, and stopping at the delegation root. It hops shadow boundaries via `.host` and portal boundaries via `_$host`.

`dom-expressions/src/client.js:438-458` shows the loop bound as `i < path.length - 2`, which skips the final `document` and `window` entries.

- Source: `dom-expressions/src/client.js:396-459`.
- Pinned by: no upstream fixture executes dispatch; this is pinned by source reading only.
- Topcoat delta: none known, but this is the least fixture-covered part of the contract. The emitter does not implement dispatch, since it uses the vendored runtime, so the risk is confined to any future decision to reimplement delegation in Rust.

### 7.4 The trailing delegateEvents call is module scoped

A single `delegateEvents([...])` call listing every delegated event used in the module is appended to the module body.

- Source: `plugin/src/shared/postprocess.js:11-20`.
- Pinned by: `__dom_hydratable_fixtures__/eventExpressions/output.js`.
- Topcoat delta: none known. Per-CGU emission must union the events across the whole module, not per function.

## 8. Hydration

### 8.1 data-hk is written by the SSR runtime, never by the compiler

The plugin never emits `data-hk`. Its only mention is a warning that removes a hand written one. The attribute is written at SSR runtime by `ssrHydrationKey()`.

`dom-expressions/src/server.js:421-424`:

```js
export function ssrHydrationKey() {
  const hk = getHydrationKey();
  return hk ? ` data-hk="${hk}"` : "";
}
```

- Source: as quoted; rejection at `plugin/src/dom/element.js:126-142`.
- Pinned by: `__ssr_hydratable_fixtures__/simpleElements`.
- Topcoat delta: Topcoat renders SSR from Rust, so Topcoat owns this emission. It must produce keys from the same allocator as point 9.1 or the client will not find its nodes.

### 8.2 Keys are on template roots only

The key is emitted only for the top level element of a template, not for every element. Everything below the root is reached by the walk.

- Source: `plugin/src/ssr/element.js:64` and `:86-89`, gated on `info.topLevel && config.hydratable`; client consumer at `plugin/src/dom/template.js:103-111`.
- Pinned by: `__ssr_hydratable_fixtures__/simpleElements` and `__dom_hydratable_fixtures__/simpleElements`.
- Topcoat delta: none known.

### 8.3 The client finds roots through a registry, not a query per node

`hydrate()` calls `gatherHydratable` once to build a `Map` from key to node, then `getNextElement` does a map lookup and deletes the entry.

- Source: `dom-expressions/src/client.js:244-262` and `:264-280`; gather at `:605-613`.
- Pinned by: `contract/fixtures/reference-traces/dom_hydratable_fixtures.simpleElements.trace`, which records `getNextElement` where the non-hydratable trace records `template.clone`.
- Topcoat delta: none known.

### 8.4 getNextMarker counts depth over sibling comments

Detailed in `contract/fixtures/markers.md`. The essentials: a flat forward scan over siblings, `$` pushes depth, `/` pops, the `/` at depth zero is the partner, and the returned node list excludes the closing marker.

- Source: `dom-expressions/src/client.js:287-306`.
- Pinned by: `contract/fixtures/markers.md`; `__dom_hydratable_fixtures__/insertChildren`.
- Topcoat delta: none known.

### 8.5 Hydration suppresses attribute writes

`setProperty`, `setAttribute`, `className`, and `setBoolAttribute` all early-return while hydrating, because the server already wrote the value. The gate is `isHydrating`.

- Source: `dom-expressions/src/client.js:330-332`, consulted at `:109`, `:114`, `:120`, `:126`, `:131`.
- Pinned by: source reading, and now by execution: `contract/parity` hydrates the counter island in a live DOM and observes zero mutations, which no attribute write could survive.
- Topcoat delta: none known. Worth stating because it means server and client must agree on attribute values exactly. A value that differs is not corrected on hydration, it is silently kept from the server.

### 8.7 Hydration suppresses content writes too, for every value type

`insertExpression` is gated the same way `setAttribute` is, and more thoroughly. Its first act is to read `isHydrating(parent)`, and every branch that would write content returns early when that is true: strings and numbers at `:480`, `null` and booleans at `:497`, arrays at `:513`, and a node value at `:535`.

`dom-expressions/src/client.js:478-480`:

```js
if (t === "string" || t === "number") {
    if (hydrating) return current;
```

The consequence is the one that matters for an emitter. **A dynamic text hole costs zero DOM mutations on hydration even when the node range handed to `insert` is wrong**, because nothing is written at all. A wrong range is therefore invisible at hydration time and shows up on the first update instead, when `isHydrating` is false and the stale range is finally used. An emitter cannot conclude from a clean hydration that its `getNextMarker` call was right.

The hydrating prologue at `:462-471` also removes any sibling comment whose data begins with `!$`. That is the placeholder syntax of point 4.3, not the `$` and `/` marker pair, so ordinary markers survive hydration.

- Source: as quoted; `isHydrating` at `:330-332`.
- Pinned by: `contract/parity`, which observes zero mutations across the hydrate bracket and three on the first update, against an emitter whose claimed range is known to be wrong.
- Topcoat delta: none in the runtime. It changes what a Topcoat emitter can be tested for: hydration parity and update correctness are separate claims and need separate assertions.

### 8.6 runHydrationEvents replays captured events

Events captured by the bootstrap script before hydration are replayed in order, stopping at the first element not yet marked complete.

- Source: `dom-expressions/src/client.js:308-327`.
- Pinned by: `contract/fixtures/hydration-script.js`.
- Topcoat delta: none known.

## 9. Key allocation

### 9.1 Keys are a prefix-free letter plus a decimal count

`solid/dist/solid.js:133-137`:

```js
function getContextId(count) {
  const num = String(count),
    len = num.length - 1;
  return sharedConfig.context.id + (len ? String.fromCharCode(96 + len) : "") + num;
}
```

The letter encodes the digit length of the number that follows: one digit gets no letter, two digits get `a`, three get `b`, and so on. That is what makes concatenated segments unambiguously separable, which in turn is what makes the `startsWith` island filter in point 11.2 sound.

`getNextContextId()` returns the key for the current count then post-increments. `getContextId()` peeks without incrementing.

- Source: as quoted; `sharedConfig` at `:121-132`.
- Pinned by: `contract/fixtures/keys.json`, 264 driven triples plus 9 asserted boundary checks.
- Topcoat delta: Topcoat must reimplement this exactly in Rust. The fixture is the acceptance test. Any divergence produces keys the client runtime cannot match.

### 9.2 Child contexts consume a parent key and reset their counter

`nextHydrateContext()` builds a child context whose `id` is the parent's next key and whose `count` restarts at zero, which is what makes ids hierarchical.

`solid/dist/solid.js:141-147`:

```js
function nextHydrateContext() {
  return {
    ...sharedConfig.context,
    id: sharedConfig.getNextContextId(),
    count: 0
  };
}
```

Driven by `createComponent` (`solid/dist/solid.js:1274-1285`), so every component invocation opens a new key namespace.

- Source: as quoted.
- Pinned by: `contract/fixtures/keys.json` field `nesting`; the full nesting oracle is `contract/fixtures/keys-nested.json` (points 9.5 to 9.9).
- Topcoat delta: Topcoat must open a child context per component call, matching solid's placement. Opening one in a different place shifts every key beneath it.

### 9.3 sharedConfig.count is not the key counter

`sharedConfig.context.count` is the per-scope key counter. `sharedConfig.count` is a separate global counting in-flight async hydration units, used to gate the effect queue. They never interact, and conflating them is an easy and severe mistake.

- Source: `solid/dist/solid.js:904-915` (queue gate) and `:1440-1444` (lazy increments).
- Pinned by: recorded here only; no fixture.
- Topcoat delta: none known. Topcoat implements no async hydration, so `sharedConfig.count` stays zero.

### 9.4 The root context id is the renderId or empty string

- Source: `dom-expressions/src/client.js:253`; `dom-expressions/src/server.js:37-42`.
- Pinned by: `contract/fixtures/keys.json`, which includes the empty context id case; `contract/fixtures/keys-nested.json` group `renderId` drives five root ids through a nested tree, including the id `"0"`, which is indistinguishable from a count and must still work because the island filter is a `startsWith` (point 11.2).
- Topcoat delta: none known.

### 9.5 A component boundary spends one parent slot, and the spend outlives the component

This is the point Topcoat's `Formatter::enter_component` has to get exactly right, and it is the one with a silent failure mode: get it wrong and every key underneath a component is plausible, self-consistent, and unmatchable.

Under hydration, `createComponent` does four things in order (`solid/dist/solid.js:1274-1285`, and the same shape in the SSR build at `solid/dist/server.js:398-407`):

```js
function createComponent(Comp, props) {
  if (hydrationEnabled) {
    if (sharedConfig.context) {
      const c = sharedConfig.context;
      setHydrateContext(nextHydrateContext());
      const r = untrack(() => Comp(props || {}));
      setHydrateContext(c);
      return r;
    }
  }
  return untrack(() => Comp(props || {}));
}
```

`nextHydrateContext` calls `getNextContextId`, which post-increments `this.context.count`, so the parent context object saved in `c` is mutated in place before the child is built. The restore on the way out puts back that same mutated object, not a snapshot. A component therefore costs exactly one slot of its parent's counter, the key of that slot becomes the child context's id, and the sibling after the component continues from the next number.

The whole scheme in Rust terms:

```text
context = (id: String, count: u32)

key(ctx)  = ctx.id + letter(digits(ctx.count) - 1) + ctx.count ; ctx.count += 1
letter(0) = ""  letter(n) = b'a' + n - 1

enter_component() -> saved:
    child_id     = key(parent)      // SPENDS the parent's slot
    push (child_id, 0)
exit_component(saved):
    pop, restoring the parent whose count has already advanced
```

The trap is `exit_component`. Saving a copy of the parent context and restoring the copy replays the component's slot for the next sibling, which shifts every later key on the page by one. The oracle catches this: fixture case `siblings/two-components` is `["00", "10"]`, and a copy-restoring implementation produces `["00", "00"]`.

- Source: as quoted; `nextHydrateContext` at `solid/dist/solid.js:141-147`.
- Pinned by: `contract/fixtures/keys-nested.json`, 102 driven cases and 11 asserted boundary checks. The relevant checks are named `a component spends exactly one parent slot; the next sibling skips it` and `the spend survives the restore for every sibling`.
- Topcoat delta: Topcoat's client side gets this for free by emitting `_$createComponent(fn, props)` for a component hole, which is the function above. The server side has to implement it, and the two must agree; see point 9.7.

### 9.6 The letter comes from the counter of the context the boundary opens in

The digit-length letter of point 9.1 applies at a component boundary exactly as it does at an element, because a boundary allocates through the same `getNextContextId`. Two independent counters are in play and each gets its own letter.

A component opened as the eleventh slot of the root context has id `a10`, so its first element key is `a100`. A component opened as the eleventh slot of a child context whose id is already `0` has id `0a10`, so its first element key is `0a100`. Both counters can cross at once: fixture case `one-component/pad10-inner11` ends `..., "a10a10", "a11"`, where the first is the eleventh element inside the component that was itself the eleventh slot of the root, and the second is the element after the component.

The letter is not derived from depth and not derived from the child index in a node list. It is derived from the numeric value of the counter of the context doing the allocating, and nothing else.

- Source: `solid/dist/solid.js:133-137` reached through `:144`.
- Pinned by: `contract/fixtures/keys-nested.json` groups `boundary` (parent counters 9, 10, 99, 100, 999, 1000, 10000) and `inner-boundary` (child counters 9, 10, 11, 99, 100), plus the check `an inner component's id crosses the CHILD context's letter boundary`.
- Topcoat delta: none known. Topcoat must reuse one key-encoding routine for both element roots and component boundaries rather than special casing either.

### 9.7 The server and the client run the same allocator, which makes the oracle two sided

Solid ships two copies of this code, one per build, and they agree on keys while differing in their guards. The client gates nesting on `enableHydration()` having run and on a live `sharedConfig.context`. The server gates it on `sharedConfig.context && !sharedConfig.context.noHydrate`, so `NoHydration` (point 11.3) stops both the keys and the nesting. `getContextId` itself is byte identical between the two.

That agreement is the actual contract: the server writes `data-hk` attributes and the client looks up the same strings in the registry it gathered from them (`dom-expressions/src/client.js:605-613`). An oracle pinning only one side could be satisfied by a scheme that never hydrates, so `extract-nested-keys.mjs` drives both, per case, and compares the client's registry lookup order against the `data-hk` order in solid's real SSR output.

- Source: `solid/dist/server.js:369-407` against `solid/dist/solid.js:121-147` and `:1274-1285`; `dom-expressions/src/server.js:522-525` and `:421-424` against `dom-expressions/src/client.js:615-617` and `:264-280`.
- Pinned by: `contract/fixtures/keys-nested.json` field `clientServerAgreement`, and the per case `elementKeysSha256`. Small cases also store the server's exact HTML in `ssr`, which is what a Topcoat SSR test can diff against.
- Topcoat delta: Topcoat has the same two sided obligation, with the sides much further apart: keys are produced by a Rust `Formatter` and consumed by compiled JS. The parity harness is where the two meet.

### 9.8 A component that renders nothing hydratable still spends a slot

The slot is spent at the boundary, before the component body runs, so a component that emits no template root still shifts its siblings. Fixture case `shape/empty-components-then-element` is three empty components followed by an element, and the element's key is `3`.

- Source: the ordering in `solid/dist/solid.js:1277-1279`.
- Pinned by: `contract/fixtures/keys-nested.json` check `an EMPTY component still spends a parent slot`.
- Topcoat delta: Topcoat must open and close the context around a component call unconditionally, not lazily on the first key allocated inside it.

### 9.9 Nesting does not stop after a hydration mismatch

`createComponent` checks `sharedConfig.context` but never `sharedConfig.done`, so once `getNextElement` has set `done` (`dom-expressions/src/client.js:269-274` in dev builds, or `:436` after the first delegated event) contexts keep nesting even though key lookups have stopped. Only `hydrate`'s `finally` clears the context (`:259-261`).

- Source: as cited.
- Pinned by: recorded here only; no fixture, because the observable behaviour after a mismatch is a hard failure either way.
- Topcoat delta: none known. Worth recording because a Rust implementation that skips the nesting when a mismatch flag is set would diverge from upstream in a state that is already unrecoverable, which makes the divergence hard to see and pointless to have.

## 10. Escaping

### 10.1 solid escapes exactly two characters per mode

`solid/web/dist/server.js:544` selects the delimiter by mode and escapes it plus `&`. Text mode escapes `<` and `&`. Attribute mode escapes `"` and `&`. Nothing else is touched.

`>` and `'` are never escaped in either mode. `escape()` is not idempotent: applying it twice turns `&` into `&amp;amp;`.

- Source: `dom-expressions/src/server.js:426-478`; compile-time twin `escapeHTML` with the identical character set at `plugin/src/shared/utils.js:345-388`.
- Pinned by: `contract/fixtures/escaping.json`, 47 driven cases and 12 asserted invariants.
- Topcoat delta: **diverges in text mode.** `crates/topcoat-view/src/escape.rs` escapes `&`, `<`, and `>` in `HtmlContext::Text` (`TEXT_ESCAPES`). solid escapes only `&` and `<`. Topcoat's set is a strict superset, so it is never unsafe, but SSR text output containing `>` is not byte identical to solid's. If Rust renders the server pass while solid's runtime performs client-side text updates, a `>` in dynamic text yields `&gt;` from the server and `>` from the client, which is a hydration text mismatch. This needs a decision: either narrow Topcoat's text escaping for values that solid will later rewrite, or ensure no dynamic text position is ever updated by solid's escaper. Attribute mode matches exactly.

### 10.2 Topcoat has escaping contexts solid does not

Topcoat additionally has `HtmlContext::Comment` (escapes `&`, `>`, `"`) and the ident contexts `AttributeKey` and `ElementName`, which validate and panic rather than escape. solid has no equivalent and does not guard those positions.

- Source: `crates/topcoat-view/src/escape.rs`.
- Pinned by: that file's own test module.
- Topcoat delta: additive. No conflict, since solid never writes into those positions.

### 10.3 solid's escape doubles as a value coercion point

`escape()` handles non-strings: in text mode it calls functions and maps arrays, and in attribute mode it stringifies booleans. Topcoat escapes only `&str`.

- Source: `dom-expressions/src/server.js:426-438`.
- Pinned by: `contract/fixtures/escaping.json` field `nonStringPassthrough`.
- Topcoat delta: Topcoat performs coercion in the type system before formatting, so there is nothing to reproduce. Recorded because an emitter that routes values through a JS escaper would inherit this behaviour unintentionally.

## 11. Islands and partial hydration

### 11.1 The _$HY bootstrap shape is fixed and partly inert

The bootstrap installs `_$HY = { events: [], completed: new WeakSet, r: {}, fe(){} }`. `r` is the resource registry, empty without streaming, and `fe` is a no-op flush hook. Both are inert here but **must exist**: the client reads `_$HY.r` unconditionally and calls `_$HY.fe`.

There is no `done` field in the initial shape. It is assigned later by the client at `dom-expressions/src/client.js:436`.

The script ends with a `<!--xs-->` marker, which is the injection anchor for later serialization scripts.

- Source: `dom-expressions/src/server.js:538-544`.
- Pinned by: `contract/fixtures/hydration-script.js`, extracted by driving the real function with a sentinel and proved to round-trip byte identically for four event lists.
- Topcoat delta: Topcoat must emit this script server side. The extracted artifact is directly usable.

### 11.2 Island scoping is a startsWith filter on keys

`gatherHydratable` admits a node to the registry only when its key starts with the given root prefix, which is what scopes hydration to one island.

`dom-expressions/src/client.js:605-613`:

```js
function gatherHydratable(element, root) {
  const templates = element.querySelectorAll(`*[data-hk]`);
  for (let i = 0; i < templates.length; i++) {
    const node = templates[i];
    const key = node.getAttribute("data-hk");
    if ((!root || key.startsWith(root)) && !sharedConfig.registry.has(key))
      sharedConfig.registry.set(key, node);
  }
}
```

This is only sound because the key encoding is prefix-free (point 9.1).

- Source: as quoted.
- Pinned by: recorded here; no fixture exercises islands.
- Topcoat delta: none known.

### 11.3 NoHydration suppresses key emission server side

`NoHydration` sets `context.noHydrate`, which makes `getHydrationKey` return falsy so `ssrHydrationKey()` yields an empty string. `Hydration` mints a fresh island scope by consuming a parent key and resetting the counter. On the client `NoHydration` returns nothing while hydrating.

The plugin auto-injects `NoHydration` around `<head>` in hydratable mode.

- Source: `dom-expressions/src/server.js:546-563`; client at `client.js:619-625`; head injection at `plugin/src/dom/element.js:143-162`.
- Pinned by: `__ssr_hydratable_fixtures__/document`.
- Topcoat delta: none known.

### 11.4 The default bootstrap event list is not the delegated set

`generateHydrationScript` defaults `eventNames` to `["click", "input"]`, only two events. The 22 delegated events are a different and larger set serving a different purpose: the bootstrap captures events before hydration, delegation dispatches them after.

An empty `eventNames` does not produce `[]`. Because the quotes are part of the template, an empty join yields `[""]`, registering a listener for the empty-string event name.

- Source: `dom-expressions/src/server.js:538-544`.
- Pinned by: `contract/fixtures/hydration-script.js` and `hydration-script.meta.json`; the empty-list quirk was found by the extractor's round-trip check.
- Topcoat delta: Topcoat must choose the list deliberately. Capturing all 22 costs 22 document listeners during the pre-hydration window; capturing 2 loses the rest.

## 12. The rxcore seam

### 12.1 Nothing needs wiring

`dom-expressions/src/client.js:10-18` imports `root`, `effect`, `memo`, `getOwner`, `createComponent`, `sharedConfig`, `untrack`, and `mergeProps` from the bare specifier `rxcore`, which is a build-time alias rather than a package. Six of those are re-exported to consumers (`:33-40`).

`solid-js/web` is that module with the alias already resolved to solid's core. Because the vendored runtime bundles `solid-js/web`, the seam is satisfied inside the artifact and there is nothing for Topcoat to wire.

- Source: as cited; upstream's own test alias at `plugin/../dom-expressions/babel.config.js`.
- Pinned by: `contract/fixtures/abi.json` field `rxcore`; `runtime/dom/check-exports.mjs`.
- Topcoat delta: none, and none expected. This is recorded to close the question, not because work is required.

## 13. Deferred shapes

Recorded so their existence is known and their entry points are findable. **Nothing here is implemented, and nothing here should be implemented as part of this milestone.**

### 13.1 Suspense, resources, and lazy

`Suspense` gates a fallback and defers effects (`solid/dist/solid.js:1636`; SSR `solid/dist/server.js:699-760`). `createResource` seeds from `_$HY.r` during hydration (`solid/dist/solid.js:259`, read at `:292-294` and `:1657-1669`). `lazy` drives the global `sharedConfig.count` async gate (`:1433`).

- Topcoat delta: out of scope. Topcoat has no async boundary primitive today.

### 13.2 Portal

Renders into a detached container mounted elsewhere and sets `_$host` on it so event delegation can bubble back to the logical tree (`solid/web/dist/web.js:712-750`, `_$host` handling at `dom-expressions/src/client.js:425` and `:444-448`).

- Topcoat delta: out of scope.

### 13.3 seroval serialization

`createSerializer` with `globalIdentifier: "_$HY.r"` and 12 default web plugins (`dom-expressions/src/serializer.js:1-54`). This is how resource values cross the server/client boundary.

- Topcoat delta: out of scope. Topcoat has no equivalent and would need a Rust side serializer to adopt it.

### 13.4 Streaming SSR

`renderToStream` and its fragment machinery (`dom-expressions/src/server.js:75-302`), the `$df(id)` replace script (`:21`, emitted at `:211`), and placeholder splicing (`:602-611`).

- Topcoat delta: out of scope.

### 13.5 Assets and request context

`useAssets`, `getAssets`, `Assets` inject into `<head>` before `</head>` (`dom-expressions/src/server.js:527-536`, `:577-584`). `getRequestEvent` and `RequestContext` are marked experimental upstream (`:613-624`). All are `voidFn` on the client (point 1.3).

- Topcoat delta: out of scope. Topcoat has its own asset system (`topcoat-asset`) that solves the same problem differently, so this is unlikely ever to be adopted.

## 14. Async, and what an executor must respect

Recorded for the async-vs-callback decision the backend has to make, and for whatever
runs a Rust `Future` inside a compiled island. Unlike point 13, this is not deferred
work: it is the set of constraints the pinned runtime imposes on ANY asynchrony,
whether or not Topcoat adopts `Suspense`.

Read the whole section before choosing an executor shape. Points 14.6 and 14.8 are
the ones that bite silently.

### 14.1 The pinned runtime contains no async/await syntax at all

Zero occurrences of `async function`, `async (`, `async x =>` or `await ` across
`solid-js/dist/*.js`, `solid-js/web/dist/*.js`, `solid-js/store/dist/*.js`,
`solid-js/universal/dist/*.js`, `solid-js/html/dist/*.js`, `solid-js/h/dist/*.js`
and `dom-expressions/src/*.js`. The runtime is synchronous control flow plus
explicit `.then()` callbacks. There is also no `setTimeout`, no
`requestAnimationFrame` and no `requestIdleCallback`; the one scheduler is
`MessageChannel` based (`solid/dist/solid.js:14-53`, `setupScheduler`) and is
dormant unless `enableScheduling()` runs (`:528-530`; the `Scheduler` global is
`null` at `:171`). The only microtask in the DOM layer is `runHydrationEvents`
(`dom-expressions/src/client.js:310`).

Two apparent hits are not hits: `dom-expressions/src/constants.js:4` and
`solid/web/dist/web.js:4` contain the STRING `"async"` in the boolean-attribute
table, for `<script async>`.

- Topcoat delta: none, and this is the good news. An executor does not have to
  interoperate with awaits inside the runtime, because there are none. Everything
  below is about what the runtime does to code that suspends, not about the runtime
  suspending.

### 14.2 An effect's return value is stored and replayed as its next argument

`runComputation` (`solid/dist/solid.js:701-736`) calls `nextValue = node.fn(value)`
and then stores it (`:726-735`). For an effect the store lands on
`else node.value = nextValue` (`:733`), because `createComputation` builds the node
without an `observers` key while `createMemo` adds one explicitly (`:247`). The
stored value is read back as the seed on the next invalidation (`:689`).

So an effect body that returns a Promise has that Promise object become
`node.value`, and on the next run it arrives as the `prev` argument. The runtime
never inspects it, never calls `.then` on it, and never checks `isPromise` on it:
`isPromise` (`:256-258`) is used only by `createResource`.

- Topcoat delta: a Rust `async` closure lowered to a JS async function and handed to
  `createEffect` type-checks and runs, and its Promise silently becomes the effect's
  value. Nothing reports it. If the executor's shape lets that happen, the emitter
  should reject it instead.

### 14.3 A tracking context does not survive a suspension point

`Listener` and `Owner` are module-level mutable globals (`solid.js:169`, `:173`),
set at `:705` and restored in a `finally` at `:722-725`. For an async function that
`finally` fires at the FIRST suspension point, not at completion, so a continuation
resuming in a later microtask sees whatever the synchronous unwind left behind, which
for a top-level `render` is `null`:

```js
Listener = Owner = node;
try {
  nextValue = node.fn(value);
} finally { Listener = listener; Owner = owner; }
```

Consequences after the gap, each a silent one in the production build:

- a signal read does not subscribe: `readSignal` gates all dependency wiring on
  `if (Listener)` (`:629`);
- `onCleanup` is a no-op: `:488-491` reads `if (Owner === null) ;`, with no warning;
- `useContext` returns the default (`:575-578` reads `Owner && Owner.context`);
- an effect or memo created in the continuation is unowned and never disposed
  (`createComputation`, `:755`);
- `getOwner()` returns `null` (`:511-513`), so the capture-the-owner idiom is only
  available to code that called it BEFORE suspending.

`startTransition` is the one place upstream restores these across a microtask
(`:531-558`: captures at `:536-537`, re-installs inside `Promise.resolve().then` at
`:539-540`, nulls them at `:555`). `runWithOwner` (`:514-527`) saves and restores
`Owner` and `Listener` and nothing else, in a synchronous `finally`.

- Topcoat delta: any Rust async block that touches the reactive graph must capture
  the owner before its first suspension point and re-enter through `runWithOwner`.
  That is a requirement on the EXECUTOR, not on user code, because a Rust `async fn`
  gives the user no place to put the capture.

### 14.4 A rejected Promise is invisible to the runtime's error handling

The only error path is the synchronous `catch` in `runComputation`
(`solid.js:708-721`) into `handleError`. `catchError`'s `try/catch` is likewise
synchronous (`:492-507`), and `ErrorBoundary` (`:1543`) is built on it. A returned
rejected Promise reaches none of them: it becomes a host-level unhandled rejection.

- Topcoat delta: an executor must not route a Rust `Err` through rejection and hope
  a boundary catches it. Either deliver the error synchronously inside the poll, or
  hand it somewhere explicit. There is no upstream safety net.

### 14.5 A batch, and the effect queues, close at the first suspension point

`batch` is `runUpdates(fn, false)` (`solid.js:452-454`). `runUpdates` (`:816-831`)
allocates the `Effects` and `Updates` arrays, runs `fn()`, and calls
`completeUpdates` on the next line, synchronously. `completeUpdates` (`:832-876`)
drains the queues and nulls `Effects` (`:872-875`). `runEffects` starts as `runQueue`
(`:159`) and is swapped to `runUserEffects` by the first `createEffect` (`:223`);
`runQueue` is a plain `for` loop (`:877-879`).

So an async function passed to `batch` is batched only up to its first suspension
point, and a `createEffect` called from a continuation finds `Effects` already
`null`, takes the `updateComputation(c)` branch (`:228`), and runs immediately and
unbatched.

- Topcoat delta: an executor that resumes futures in microtasks gets no batching for
  anything after the first suspension. Two signal writes in a continuation are two
  update passes.

### 14.6 Hydration's window is exactly one synchronous call stack

`sharedConfig` is a single module-level object literal (`solid.js:121-132`) that
`web.js` imports (`web.js:1`), so there is one instance process wide. `hydrate`
clears the context in a synchronous `finally`
(`dom-expressions/src/client.js:256-261`, prod `web.js:357-366`):

```js
try {
  gatherHydratable(element, options.renderId);
  return render(code, element, [...element.childNodes], options);
} finally {
  sharedConfig.context = null;
}
```

`render` itself (`client.js:48-65`) has no async accommodation. So a continuation
resuming after the bracket sees `sharedConfig.context === null`, which makes
`isHydrating()` false (`client.js:330-332`) and sends `getNextElement` down
`return template()` (`client.js:275`, `web.js:373`): it BUILDS FRESH DOM instead of
adopting the server's. In the development build a mismatch throws
(`client.js:269-274`); that branch is compiled out of production, and `web.js:368-378`
has no equivalent, so in production this is entirely silent.

Five further breakages after the gap, all cited: key allocation desynchronises
because `getNextContextId` mutates a context object that has been discarded
(`solid.js:130`); the registry has already been consumed entry by entry
(`client.js:278` deletes each key as it is claimed) and only `Suspense` re-gathers
(`client.js:250`, called at `solid.js:1670`); nodes reached after the gap are never
added to `completed`, because `client.js:277` is unreachable once `isHydrating` is
false; `sharedConfig.count` cannot hold the window open because nothing but `lazy`
increments it (`:1440-1441`), so `runUserEffects` clears the context at `:910` and
flushes; and `onCleanup` after the gap is a no-op (`:489`), so the island leaks.

- Topcoat delta: **an island's setup must be synchronous.** Anything else degrades
  to client rendering without a word in production, which is the exact failure class
  the parity harness exists to catch. If an island ever needs async setup, it needs
  14.7's machinery, not an `async fn`.

### 14.7 The only sanctioned way to hold a hydration window open is open-coded twice

`lazy` (`solid.js:1433-1467`) and `Suspense` (`:1636-1726`) both do the same thing by
hand: capture `sharedConfig.context` into a local BEFORE the async call, maintain the
`sharedConfig.count` outstanding-work counter, and re-install with
`setHydrateContext(ctx)` inside the `.then` while checking `sharedConfig.done`:

```js
(p || (p = fn())).then(mod => {
  !sharedConfig.done && setHydrateContext(ctx);
  sharedConfig.count--;
  set(() => mod.default);
  setHydrateContext();
});
```

Note that it restores to `undefined` (a bare `setHydrateContext()`), not to whatever
was there before. `runUserEffects` consults `count` to defer user effects until the
async work settles (`:904-916`). `sharedConfig.count` is written ONLY by `lazy`
(`:1440-1441`), so nothing else in the runtime can register outstanding async work,
and there is no general-purpose helper for any of this.

`startTransition` is instructive by omission: it restores `Listener` and `Owner`
across a microtask and does NOT touch `sharedConfig` (`:531-558`). Restoring the
reactive globals and restoring the hydration context are separate obligations.

- Topcoat delta: adopting async hydration means open-coding this a third time, in
  Rust, including the `done` check and the counter. Out of scope for this milestone
  (point 13.1), but the shape is now recorded rather than guessed at.

### 14.8 A delegated event during an async gap ends hydration permanently

`eventHandler` cancels hydration on the first delegated event
(`dom-expressions/src/client.js:436`, prod `web.js:508`), with that comment in the
source:

```js
// cancel hydration
if (sharedConfig.registry && !sharedConfig.done) sharedConfig.done = _$HY.done = true;
```

`_$HY.done` starts falsy: the server's bootstrap emits
`_$HY={events:[],completed:new WeakSet,r:{},fe(){}}` and sets no `done`
(`dom-expressions/src/server.js:538-544`). Once it is true, `hydrate` degrades to a
plain client render on entry (`client.js:245`, `web.js:350`).

`runHydrationEvents` (`client.js:308-327`) replays the buffer in a microtask and
**bails on the first event whose target is not yet in `completed`** (`:316`), which
stalls the whole queue behind one unhydrated node. Replaying a buffered event is
itself what sets `done`.

- Topcoat delta: a user click landing in an async gap does not merely lose a frame,
  it cancels hydration for the rest of the document. This is the strongest single
  argument for keeping island setup synchronous (14.6).

### 14.9 Promise detection upstream is a property test, not a callable test

`isPromise` is `v && typeof v === "object" && "then" in v` (`solid.js:256-258`). Not
`typeof v.then === "function"`, not `instanceof Promise`. Any thenable passes,
including one whose `then` is inherited or not callable, which then throws a
TypeError where it is called (`:381`).

`createResource` accepts three fetcher return shapes (`:353-381`): a non-thenable,
resolved synchronously (`:366-369`); a settled marker carrying `v`/`s`, checked
BEFORE `.then` so an already-streamed value skips the microtask (`:371-374`); and a
thenable (`:381`). Rejections go to `loadEnd(..., castError(e), ...)` and surface
only on read, and only when nothing is in flight (`:328`: `if (err !== undefined && !pr) throw err`).
A superseded request is dropped by the staleness guard at `:297` (`if (pr === p)`).
Reading a pending resource outside `Suspense` neither throws nor suspends: it returns
`value()`, i.e. the initial value (`:324-341`).

- Topcoat delta: whatever surrogate represents a Rust `Future` on the JS side must
  not carry a `then` property unless it really is a thenable, or `createResource`
  will mistake it for one. Topcoat's existing browser runtime already gets this
  right, and its shape is worth keeping:
  `crates/topcoat-runtime/browser/src/surrogate/future.ts:1-23` models a Rust future
  as a LAZY `PromiseLike` whose thunk runs on the first `then` and is memoised
  afterwards, which is Rust's own poll-when-driven semantics rather than a Promise's
  run-on-construction semantics.

### 14.10 A Promise inserted into the DOM is silently dropped

`insertExpression` (`client.js:461-546`) branches on `string`/`number`,
`null`/`boolean`, `function`, `Array` and `value.nodeType`, then falls off the end at
`:543` into a development-only `console.warn("Unrecognized value. Skipped inserting")`.
The production `web.js` has no such branch at all.

- Topcoat delta: returning a future from a view hole renders nothing and reports
  nothing in production. The emitter has to make that unrepresentable rather than
  relying on a warning that is not there.

### 14.11 There is no cancellation anywhere in the pin

`AbortController`, `AbortSignal` and `abort` do not appear in `solid.js`, `web.js`,
`store.js` or `client.js`. `createResource` passes only `{ value, refetching }` to
the fetcher (`:355-358`). Its `onCleanup` (`:286-291`) nulls the local `pr`, which
makes the eventual `loadEnd`'s `pr === p` check fail, so the in-flight request is
IGNORED and not cancelled: the network request continues. That cleanup is also
guarded by `if (Owner)`, so a resource created outside an owner has none at all.

- Topcoat delta: dropping a Rust future is expected to cancel its work, and the
  pinned runtime offers no counterpart. An executor that maps drop onto anything has
  to build it, and the `fetch` a procedure call issues (point 3 of PROCEDURES.md) is
  the first thing that will want it.

## Open questions

Points the contract records but does not settle, in rough priority order.

1. **The text-escaping divergence (10.1) needs a decision.** It is the only identified behavioural conflict between an existing Topcoat implementation and the pinned runtime.
2. **MathML detection (2.3) has no fixture coverage.** No upstream fixture family exercises it, so the pin rests on source reading alone.
3. **Event dispatch (7.3) has no fixture coverage.** Acceptable while Topcoat uses the vendored runtime; it becomes a risk the moment delegation is reimplemented in Rust.
4. **The template validator (5.1) is a real work item.** `templates.jsonl` supplies the expected answers for 51 cases, but a Rust HTML parser has to produce them.
5. **The executor shape is not settled, and 14.6 constrains it.** An island's setup
   must be synchronous or hydration silently degrades to client rendering, so
   whatever runs a Rust `Future` inside an island has to keep suspension out of the
   hydration bracket. The facts are in point 14; the decision is the backend's.
