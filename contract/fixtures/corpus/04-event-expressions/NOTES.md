# 04 — event expressions

Which of the three dispatch paths a handler takes, decided entirely by the event
name.

## Why the two are equivalent

Topcoat's `@name=$(closure)` and JSX's `onName={closure}` both attach a handler.
Both sides consult the same 22-name delegated set — view-dom vendors it in
`delegated_events.rs`, pinned against `fixtures/delegated-events.json` — so
`click` delegates and `change` does not, identically.

## Deliberate differences

**Topcoat has one spelling where JSX has five.** Upstream's fixture covers
`onclick`, `onClick`, `on:click`, `oncapture:click`, and the
`[handler, arg]` bound-argument array. Topcoat has only `@click`. So:

- **`on:` has no equivalent.** In JSX it forces `addEventListener` even for a
  delegated name. In Topcoat the only way to reach `addEventListener` is to use
  a name that is not in the delegated set — the choice is not the author's.
- **`oncapture:` has no equivalent.** No capture-phase handlers at all.
- **The `[handler, data]` array form has no equivalent.** Solid uses it to avoid
  a closure allocation per row; Topcoat's closure captures directly. This is a
  performance affordance, not a semantic one, but it does mean the emitted
  `addEventListener` call shape differs: solid passes the array, Topcoat passes a
  function.
- **`@click="alert(1)"`, raw JavaScript as a string, has no JSX equivalent** and
  is in any case rejected by view-dom today ("an event handler written as
  JavaScript").

**`@custom-event` is not `on:custom-event`.** The JSX file uses `on:custom-event`
because that is how JSX reaches a non-delegated listener for a dashed name; the
Topcoat file writes `@custom-event`. Same outcome, different route.

## view-dom features needed

- `@name=$(expr)` and `@name=(expr)` — **present**, both lower identically as
  `reactive: false`
- the delegated/direct split and the sorted per-template `events` set —
  **present**
- **NOT-YET-LOWERABLE:** capture phase, the bound-argument array form, and
  handlers written as raw JavaScript strings.
