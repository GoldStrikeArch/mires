package main

import (
	"github.com/gopherjs/gopherjs/js"
	"math"
	"regexp"
	"strconv"
	"strings"
)

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
func makeHost(k string, args []*js.Object) *js.Object {
	return globalValue("Reflect").Call("construct", globalValue(k), pack(args))
}
func nullish(x *js.Object) bool                { return x == nil || x == js.Undefined }
func boolean(v bool) *js.Object                { return globalValue("Boolean").Invoke(v) }
func header(x *js.Object, k string) *js.Object { return x.Call("get", k) }
func ok(v *js.Object) *js.Object               { return globalValue("Promise").Call("resolve", v) }
func bad(v *js.Object) *js.Object              { return globalValue("Promise").Call("reject", v) }
func recoverPromise(p *js.Object, f func(*js.Object) *js.Object) *js.Object {
	return p.Call("catch", f)
}
func handled(p *js.Object, s, f func(*js.Object) *js.Object) *js.Object { return p.Call("then", s, f) }
func timer(f, ms *js.Object) *js.Object                                 { return js.Global.Call("setTimeout", f, ms) }
func stopTimer(id *js.Object)                                           { js.Global.Call("clearTimeout", id) }
func waitTime(ms *js.Object) *js.Object {
	return promise(func(resolve, reject *js.Object) *js.Object { timer(resolve, ms); return undef() })
}
func isNumeric(x *js.Object) bool {
	return x != nil && x != js.Undefined && globalValue("Object").Get("prototype").Get("toString").Call("call", x).String() == "[object Number]"
}
func lastOrIndex(xs, i *js.Object) *js.Object { return xs.Index(min(i.Int(), xs.Length()-1)) }
func smaller(a, b *js.Object) bool            { return a.Float() < b.Float() }
func increment(a *js.Object) *js.Object       { return num(a.Int() + 1) }
func lesser(a, b *js.Object) *js.Object {
	if smaller(a, b) {
		return a
	}
	return b
}
func retryTag(a *js.Object) *js.Object { return strVal("retry-" + strconv.Itoa(a.Int())) }
func exponential(a *js.Object) *js.Object {
	return globalValue("Number").Invoke(300 * math.Exp2(a.Float()))
}
func jsonType(a *js.Object) bool {
	if nullish(a) {
		return false
	}
	m := strings.ToLower(strings.TrimSpace(strings.SplitN(a.String(), ";", 2)[0]))
	return strings.Contains(m, "/") && (strings.HasSuffix(m, "/json") || strings.HasSuffix(m, "+json") || strings.HasSuffix(m, "-json") || strings.HasSuffix(m, ".json"))
}

var digits = regexp.MustCompile("^[0-9]+$")

func parseAfter(a *js.Object) *js.Object {
	s := strings.TrimSpace(a.String())
	if !digits.MatchString(s) {
		return nil
	}
	return globalValue("Number").Invoke(1000 * globalValue("Number").Invoke(s).Float())
}
func canonicalMethod(method string) string {
	switch strings.ToLower(method) {
	case "get", "post", "put", "patch", "head", "delete", "query", "options", "trace":
		return strings.ToUpper(method)
	}
	return method
}
func requestMethod(c *js.Object) *js.Object {
	m := get(c, "method")
	if nullish(m) {
		return strVal("GET")
	}
	return strVal(canonicalMethod(m.String()))
}
func allowsMethod(c, m *js.Object) bool {
	ms := c.Get("methods")
	k := canonicalMethod(m.String())
	if ms == js.Undefined {
		switch k {
		case "GET", "PUT", "HEAD", "DELETE", "OPTIONS", "TRACE", "QUERY":
			return true
		}
		return false
	}
	for i := 0; i < ms.Length(); i++ {
		if canonicalMethod(ms.Index(i).String()) == k {
			return true
		}
	}
	return false
}
func allowsStatus(c, s *js.Object) bool {
	v := c.Get("statusCodes")
	if v == js.Undefined {
		switch s.Int() {
		case 408, 413, 429, 500, 502, 503, 504:
			return true
		}
		return false
	}
	return v.Call("includes", s).Bool()
}

