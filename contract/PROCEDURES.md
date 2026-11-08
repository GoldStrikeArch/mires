# PROCEDURES: the procedure call wire

What a client POSTs to a `#[procedure]` route and what comes back, stated once so
that the framework crate's HTTP tests and wave 3's client fixtures cannot disagree
about it while both passing.

The machine-readable half is `fixtures/procedure-wire.json`. This page is the prose:
what the wire is, why it is that and not something else, and what is not settled.
Neither is the implementation, and both cite it.

**Status.** The surrogate wire below is what exists today. The serde wire is the
wave-2 framework agent's design, decided and recorded in
`build/logs/wave2-framework-report.md:88-137`, with `SERDE_CONTENT_TYPE` and
`serde_args` already landed in `crates/topcoat-runtime/src/procedure.rs` and the
`#[procedure(serde)]` codegen not yet wired. This page describes both wires as
specified; `fixtures/procedure-wire.json` marks each vector as pinned by an existing
test or derived from the cited code, so what is measured and what is predicted are
told apart per row rather than in a caveat at the top.

## Why this page is a contract artifact and not framework documentation

Three consumers have to agree, and two of them do not exist yet:

1. the server, which decodes the body;
2. the framework's HTTP tests, which are being written this wave and were the
   repo's first procedure tests at any level above parsing
   (`build/logs/wave2-framework-report.md:73-77`);
3. wave 3's client fixtures, which assert that the Rust-to-JS client PRODUCES these
   bytes.

Consumers 2 and 3 read the same rows in opposite directions. If each writes its own
expectations from the source, both can be green while disagreeing about, say, whether
a zero-argument call sends `[]` or `null`, and nothing would notice until a real
client met a real server. So the rows live here, once.

## The route

`POST /_topcoat/procedures/{id}`

- The prefix is `PROCEDURE_ROUTE_PREFIX` (`crates/topcoat-runtime/src/procedure.rs:13`)
  and the route is built by `ProcedureRoute::new` (`:169-182`).
- POST and nothing else: `Methods::Only(&[Method::POST])` (`:184-187`). Any other
  method is the router's 405, not the procedure's business.
- The id is a v4 UUID minted at MACRO-EXPANSION time
  (`crates/topcoat-runtime/grammar/src/procedure.rs:123`). So it is baked into both
  halves of one build and is stable for a given binary, and it is NOT stable across
  rebuilds. A client and a server from different builds do not share procedure ids.
  This is the single most important operational fact on this page for anyone caching
  a compiled client.
- The client URL-encodes the id
  (`crates/topcoat-runtime/browser/src/surrogate/procedure.ts:12`).
- An id that was never mounted has no route, so it is an ordinary 404. There is no
  "no such procedure" error.

## Two wires, told apart by the request media type

| | surrogate wire | serde wire |
| --- | --- | --- |
| selected by | `#[procedure]` | `#[procedure(serde)]` |
| request `Content-Type` | `application/json` | `application/topcoat+json` |
| arguments | JSON array of `dehydrate()` forms | JSON array, each by its own `Serialize` |
| **zero arguments** | **`null`** | **`[]`** |
| response `Content-Type` | `application/json` | `application/json` |
| response body | the return value's surrogate | the return value by `Serialize` |

A procedure serves ONE wire. That is not a simplification, it is what makes the serde
wire possible at all: emitting both branches in one handler would put
`A: Surrogated + R: Surrogated` on every serde procedure, and escaping exactly that
bound is the whole point, since an arbitrary `#[derive(Deserialize)]` type has no
`Surrogated` impl. Bounds are checked at the generated call site whether or not the
branch is reachable, so there is no honest way to have both. The consequence is
observable and acceptable: a surrogate-wire caller hitting a serde procedure is
rejected by the content-type guard with a message naming the media type it wanted,
and a procedure needed by both clients during a migration is declared twice
(`build/logs/wave2-framework-report.md:99-110`).

A serde-only procedure also cannot be named from `expr!`, and needs no new
enforcement to prevent it: `ProcedureSurrogate::call` requires `A: Surrogated`
(`crates/topcoat-runtime/src/procedure.rs:234-247`), so the error appears at the
`expr!` call site for free.

### Why a distinct media type and not `application/json`

Because the surrogate wire already occupies `application/json`: the browser client
posts it explicitly (`browser/src/surrogate/procedure.ts:15`). So `application/json`
cannot identify the serde wire, and the new wire takes the distinct type while the
existing one is untouched. Recorded upstream as deviation D1.

`application/topcoat+json` is a structured-suffix type, which means the router's own
`Json` extractor already accepts it: `json_content_type` takes `application/json` and
any `application/*+json` (`crates/topcoat-router/src/content/json.rs:154-170`). That
is what lets the serde handler guard on the exact media type and then reuse `Json`
for the body read, the size limit and the `serde_path_to_error` diagnostics, rather
than reaching for a bare `serde_json::from_slice` and losing all three.

