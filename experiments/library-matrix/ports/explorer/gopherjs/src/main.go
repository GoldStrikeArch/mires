package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

func list(values ...interface{}) *js.Object {
	a := js.Global.Get("Array").New()
	for _, v := range values {
		a.Call("push", v)
	}
	return a
}

// object is a JS object literal: GopherJS externalizes a Go map to a plain object.
type object = map[string]interface{}

// persist is one idb script (idb CONTRACT.md, "Input"): it creates the store, puts every row in
// one readwrite transaction, then walks the store with a cursor in primary-key order.
func persist(rows *js.Object) object {
	transaction := func(mode string, step object) object {
		return object{"op": "tx", "conn": "explorer", "stores": "rows", "mode": mode, "steps": list(step), "catch": false}
	}
	schema := list(
		object{"op": "createStore", "name": "rows", "options": object{"keyPath": "id"}},
		object{"op": "createIndex", "store": "rows", "name": "byGroup", "keyPath": "group"},
	)
	openDB := object{"op": "openDB", "db": "explorer", "conn": "explorer", "version": 1, "upgrade": schema, "catch": false}
	puts := list()
	for i := 0; i < rows.Length(); i++ {
		puts.Call("push", object{"op": "put", "store": nil, "index": nil, "args": list(rows.Index(i)), "catch": false})
	}
	write := object{"op": "batch", "requests": puts, "abort": false, "done": true}
	walk := object{"op": "openCursor", "store": nil, "index": nil, "args": list(), "positions": list(),
		"rest": list(list("continue")), "catch": false}
	return object{"steps": list(openDB, transaction("readwrite", write), transaction("readonly", walk))}
}

// stored returns the stored rows' value texts, from the cursor entries of the idb log.
func stored(history *js.Object) []string {
	log := history.Index(0).Get("log")
	var values []string
	for i := 0; i < log.Length(); i++ {
		if entry := log.Index(i); entry.Index(0).String() == "cursor" {
			values = append(values, entry.Index(3).Get("value").String())
		}
	}
	return values
}

func hits(history *js.Object) *js.Object {
	return history.Index(1).Get("results").Index(1).Get("hits")
}

func run(packet *js.Object) *js.Object {
	input, history := packet.Index(0), packet.Index(1)
	json := js.Global.Get("JSON")
	switch history.Length() {
	case 0:
		rows, docs := list(), input.Get("records")
		for i := 0; i < docs.Length(); i++ {
			doc := docs.Index(i)
			rows.Call("push", object{"id": doc.Get("id"), "group": "documents", "value": json.Call("stringify", doc)})
		}
		return list("idb", persist(rows))
	case 1:
		docs := list()
		for _, text := range stored(history) {
			docs.Call("push", json.Call("parse", text))
		}
		options := object{"fields": list("title", "body"), "storeFields": list("body")}
		add := object{"op": "addAll", "documents": docs}
		query := object{"op": "search", "query": input.Get("query")}
		return list("minisearch", object{"options": options, "operations": list(add, query)})
	case 2:
		result, markdown := hits(history), ""
		if result.Length() != 0 {
			markdown = result.Index(0).Get("body").String()
		}
		return list("marked", object{"op": "parse", "markdown": markdown})
	case 3:
		return list("fflate", object{"op": "strToU8", "text": strings.Join(stored(history), "\n")})
	case 4:
		return list("fflate", object{"op": "gzipSync", "data": object{"bytes": history.Index(3)}, "options": object{"level": 6, "mtime": 0}})
	case 5:
		return list("uplot", input.Get("chart"))
	default:
		ids, result := list(), hits(history)
		for i := 0; i < result.Length(); i++ {
			ids.Call("push", result.Index(i).Get("id"))
		}
		return list(nil, list(ids, history.Index(2), history.Index(4), history.Index(3).Length(), history.Index(5)))
	}
}
func main() { js.Module.Get("exports").Set("run", run) }