func makeError(name *js.Object, message *js.Object) *js.Object {
	value := makeHost("Error", []*js.Object{message})
	setProp(value, "name", name)
	return value
}
func buildRequest(c *js.Object) *js.Object {
	init := objectValue()
	headers := makeHost("Headers", []*js.Object{get(c, "headers")})
	setProp(init, "method", requestMethod(c))
	setProp(init, "headers", headers)
	payload := get(c, "json")
	if !missing(payload) {
		setProp(init, "body", call(globalValue("JSON"), "stringify", []*js.Object{payload}))
		if nullish(header(headers, "content-type")) {
			call(headers, "set", []*js.Object{strVal("content-type"), strVal("application/json")})
		}
	}
	if !truth(header(headers, "accept")) {
		mime := (func() *js.Object {
			if equalText(get(c, "read"), "json") {
				return strVal("application/json")
			}
			return strVal("text/*")
		}())
		call(headers, "set", []*js.Object{strVal("accept"), mime})
	}
	return makeHost("Request", []*js.Object{get(c, "url"), init})
}
func fixtureFetch(c *js.Object, req *js.Object, user *js.Object, count *js.Object, trace *js.Object) *js.Object {
	h := get(req, "headers")
	respond := func(body *js.Object) *js.Object {
		push(trace, pack([]*js.Object{strVal("fetch"), get(req, "method"), get(req, "url"), header(h, "accept"), header(h, "content-type"), header(h, "x-matrix"), body}))
		if equalText(get(c, "abort"), "fetch") {
			call(user, "abort", []*js.Object{makeError(strVal("AbortError"), strVal("cancelled"))})
		}
		signal := get(req, "signal")
		if truth(get(signal, "aborted")) {
			return bad(get(signal, "reason"))
		} else {
			reply := lastOrIndex(get(c, "replies"), count)
			if equalText(get(reply, "kind"), "network") {
				return bad(makeHost("TypeError", []*js.Object{strVal("Failed to fetch")}))
			} else {
				if equalText(get(reply, "kind"), "stall") {
					stall := func(resolve *js.Object, reject *js.Object) *js.Object {
						aborted := func(event *js.Object) *js.Object {
							settle(reject, get(signal, "reason"))
							return undef()
						}
						options := objectValue()
						setProp(options, "once", boolean(true))
						call(signal, "addEventListener", []*js.Object{strVal("abort"), callback(aborted), options})
						return undef()
					}
					return promise(stall)
				} else {
					options := objectValue()
					setProp(options, "status", get(reply, "status"))
					setProp(options, "headers", get(reply, "headers"))
					return makeHost("Response", []*js.Object{get(reply, "body"), options})
				}
			}
		}
	}
	return chain(call(req, "text", []*js.Object{}), respond)
}
func dispatch(c *js.Object, base *js.Object, user *js.Object, count *js.Object, trace *js.Object) *js.Object {
	ctl := makeHost("AbortController", []*js.Object{})
	signal := call(globalValue("AbortSignal"), "any", []*js.Object{pack([]*js.Object{get(user, "signal"), get(ctl, "signal")})})
	init := objectValue()
	setProp(init, "signal", signal)
	req := makeHost("Request", []*js.Object{call(base, "clone", []*js.Object{}), init})
	fetching := fixtureFetch(c, req, user, count, trace)
	timeout := get(c, "timeout")
	if isNumeric(timeout) {
		holder := objectValue()
		executor := func(resolve *js.Object, reject *js.Object) *js.Object {
			expiry := func(unused *js.Object) *js.Object {
				settle(reject, makeError(strVal("TimeoutError"), strVal("Request timed out")))
				call(ctl, "abort", []*js.Object{})
				return undef()
			}
			setProp(holder, "timer", timer(callback(expiry), timeout))
			return undef()
		}
		expiryTask := promise(executor)
		clear := func(unused *js.Object) *js.Object {
			stopTimer(get(holder, "timer"))
			return undef()
		}
		return always(call(globalValue("Promise"), "race", []*js.Object{pack([]*js.Object{fetching, expiryTask})}), clear)
	} else {
		return fetching
	}
}
func parseJson(body *js.Object) *js.Object {
	return call(globalValue("JSON"), "parse", []*js.Object{body})
}
func errorData(response *js.Object) *js.Object {
	h := get(response, "headers")
	read := func(body *js.Object) *js.Object {
		if !truth(body) {
			return nilValue()
		} else {
			if jsonType(header(h, "content-type")) {
				invalid := func(unused *js.Object) *js.Object {
					return nilValue()
				}
				return recoverPromise(chain(ok(body), parseJson), invalid)
			} else {
				return body
			}
		}
	}
	return chain(call(response, "text", []*js.Object{}), read)
}
func retryDelay(c *js.Object, method *js.Object, count *js.Object, fault *js.Object, trace *js.Object) *js.Object {
	name := at(fault, 0)
	status := at(fault, 1)
	raw := at(fault, 3)
	eligible := (allowsMethod(c, method) && smaller(count, get(c, "limit")))
	serverStatus := (equalNum(status, 413) || equalNum(status, 429) || equalNum(status, 503))
	http := equalText(name, "HTTPError")
	retriable := (func() bool {
		if http {
			return allowsStatus(c, status)
		}
		return (equalText(name, "NetworkError") || (equalText(name, "TimeoutError") && truth(get(c, "retryOnTimeout"))))
	}())
	if (!eligible) || (!retriable) || (http && equalNum(status, 413) && (!truth(raw))) {
		return nilValue()
	} else {
		after := (func() *js.Object {
			if http && serverStatus && truth(raw) {
				return parseAfter(raw)
			}
			return nilValue()
		}())
		if !nullish(after) {
			return lesser(after, get(c, "maxRetryAfter"))
		} else {
			ms := exponential(count)
			push(trace, pack([]*js.Object{strVal("backoff"), increment(count), ms}))
			return lesser(ms, get(c, "backoffLimit"))
		}
	}
}
func outputError(trace *js.Object, count *js.Object, f *js.Object, hook *js.Object) *js.Object {
	if truth(hook) {
		push(trace, pack([]*js.Object{strVal("beforeError"), count, at(f, 0)}))
	}
	return pack([]*js.Object{trace, pack([]*js.Object{strVal("error"), at(f, 0), at(f, 1), at(f, 2)})})
}
func attempt(c *js.Object, base *js.Object, user *js.Object, count *js.Object, trace *js.Object) *js.Object {
	onFailure := func(f *js.Object) *js.Object {
		delay := retryDelay(c, get(base, "method"), count, f, trace)
		if nullish(delay) {
			return outputError(trace, count, f, boolean(true))
		} else {
			proceed := func(unused *js.Object) *js.Object {
				following := increment(count)
				push(trace, pack([]*js.Object{strVal("beforeRetry"), following, at(f, 0)}))
				call(get(base, "headers"), "set", []*js.Object{strVal("x-matrix"), retryTag(following)})
				if equalText(get(c, "abort"), "retry") {
					call(user, "abort", []*js.Object{makeError(strVal("AbortError"), strVal("cancelled"))})
				}
				return attempt(c, base, user, following, trace)
			}
			return chain(waitTime(delay), proceed)
		}
	}
	success := func(response *js.Object) *js.Object {
		status := get(response, "status")
		push(trace, pack([]*js.Object{strVal("afterResponse"), count, status}))
		if (!truth(get(response, "ok"))) && truth(get(c, "throwHttpErrors")) {
			failed := func(data *js.Object) *js.Object {
				return onFailure(pack([]*js.Object{strVal("HTTPError"), status, data, header(get(response, "headers"), "retry-after")}))
			}
			return chain(errorData(response), failed)
		} else {
			result := func(payload *js.Object) *js.Object {
				return pack([]*js.Object{trace, pack([]*js.Object{strVal("ok"), payload})})
			}
			body := call(response, "text", []*js.Object{})
			if equalText(get(c, "read"), "json") {
				invalid := func(unused *js.Object) *js.Object {
					return outputError(trace, count, pack([]*js.Object{strVal("SyntaxError"), nilValue(), nilValue(), nilValue()}), boolean(false))
				}
				return recoverPromise(chain(chain(body, parseJson), result), invalid)
			} else {
				return chain(body, result)
			}
		}
	}
	failure := func(error *js.Object) *js.Object {
		name := get(error, "name")
		wrapped := (func() *js.Object {
			if equalText(name, "TypeError") && equalText(get(error, "message"), "Failed to fetch") {
				return strVal("NetworkError")
			}
			return name
		}())
		return onFailure(pack([]*js.Object{wrapped, nilValue(), nilValue(), nilValue()}))
	}
	return handled(dispatch(c, base, user, count, trace), success, failure)
}
func requestCase(c *js.Object) *js.Object {
	trace := pack([]*js.Object{pack([]*js.Object{strVal("init")})})
	user := makeHost("AbortController", []*js.Object{})
	if equalText(get(c, "abort"), "before") {
		call(user, "abort", []*js.Object{makeError(strVal("AbortError"), strVal("cancelled"))})
	}
	ready := func(base *js.Object) *js.Object {
		if truth(get(get(user, "signal"), "aborted")) {
			return ok(outputError(trace, num(0), pack([]*js.Object{strVal("AbortError"), nilValue(), nilValue(), nilValue()}), boolean(true)))
		} else {
			push(trace, pack([]*js.Object{strVal("beforeRequest"), num(0)}))
			call(get(base, "headers"), "set", []*js.Object{strVal("x-matrix"), strVal("initial")})
			return attempt(c, base, user, num(0), trace)
		}
	}
	invalid := func(err *js.Object) *js.Object {
		return outputError(trace, num(0), pack([]*js.Object{get(err, "name"), nilValue(), nilValue(), nilValue()}), boolean(false))
	}
	return handled(chain(ok(c), buildRequest), ready, invalid)
}
func run(inputValue *js.Object) *js.Object {
	results := pack([]*js.Object{})
	evaluate := func(c *js.Object) *js.Object {
		save := func(result *js.Object) *js.Object {
			push(results, result)
			return undef()
		}
		return chain(requestCase(c), save)
	}
	complete := func(unused *js.Object) *js.Object {
		return results
	}
	return chain(sequence(get(inputValue, "cases"), evaluate), complete)
}
func main() { js.Module.Get("exports").Set("run", run) }
