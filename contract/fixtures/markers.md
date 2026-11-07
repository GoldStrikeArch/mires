# Markers: `<!$>` / `<!/>` versus `<!--$-->` / `<!--/-->`

Extracted from dom-expressions 0.40.8 at
`ryansolid/dom-expressions@56bd4054237b031c0cca8c1ec69da162c52ed4a6`.

## The short answer

They are **the same two markers written in two syntaxes**, chosen by where the
markup is produced:

| form | written by | why |
|---|---|---|
| `<!$>` `<!/>` `<!>` | the **client** template string | shorter; the HTML parser expands them |
| `<!--$-->` `<!--/-->` | the **SSR** output stream | must be literal, nothing re-parses it into existence |

Both produce a comment node whose `nodeValue` is `"$"` or `"/"`. The runtime
only ever reads `nodeValue`, so it cannot tell them apart, and must not.

## Why the client form works: bogus comments

`<!$>` is not valid HTML comment syntax. It is a **bogus comment**, and the HTML
tokenizer has a defined recovery path for it: on seeing `<!` not followed by
`--`, `DOCTYPE`, or `[CDATA[`, it enters the bogus comment state and emits a
comment whose data is everything up to the next `>`.

So `<!$>` becomes a comment with data `$`, exactly as `<!--$-->` does. The
client form is only usable because it goes through a parser, it is embedded in
a template string that the browser parses via `innerHTML`
(`dom-expressions/src/client.js:78`, `t.innerHTML = html`). SSR output is written
to a byte stream that no parser sees before the browser's, so the long form is
required there.

`<!>` is the same trick with empty data: a plain anchor comment marking a single
dynamic insertion point.

## Upstream states the equivalence

The plugin's own validator normalises one form into the other before checking a
template round-trips, which is the authoritative statement that they are
interchangeable -
`packages/babel-plugin-jsx-dom-expressions/src/shared/validate.js:31-33`:

```js
    .replaceAll("<!>", "<!---->")
    .replaceAll("<!$>", "<!--$-->")
    .replaceAll("<!/>", "<!--/-->")
```

## Where each is emitted

**Client**, in `createPlaceholder` -
`packages/babel-plugin-jsx-dom-expressions/src/dom/element.js:1176-1181`:

```js
function createPlaceholder(path, results, tempPath, i, char) {
  const exprId = path.scope.generateUidIdentifier("el$"),
    config = getConfig(path);
  let contentId;
  results.template += `<!${char}>`;
  results.templateWithClosingTags += `<!${char}>`;
```

`char` is `"$"` (called at `:1138`), `"/"` (at `:1142`), or `""` for the plain
anchor.

**SSR**, in the element transform -
`packages/babel-plugin-jsx-dom-expressions/src/ssr/element.js:500-509`:

```js
      // boxed by textNodes
      if (markers && !child.spreadElement) {
        appendToTemplate(results.template, `<!--$-->`);
        results.template.push("");
        results.templateValues.push(child.exprs[0]);
        appendToTemplate(results.template, `<!--/-->`);
      } else {
```

## When markers are emitted at all

`packages/babel-plugin-jsx-dom-expressions/src/dom/element.js:1133-1134`:

```js
      const multi = checkLength(filteredChildren),
        markers = config.hydratable && multi;
```

So a marker **pair** is emitted only when hydration is on **and** the parent has
more than one child (`checkLength` returns `i > 1`,
`src/shared/utils.js:212-221`). A single dynamic child of a hydratable element
gets **no markers**, it is hydrated by `insert(parent, expr)` reusing
`parent.childNodes` directly.

The bare `<!>` anchor is emitted on a different condition: when the expression is
sandwiched between text nodes (`wrappedByText`, `src/shared/utils.js:247-268`),
regardless of hydration.

### Adjacent-anchor merging

Consecutive expression children **share one `<!>`** in the non-hydratable case.
The guard is the `nextPlaceholder` variable (`src/dom/element.js:1139-1144`): it
is set after emitting an anchor, reused by the next adjacent expression, and
cleared (`:1171`) when a non-expression child intervenes. So
`<span> {greeting}{name} </span>` compiles to the template
`` `<span> <!> ` `` with both inserts targeting the same node.

There is **no** merging in the hydratable path: each expression gets its own
`<!$>`...`<!/>` pair, because the pair is what delimits its content during
hydration.

## `getNextMarker`'s depth counting

`dom-expressions/src/client.js:287-306`:

```js
export function getNextMarker(start) {
  let end = start, count = 0, current = [];
  if (isHydrating(start)) {
    while (end) {
      if (end.nodeType === 8) {
        const v = end.nodeValue;
        if (v === "$") count++;
        else if (v === "/") {
          if (count === 0) return [end, current];
          count--;
        }
      }
      current.push(end);
      end = end.nextSibling;
    }
  }
  return [end, current];
}
```

The semantics:

- Scan **forward over siblings** from `start`, this is a flat sibling walk, not
  a tree descent. Markers nested inside a child element are invisible to it.
- `$` pushes depth, `/` pops. The `/` seen at **depth 0** is this marker's
  partner; anything deeper belongs to a nested marker pair.
- Returns `[endNode, current]` where `current` is every node passed over, the
  existing server-rendered content between the markers, which `insert` then owns
  and may replace.
- The closing marker itself is **not** pushed into `current`: the early `return`
  happens before the `current.push(end)`.
- When not hydrating (`isHydrating`, `client.js:330-332`) the loop is skipped
  entirely and it returns `[start, []]`.

The depth counter is why the pairs must nest properly. An emitter that emits an
unbalanced `$` or `/` does not merely mislabel one position, it desynchronises
every marker after it in the same sibling list.

## A third, unrelated marker: `<!--!$-->`

Not a hydration marker. It is the SSR **array-element separator**, written
between two adjacent non-object values so the client can tell where one array
item ends and the next begins, `dom-expressions/src/server.js:488`:

```js
      if (!top && typeof prev !== "object" && typeof node[i] !== "object") mapped += `<!--!$-->`;
```

It is stripped during hydration, `dom-expressions/src/client.js:468`:

```js
      if (node.nodeType === 8 && node.data.slice(0, 2) === "!$") node.remove();
```

Note the test is a **prefix** check (`slice(0, 2)`), not equality, so it also
matches the streaming placeholder form `<!--!$${id}-->` used by
`server.js:155-158`.

An emitter must not confuse `!$` with `$`: `getNextMarker` compares
`nodeValue === "$"` exactly, so a `!$` separator is correctly ignored by the
depth counter, but only because the comparison is exact. Loosening it to a
prefix test would break marker pairing.

## Consequences for the emitter

1. Emit the short form (`<!$>`, `<!/>`, `<!>`) **only** inside template strings
   that will be parsed via `innerHTML`. Anywhere the bytes reach the browser
   without an intervening parse, i.e. SSR, emit the long form.
2. Pairs must be balanced within a sibling list, and nesting must be proper.
3. Emit pairs only for dynamic children of **multi-child** parents when
   hydrating; a lone dynamic child must get none, or hydration will look for a
   marker that the server never wrote.
4. Never emit a comment whose data is `$` or `/` for any other purpose.
