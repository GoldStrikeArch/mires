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

func stored(history *js.Object) []string {
	rows := history.Index(0).Index(3)
	values := make([]string, rows.Length())
	for i := range values {
		values[i] = rows.Index(i).Index(2).String()
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
			rows.Call("push", map[string]interface{}{"id": doc.Get("id"), "group": "documents", "value": json.Call("stringify", doc)})
		}
		return list("idb", map[string]interface{}{"batches": list(map[string]interface{}{"rows": rows}), "keys": list(), "groups": list(), "step": 1})
	case 1:
		docs := list()
		for _, text := range stored(history) {
			docs.Call("push", json.Call("parse", text))
		}
		options := map[string]interface{}{"fields": list("title", "body"), "storeFields": list("body")}
		add := map[string]interface{}{"op": "addAll", "documents": docs}
		query := map[string]interface{}{"op": "search", "query": input.Get("query")}
		return list("minisearch", map[string]interface{}{"options": options, "operations": list(add, query)})
	case 2:
		result, markdown := hits(history), ""
		if result.Length() != 0 {
			markdown = result.Index(0).Get("body").String()
		}
		return list("marked", map[string]interface{}{"op": "parse", "markdown": markdown})
	case 3:
		return list("fflate", map[string]interface{}{"op": "strToU8", "text": strings.Join(stored(history), "\n")})
	case 4:
		return list("fflate", map[string]interface{}{"op": "gzipSync", "data": map[string]interface{}{"bytes": history.Index(3)}, "options": map[string]interface{}{"level": 6, "mtime": 0}})
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
