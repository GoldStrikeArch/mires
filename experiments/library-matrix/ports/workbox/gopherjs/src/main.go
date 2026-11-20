package main

import (
	"github.com/gopherjs/gopherjs/js"
	"strings"
)

func globalValue(k string) *js.Object              { return js.Global.Get(k) }
func get(x *js.Object, k string) *js.Object        { return x.Get(k) }
func setProp(x *js.Object, k string, v *js.Object) { x.Set(k, v) }
func at(x *js.Object, i int) *js.Object            { return x.Index(i) }
func missing(x *js.Object) bool                    { return x == js.Undefined }
func nullish(x *js.Object) bool                    { return x == nil || missing(x) }
func truth(x *js.Object) bool                      { return js.Global.Get("Boolean").Invoke(x).Bool() }
func equalText(x *js.Object, s string) bool        { return !nullish(x) && x.String() == s }
func equalNum(x *js.Object, i int) bool            { return x.Int() == i }
func strVal(s string) *js.Object                   { return js.Global.Get("String").Invoke(s) }
func boolean(v bool) *js.Object                    { return js.Global.Get("Boolean").Invoke(v) }
func nilValue() *js.Object                         { return nil }
func undef() *js.Object                            { return js.Undefined }
func objectValue() *js.Object                      { return js.Global.Get("Object").New() }
func pack(xs []*js.Object) *js.Object {
	a := js.Global.Get("Array").New()
	for _, v := range xs {
		a.Call("push", v)
	}
	return a
}
func push(a, v *js.Object) { a.Call("push", v) }
func call(x *js.Object, k string, args []*js.Object) *js.Object {
	return js.Global.Get("Reflect").Call("apply", x.Get(k), x, pack(args))
}
func makeHost(k string, args []*js.Object) *js.Object {
	return globalValue("Reflect").Call("construct", globalValue(k), pack(args))
}
func chain(p *js.Object, f func(*js.Object) *js.Object) *js.Object { return p.Call("then", f) }
func recoverPromise(p *js.Object, f func(*js.Object) *js.Object) *js.Object {
	return p.Call("catch", f)
}
func always(p *js.Object, f func(*js.Object) *js.Object) *js.Object {
	return p.Call("finally", func() *js.Object { return f(js.Undefined) })
}
func mapValues(xs *js.Object, f func(*js.Object) *js.Object) {
	for i := 0; i < xs.Length(); i++ {
		f(xs.Index(i))
	}
}
func sequence(xs *js.Object, f func(*js.Object) *js.Object) *js.Object {
	p := ok(undef())
	for i := 0; i < xs.Length(); i++ {
		item := xs.Index(i)
		p = chain(p, func(_ *js.Object) *js.Object { return f(item) })
	}
	return p
}
func ok(x *js.Object) *js.Object               { return globalValue("Promise").Call("resolve", x) }
func bad(x *js.Object) *js.Object              { return globalValue("Promise").Call("reject", x) }
func suffix(x *js.Object, s string) *js.Object { return strVal(x.String() + s) }
func starts(x *js.Object, s string) bool       { return strings.HasPrefix(x.String(), s) }
func ends(x *js.Object, s string) bool         { return strings.HasSuffix(x.String(), s) }
func relative(url, base *js.Object) *js.Object {
	return strVal(strings.TrimPrefix(url.String(), base.String()))
}
func collect(it *js.Object) *js.Object { return globalValue("Array").Call("from", it) }
func badStatus(v *js.Object) bool      { return v.Int() >= 400 }
func uniqueName() *js.Object {
	return strVal("mires-workbox-" + globalValue("crypto").Call("randomUUID").String())
}
func skipWaiting() *js.Object              { return js.Global.Call("skipWaiting") }
func fetchRequest(r *js.Object) *js.Object { return js.Global.Call("fetch", r) }
func defaulted(x, y *js.Object) *js.Object {
	if missing(x) {
		return y
	}
	return x
}
func same(x, y *js.Object) bool { return js.Global.Get("Object").Call("is", x, y).Bool() }