### Media type matching

Take everything before the first `;`, trim, compare case-insensitively
(`crates/topcoat-runtime/src/procedure.rs:56-66`). So
`application/topcoat+json; charset=utf-8` and `Application/Topcoat+JSON` are both
accepted, and a client that sends a charset is not punished for it.

### The zero-argument asymmetry, which is deliberate and unshared

The surrogate wire sends the literal `null`, because the argument tuple for zero
arguments is `()`, whose surrogate is `()`, and serde accepts only `null` for a unit.
The browser client spells it `args.length === 0 ? null : args.map(...)`
(`browser/src/surrogate/procedure.ts:17-24`).

The serde wire sends `[]`, because its zero-argument codegen decodes `[(); 0]`, which
accepts exactly a zero-length array (`crates/topcoat-runtime/src/procedure.rs:26-30`).

Each wire rejects the other's spelling, and that is what makes the asymmetry safe
rather than a trap: `[]` is a 400 on the surrogate wire and `null` is a 400 on the
serde wire, both pinned (`:302-323`). A single client cannot accidentally satisfy the
wrong one.

Note also that ONE argument is still an array. The tuple is `(T,)`, so the body is
`[value]` and never a bare `value`.

## Errors

Every error is `text/plain; charset=utf-8` carrying the error's `Display` string
(`crates/topcoat-router/src/response.rs:17,127`). There is no JSON error envelope,
and inventing one for procedures alone would make them the only endpoint in the
framework with one.

| case | status | body |
| --- | --- | --- |
| wrong media type | 400 | ``bad request: expected request with `Content-Type: application/topcoat+json` `` |
| malformed JSON | 400 | `bad request: invalid JSON syntax: ...` |
| wrong argument type | 400 | ``bad request: invalid JSON value: ... (at `[1]`) `` |
| wrong arity | 400 | `bad request: invalid JSON value: ...` |
| the procedure returned an error | its own | see below |
| method other than POST | 405 | the router's |
| unknown id | 404 | the router's |

The 400 shape comes from `BadRequestError`, whose `Display` is
`bad request: {description}` or `bad request: {description} (at `{path}`)`
(`crates/topcoat-router/src/error/bad_request.rs:70-84`). The path on a decode
failure is `serde_path_to_error`'s, which for an argument array is the argument's
zero-based index **in brackets**: `` (at `[1]`) `` means the second argument was
wrong. That is worth more than it looks, because it is the difference between "your
call was wrong" and "your second argument was wrong".

The brackets are the rule and not a quirk of this case: `serde_path_to_error` joins
segments with `.` and renders a SEQUENCE index as `[n]`, and the argument array is a
sequence. So a bad field inside a struct argument should read `` [0].name ``. The
top-level form is observed; the nested form follows from the same rule and is **not
confirmed**. Do not assert it without checking, because the top-level spelling was
predicted wrong here once already: this page originally said `` (at `1`) ``, and the
framework agent's landed HTTP tests corrected it. `fixtures/procedure-wire.json`'s
`$corrections` block records it, and the lesson is recorded there too, because it is
the more useful half. The vector was NOT marked `derived` -- it cited a real unit test
(`crates/topcoat-runtime/src/procedure.rs:341-349`), but that test asserts only that
the error is a `BadRequestError`, not what the rendered path says. **A citation is
only as strong as what the cited test actually asserts**, and nothing in this page's
own gate could have caught that, because the gate reads source text and does not run
the handler.

**Wrong media type is 400 and not 415**, matching what `Json` itself already returns
for a content-type mismatch (`content/json.rs:86-88`). A real 415 would mean a new
public error type in `crates/topcoat-router`, including a `try_downcast!` arm in
`error_into_response` (`error.rs:45-51`) that silently degrades to a 500 if
forgotten. Two different statuses for "wrong content type on a JSON endpoint" in one
framework is worse than HTTP purism. Recorded upstream as deviation D2.
`harness/check-procedure-wire.mjs` asserts that no 415 constructor has appeared, so
if one ever does, this rationale gets revisited rather than quietly outliving its
premise.

