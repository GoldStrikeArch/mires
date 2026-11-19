// Package main is MiniSearch 7.2.0 in Go, compiled to JavaScript by GopherJS: a faithful port of
// the subset libraries/minisearch/CONTRACT.md defines.
//
//   - searchable_map.go: the radix tree (SearchableMap), with its prefix and fuzzy walks;
//   - minisearch.go: the MiniSearch type (documents, field lengths, vacuuming, state);
//   - search.go: search and autoSuggest (query trees, BM25+, the combinators, result assembly);
//   - options.go: search options and their spread-merging;
//   - text.go: UTF-16 text, the default tokenizer and toLowerCase;
//   - boundary.go: the corpus input into Go values, the operation script, the result back.
//
// run is published on js.Module.exports, which the builder's facade binds and re-exports.
package main

import "github.com/gopherjs/gopherjs/js"

func main() {
	js.Module.Get("exports").Set("run", run)
}
