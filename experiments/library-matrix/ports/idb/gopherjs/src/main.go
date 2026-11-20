package main

import "github.com/gopherjs/gopherjs/js"

func globalValue(k string) *js.Object              { return js.Global.Get(k) }
func get(x *js.Object, k string) *js.Object        { return x.Get(k) }
func setProp(x *js.Object, k string, v *js.Object) { x.Set(k, v) }
func at(x *js.Object, i int) *js.Object            { return x.Index(i) }
func missing(x *js.Object) bool                    { return x == js.Undefined }
func truth(x *js.Object) bool                      { return js.Global.Get("Boolean").Invoke(x).Bool() }
func equalNum(x *js.Object, i int) bool            { return x.Int() == i }
func equalText(x *js.Object, s string) bool        { return x.String() == s }
func strVal(s string) *js.Object                   { return js.Global.Get("String").Invoke(s) }
func num(i int) *js.Object                         { return js.Global.Get("Number").Invoke(i) }
func nilValue() *js.Object                         { return nil }
func undef() *js.Object                            { return js.Undefined }
func objectValue() *js.Object                      { return js.Global.Get("Object").New() }
func pack(values []*js.Object) *js.Object {
	a := js.Global.Get("Array").New()
	for _, v := range values {
		a.Call("push", v)
	}
	return a
}
func push(a, v *js.Object) { a.Call("push", v) }
func call(x *js.Object, k string, args []*js.Object) *js.Object {
	return js.Global.Get("Reflect").Call("apply", x.Get(k), x, pack(args))
}
func callback(f func(*js.Object) *js.Object) *js.Object { return js.InternalObject(f) }
func promise(f func(*js.Object, *js.Object) *js.Object) *js.Object {
	return js.Global.Get("Promise").New(f)
}
func settle(f, v *js.Object)                                       { f.Invoke(v) }
func chain(p *js.Object, f func(*js.Object) *js.Object) *js.Object { return p.Call("then", f) }
func always(p *js.Object, f func(*js.Object) *js.Object) *js.Object {
	return p.Call("finally", func() *js.Object { return f(js.Undefined) })
}
func mapValues(xs *js.Object, f func(*js.Object) *js.Object) *js.Object {
	a := pack(nil)
	for i := 0; i < xs.Length(); i++ {
		push(a, f(xs.Index(i)))
	}
	return a
}
func sequence(xs *js.Object, f func(*js.Object) *js.Object) *js.Object {
	p := js.Global.Get("Promise").Call("resolve")
	for i := 0; i < xs.Length(); i++ {
		item := xs.Index(i)
		p = chain(p, func(_ *js.Object) *js.Object { return f(item) })
	}
	return p
}
func abortError() *js.Object               { return globalValue("DOMException").New("AbortError", "AbortError") }
func errorName(name *js.Object) *js.Object { return strVal("!" + name.String()) }
func uniqueName() *js.Object {
	return strVal("mires-matrix-idb-" + globalValue("crypto").Call("randomUUID").String())
}