**Do not assert the whole line for a decode error.** The middle of the message is
serde_json's own wording, e.g. `invalid type: integer `7`, expected a string at line
1 column 7`. That is not Topcoat's string to pin, and a serde_json patch release
would break the test for no reason. Assert the prefix and the path, which is what the
vectors' `bodyStartsWith` / `bodyEndsWith` pair is for.

**A procedure's own error is the router's ordinary error, unchanged.** Procedures add
no error handling: the handler returns `Result<Response>` and
`error_into_response` (`crates/topcoat-router/src/error.rs:34-53`) downcasts
`Forbidden`, `BadRequest`, `InternalServer`, `NotFound`, `MethodNotAllowed`,
`Redirect`, `Unauthorized` in that order, and anything else becomes a 500 whose body
is the literal `internal server error` with nothing about the cause
(`error/internal_server.rs:55-67`). So a procedure that fails for an internal reason
tells the client nothing, which is the correct default and worth knowing before
someone tries to debug a client against it.

## The client side, and what it says about the executor

The existing browser client is worth reading before designing the Rust one, because
it already answers the question the executor decision turns on.

`Procedure.call` returns a `Future`, and `Future` is a LAZY `PromiseLike`: the async
thunk runs on the first `then` and the resulting promise is memoised
(`crates/topcoat-runtime/browser/src/surrogate/future.ts:1-23`, whose own doc comment
says "This mimics the behavior of Rust's Futures"). That is Rust's poll-when-driven
semantics rather than a Promise's run-on-construction semantics, and it is the shape
the Rust-to-JS client should keep: a `Future` that has not been awaited must not have
issued its `fetch`.

Two constraints from the pinned runtime bear directly on it, both in CONTRACT-DOM
point 14:

- **A future must not be resolved inside the hydration bracket** (14.6). The bracket
  is exactly one synchronous call stack, and after a suspension point
  `sharedConfig.context` is null, so `getNextElement` builds fresh DOM instead of
  adopting the server's, silently in production. A procedure call during island
  setup is therefore not an option; it belongs in an event handler or an effect that
  runs after hydration.
- **Nothing upstream cancels** (14.11). There is no `AbortController` anywhere in the
  pin, and `createResource`'s cleanup ignores an in-flight request rather than
  aborting it. Dropping a Rust future is expected to cancel its work, and a procedure
  call's `fetch` is the first thing that will want that. Today's browser client does
  not abort either, so a dropped call keeps its request in flight and discards the
  answer.

Rejection handling today: the client throws
`` `Procedure call failed: ${response.status} ${response.statusText}` `` on any
non-2xx (`browser/src/surrogate/procedure.ts:26-31`), which discards the response
BODY. So the 400 diagnostics above, including the argument index the server went to
the trouble of computing, never reach the developer. That is a defect worth fixing in
whichever client is written next, and it is recorded here rather than in a TODO
because the wire spec is the place where "the server says this and the client throws
it away" is visible.

## Test vectors

`fixtures/procedure-wire.json`, 19 vectors. Each carries the media type, the exact
body, and the expectation; `direction` says whether it is a request or a response,
and `derived: true` marks a vector that follows from the cited code but that no
existing test pins yet.

Read them one way for server tests (build the request, assert the response) and the
other way for client fixtures (assert the client produces this exact body for these
arguments). They are the same rows on purpose.

The framework crate distils these vectors into
`crates/topcoat-runtime/macro/tests/fixtures/procedure_wire.json` for its own HTTP
tests. That copy is downstream of this one and says so; when running the wire
disagrees with a vector, the correction lands there first (in a `$corrections` block)
and is folded back here. `check-procedure-wire.mjs` asserts the two agree and that no
reported correction is left unfolded, so a copy that knows something this file does
not is a gate failure rather than a divergence nobody notices.

`harness/check-procedure-wire.mjs` runs in `run-all.mjs`'s step list, so the spec is
gated by the same drift check as the generated fixtures even though nothing
regenerates it: it re-reads the source lines the file cites and fails if a cited
value moved. 78 checks over the 19 vectors today. What it covers is the route prefix,
both media types, the POST-only method, the UUID id, both zero-argument spellings,
the error prefixes, the exact wrong-media-type message, the absence of a 415, that
every vector body is valid JSON where it claims to be, and that every accept/reject
media type lands on the side the spec says.

What it cannot cover, because it reads text and does not compile anything: whether
the handler actually behaves this way. That is what the framework's HTTP tests are
for, and they should consume these vectors rather than restate them.

## Open questions

1. **The response has no error envelope, so a client cannot tell a 400 from the
   server's guard apart from a 400 the procedure itself returned.** Both are
   `text/plain` starting with `bad request:`. Fine today; it stops being fine as soon
   as a client wants to retry one and not the other.
2. **`#[procedure(serde)]` codegen is not landed**, so every serde-wire vector marked
   `derived` is a prediction. The helper it will call (`serde_args`) IS landed and
   unit-tested, so the request-decoding half is on firmer ground than the response
   half.
3. **Cancellation is unspecified on both sides.** See CONTRACT-DOM 14.11.
4. **Procedure ids do not survive a rebuild** (a v4 UUID per expansion). Nothing
   depends on it yet. A cached or separately-deployed client would make it a
   versioning problem, and the fix would be a stable id derived from the item path
   rather than a random one.
