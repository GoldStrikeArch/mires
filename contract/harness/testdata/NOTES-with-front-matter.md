---
{
  "deltas": [
    {
      "id": "text-gt-escaping",
      "kind": "rewrite",
      "side": "b",
      "op": "template",
      "field": "html",
      "from": "&gt;",
      "to": ">",
      "why": "Same rule as testdata/deltas.json, declared inline instead. Exists so the test covers the NOTES.md front-matter path as well as the JSON one."
    }
  ]
}
---

# Front-matter delta declaration

`compare-trace.mjs --delta` accepts this file as well as a `.json` one. The front
matter is delimited by `---` lines and its body is **JSON, not YAML** -- this
package has no dependencies and is not about to grow a YAML parser for four
fields.

A per-fixture `NOTES.md` can therefore carry its own accepted deltas next to the
prose explaining them, which is the point: the justification and the rule live in
the same file and go stale together, visibly.