func absolute(path *js.Object, base *js.Object) *js.Object {
	return get(makeHost("URL", []*js.Object{path, base}), "href")
}
func network(source *js.Object, base *js.Object) *js.Object {
	if equalText(source, "offline") {
		return bad(strVal("offline"))
	} else {
		req := makeHost("Request", []*js.Object{absolute(suffix(source, ".mjs"), base)})
		return fetchRequest(req)
	}
}
func precacheKey(entries *js.Object, path *js.Object, base *js.Object) *js.Object {
	url := makeHost("URL", []*js.Object{path, base})
	setProp(url, "hash", strVal(""))
	exact := call(entries, "get", []*js.Object{get(url, "href")})
	if !missing(exact) {
		return exact
	} else {
		params := get(url, "searchParams")
		keys := collect(call(params, "keys", []*js.Object{}))
		ignore := func(key *js.Object) *js.Object {
			if equalText(key, "fbclid") || starts(key, "utm_") {
				call(params, "delete", []*js.Object{key})
			}
			return undef()
		}
		mapValues(keys, ignore)
		stripped := call(entries, "get", []*js.Object{get(url, "href")})
		if !missing(stripped) {
			return stripped
		} else {
			original := get(url, "pathname")
			setProp(url, "pathname", suffix(original, "index.html"))
			index := (func() *js.Object {
				if ends(original, "/") {
					return call(entries, "get", []*js.Object{get(url, "href")})
				}
				return undef()
			}())
			if !missing(index) {
				return index
			} else {
				setProp(url, "pathname", suffix(original, ".html"))
				return call(entries, "get", []*js.Object{get(url, "href")})
			}
		}
	}
}
func install(entries *js.Object, cache *js.Object, source *js.Object, base *js.Object) *js.Object {
	updated := pack([]*js.Object{})
	unchanged := pack([]*js.Object{})
	check := func(entry *js.Object) *js.Object {
		original := at(entry, 0)
		key := at(entry, 1)
		found := func(hit *js.Object) *js.Object {
			if !missing(hit) {
				push(unchanged, relative(original, base))
				return undef()
			} else {
				received := func(res *js.Object) *js.Object {
					if badStatus(get(res, "status")) {
						return bad(strVal("bad-precaching-response"))
					} else {
						stored := func(unused *js.Object) *js.Object {
							push(updated, relative(original, base))
							return undef()
						}
						return chain(call(cache, "put", []*js.Object{key, call(res, "clone", []*js.Object{})}), stored)
					}
				}
				return chain(network(source, base), received)
			}
		}
		return chain(call(cache, "match", []*js.Object{key}), found)
	}
	complete := func(unused *js.Object) *js.Object {
		return pack([]*js.Object{updated, unchanged})
	}
	return chain(sequence(collect(call(entries, "entries", []*js.Object{})), check), complete)
}
func activate(entries *js.Object, cache *js.Object, base *js.Object) *js.Object {
	active := collect(call(entries, "values", []*js.Object{}))
	removed := pack([]*js.Object{})
	listKeys := func(keys *js.Object) *js.Object {
		remove := func(req *js.Object) *js.Object {
			path := get(req, "url")
			if truth(call(active, "includes", []*js.Object{path})) {
				return undef()
			} else {
				deleted := func(unused *js.Object) *js.Object {
					push(removed, relative(path, base))
					return undef()
				}
				return chain(call(cache, "delete", []*js.Object{path}), deleted)
			}
		}
		complete := func(unused *js.Object) *js.Object {
			return call(removed, "sort", []*js.Object{})
		}
		return chain(sequence(keys, remove), complete)
	}
	return chain(call(cache, "keys", []*js.Object{}), listKeys)
}
func strategy(entries *js.Object, cache *js.Object, name *js.Object, path *js.Object, source *js.Object, base *js.Object) *js.Object {
	key := precacheKey(entries, path, base)
	route := relative(path, base)
	if !missing(key) {
		precached := func(hit *js.Object) *js.Object {
			return (func() *js.Object {
				if missing(hit) {
					return network(source, base)
				}
				return hit
			}())
		}
		return chain(call(cache, "match", []*js.Object{key}), precached)
	} else {
		if starts(route, "network/") {
			return network(source, base)
		} else {
			if starts(route, "first/") || starts(route, "only/") {
				opened := func(runtime *js.Object) *js.Object {
					matched := func(hit *js.Object) *js.Object {
						if !missing(hit) {
							return hit
						} else {
							if starts(route, "only/") {
								return bad(strVal("no-response"))
							} else {
								fetched := func(response *js.Object) *js.Object {
									if equalNum(get(response, "status"), 200) {
										stored := func(unused *js.Object) *js.Object {
											return response
										}
										return chain(call(runtime, "put", []*js.Object{path, call(response, "clone", []*js.Object{})}), stored)
									} else {
										return response
									}
								}
								return chain(network(source, base), fetched)
							}
						}
					}
					return chain(call(runtime, "match", []*js.Object{path}), matched)
				}
				return chain(call(globalValue("caches"), "open", []*js.Object{suffix(name, "-runtime")}), opened)
			} else {
				return ok(nilValue())
			}
		}
	}
}
func fetchRequestForStep(step, base *js.Object) *js.Object {
	req := get(step, "request")
	if truth(req) {
		return req
	}
	init := objectValue()
	if method := get(step, "method"); truth(method) {
		setProp(init, "method", method)
	}
	return makeHost("Request", []*js.Object{absolute(get(step, "path"), base), init})
}
func phase(step *js.Object, name *js.Object, items *js.Object, base *js.Object) *js.Object {
	entries := makeHost("Map", []*js.Object{})
	state := objectValue()
	add := func(item *js.Object) *js.Object {
		url := makeHost("URL", []*js.Object{get(item, "path"), base})
		original := get(url, "href")
		revision := get(item, "revision")
		if truth(revision) {
			call(get(url, "searchParams"), "set", []*js.Object{strVal("__WB_REVISION__"), revision})
		}
		key := get(url, "href")
		previous := call(entries, "get", []*js.Object{original})
		if (!missing(previous)) && (!same(previous, key)) {
			setProp(state, "conflict", boolean(true))
		}
		call(entries, "set", []*js.Object{original, key})
		return undef()
	}
	mapValues(items, add)
	if truth(get(state, "conflict")) {
		return bad(strVal("conflicting revisions"))
	} else {
		op := get(step, "phase")
		source := defaulted(get(step, "source"), strVal("alpha"))
		opened := func(cache *js.Object) *js.Object {
			if equalText(op, "install") {
				return install(entries, cache, source, base)
			} else {
				if equalText(op, "activate") {
					return activate(entries, cache, base)
				} else {
					if equalText(op, "message") {
						if equalText(get(step, "message"), "SKIP_WAITING") {
							skipped := func(unused *js.Object) *js.Object {
								return boolean(true)
							}
							return chain(skipWaiting(), skipped)
						} else {
							return boolean(false)
						}
					} else {
						req := fetchRequestForStep(step, base)
						method := get(req, "method")
						if !equalText(method, "GET") {
							return nilValue()
						} else {
							path := get(req, "url")
							received := func(response *js.Object) *js.Object {
								if nullish(response) {
									return nilValue()
								} else {
									body := func(text *js.Object) *js.Object {
										return pack([]*js.Object{get(response, "status"), text})
									}
									return chain(call(response, "text", []*js.Object{}), body)
								}
							}
							failed := func(unused *js.Object) *js.Object {
								return pack([]*js.Object{strVal("no-response")})
							}
							return recoverPromise(chain(strategy(entries, cache, name, path, source, base), received), failed)
						}
					}
				}
			}
		}
		return chain(call(globalValue("caches"), "open", []*js.Object{name}), opened)
	}
}
func run(inputValue *js.Object) *js.Object {
	base := absolute(strVal("experiments/library-matrix/libraries/workbox/"), get(globalValue("location"), "href"))
	if !missing(get(inputValue, "phase")) {
		task := phase(inputValue, get(inputValue, "name"), defaulted(get(inputValue, "manifest"), pack([]*js.Object{})), base)
		event := get(inputValue, "event")
		if truth(event) {
			call(event, "waitUntil", []*js.Object{task})
		}
		return task
	} else {
		name := uniqueName()
		state := objectValue()
		setProp(state, "manifest", pack([]*js.Object{}))
		results := pack([]*js.Object{})
		execute := func(step *js.Object) *js.Object {
			items := defaulted(get(step, "manifest"), get(state, "manifest"))
			setProp(state, "manifest", items)
			collectResult := func(result *js.Object) *js.Object {
				push(results, result)
				return undef()
			}
			return chain(phase(step, name, items, base), collectResult)
		}
		complete := func(unused *js.Object) *js.Object {
			return results
		}
		cleanup := func(unused *js.Object) *js.Object {
			caches := globalValue("caches")
			return call(globalValue("Promise"), "all", []*js.Object{pack([]*js.Object{call(caches, "delete", []*js.Object{name}), call(caches, "delete", []*js.Object{suffix(name, "-runtime")})})})
		}
		return always(chain(sequence(get(inputValue, "steps"), execute), complete), cleanup)
	}
}
func main() { js.Module.Get("exports").Set("run", run) }