func request(req *js.Object) *js.Object {
	executor := func(resolve *js.Object, reject *js.Object) *js.Object {
		success := func(event *js.Object) *js.Object {
			setProp(req, "onsuccess", nilValue())
			setProp(req, "onerror", nilValue())
			settle(resolve, get(req, "result"))
			return undef()
		}
		failure := func(event *js.Object) *js.Object {
			setProp(req, "onsuccess", nilValue())
			setProp(req, "onerror", nilValue())
			settle(reject, get(req, "error"))
			return undef()
		}
		setProp(req, "onsuccess", callback(success))
		setProp(req, "onerror", callback(failure))
		return undef()
	}
	return promise(executor)
}
func clearTransaction(tx *js.Object) *js.Object {
	setProp(tx, "oncomplete", nilValue())
	setProp(tx, "onerror", nilValue())
	setProp(tx, "onabort", nilValue())
	return undef()
}
func transactionDone(tx *js.Object) *js.Object {
	executor := func(resolve *js.Object, reject *js.Object) *js.Object {
		complete := func(event *js.Object) *js.Object {
			clearTransaction(tx)
			settle(resolve, undef())
			return undef()
		}
		failure := func(event *js.Object) *js.Object {
			reason := get(tx, "error")
			clearTransaction(tx)
			settle(reject, (func() *js.Object {
				if truth(reason) {
					return reason
				}
				return abortError()
			}()))
			return undef()
		}
		setProp(tx, "oncomplete", callback(complete))
		setProp(tx, "onerror", callback(failure))
		setProp(tx, "onabort", callback(failure))
		return undef()
	}
	return promise(executor)
}
func openDatabase(name *js.Object) *js.Object {
	req := call(globalValue("indexedDB"), "open", []*js.Object{name, num(1)})
	upgrade := func(event *js.Object) *js.Object {
		options := objectValue()
		setProp(options, "keyPath", strVal("id"))
		store := call(get(req, "result"), "createObjectStore", []*js.Object{strVal("rows"), options})
		call(store, "createIndex", []*js.Object{strVal("byGroup"), strVal("group")})
		return undef()
	}
	setProp(req, "onupgradeneeded", callback(upgrade))
	return request(req)
}
func writeBatch(db *js.Object, batch *js.Object) *js.Object {
	tx := call(db, "transaction", []*js.Object{strVal("rows"), strVal("readwrite")})
	completion := transactionDone(tx)
	store := call(tx, "objectStore", []*js.Object{strVal("rows")})
	submit := func(row *js.Object) *js.Object {
		return (func() *js.Object {
			if truth(get(batch, "add")) {
				return request(call(store, "add", []*js.Object{row}))
			}
			return request(call(store, "put", []*js.Object{row}))
		}())
	}
	promises := mapValues(get(batch, "rows"), submit)
	push(promises, completion)
	settled := call(globalValue("Promise"), "allSettled", []*js.Object{promises})
	if truth(get(batch, "abort")) {
		call(tx, "abort", []*js.Object{})
	}
	collect := func(items *js.Object) *js.Object {
		outcome := func(item *js.Object) *js.Object {
			value := get(item, "value")
			return (func() *js.Object {
				if equalText(get(item, "status"), "rejected") {
					return errorName(get(get(item, "reason"), "name"))
				}
				return (func() *js.Object {
					if missing(value) {
						return strVal("done")
					}
					return value
				}())
			}())
		}
		return mapValues(items, outcome)
	}
	return chain(settled, collect)
}
func lookup(source *js.Object, keys *js.Object) *js.Object {
	submit := func(key *js.Object) *js.Object {
		return request(call(source, "get", []*js.Object{key}))
	}
	return call(globalValue("Promise"), "all", []*js.Object{mapValues(keys, submit)})
}
func storedValues(rows *js.Object) *js.Object {
	value := func(row *js.Object) *js.Object {
		return (func() *js.Object {
			if missing(row) {
				return nilValue()
			}
			return get(row, "value")
		}())
	}
	return mapValues(rows, value)
}
func readBack(db *js.Object, inputValue *js.Object, transactions *js.Object) *js.Object {
	tx := call(db, "transaction", []*js.Object{strVal("rows")})
	completion := transactionDone(tx)
	store := call(tx, "objectStore", []*js.Object{strVal("rows")})
	index := call(store, "index", []*js.Object{strVal("byGroup")})
	reads := lookup(store, get(inputValue, "keys"))
	indexReads := lookup(index, get(inputValue, "groups"))
	source := (func() *js.Object {
		if truth(get(inputValue, "index")) {
			return index
		}
		return store
	}())
	only := get(inputValue, "only")
	bounds := (func() *js.Object {
		if missing(only) {
			return nilValue()
		}
		return call(globalValue("IDBKeyRange"), "only", []*js.Object{only})
	}())
	direction := (func() *js.Object {
		if truth(get(inputValue, "reverse")) {
			return strVal("prev")
		}
		return strVal("next")
	}())
	req := call(source, "openCursor", []*js.Object{bounds, direction})
	step := get(inputValue, "step")
	cursors := pack([]*js.Object{})
	var visit func(*js.Object) *js.Object
	visit = func(cursor *js.Object) *js.Object {
		if truth(cursor) {
			push(cursors, pack([]*js.Object{get(cursor, "key"), get(cursor, "primaryKey"), get(get(cursor, "value"), "value")}))
			if equalNum(step, 1) {
				call(cursor, "continue", []*js.Object{})
			} else {
				call(cursor, "advance", []*js.Object{step})
			}
			return chain(request(req), visit)
		} else {
			return cursors
		}
	}
	cursorDone := chain(request(req), visit)
	awaited := call(globalValue("Promise"), "all", []*js.Object{pack([]*js.Object{reads, indexReads, cursorDone, completion})})
	output := func(result *js.Object) *js.Object {
		return pack([]*js.Object{transactions, storedValues(at(result, 0)), storedValues(at(result, 1)), at(result, 2)})
	}
	return chain(awaited, output)
}
func exercise(db *js.Object, inputValue *js.Object) *js.Object {
	transactions := pack([]*js.Object{})
	write := func(batch *js.Object) *js.Object {
		saved := func(outcomes *js.Object) *js.Object {
			push(transactions, outcomes)
			return undef()
		}
		return chain(writeBatch(db, batch), saved)
	}
	nextPhase := func(unused *js.Object) *js.Object {
		return readBack(db, inputValue, transactions)
	}
	return chain(sequence(get(inputValue, "batches"), write), nextPhase)
}
func run(inputValue *js.Object) *js.Object {
	name := uniqueName()
	opened := func(db *js.Object) *js.Object {
		close := func(unused *js.Object) *js.Object {
			return call(db, "close", []*js.Object{})
		}
		return always(exercise(db, inputValue), close)
	}
	cleanup := func(unused *js.Object) *js.Object {
		return request(call(globalValue("indexedDB"), "deleteDatabase", []*js.Object{name}))
	}
	return always(chain(openDatabase(name), opened), cleanup)
}
func main() { js.Module.Get("exports").Set("run", run) }
