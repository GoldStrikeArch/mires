type j
@val external globals:j="globalThis"
@get_index external get:(j,string)=>j=""
@set_index external setProp:(j,string,j)=>unit=""
@get_index external at:(j,int)=>j=""
external cast:'a=>'b="%identity"
let globalValue=k=>get(globals,k)
let strVal=(s:string):j=>cast(s)
let num=(i:int):j=>cast(i)
let nil=():j=>cast(Nullable.null)
let undef=():j=>cast(None)
let missing=x=>x===undef()
@val external truth:j=>bool="Boolean"
let equalText=(x,s)=>x===strVal(s)
let equalNum=(x,i)=>x===num(i)
@new external objectValue:unit=>j="Object"
let pack=(a:array<j>):j=>cast(a)
@send external push:(j,j)=>unit="push"
@scope("Reflect") @val external apply:(j,j,array<j>)=>j="apply"
let call=(x,k,args)=>apply(get(x,k),x,args)
let callback=(f:j=>j):j=>cast(f)
@new external promise:((j,j)=>j)=>j="Promise"
let settle=(f,v)=>{let cb:j=>unit=cast(f);cb(v)}
@send external chain:(j,j=>j)=>j="then"
@send external always:(j,j=>j)=>j="finally"
@send external mapValues:(j,j=>j)=>j="map"
@scope("Promise") @val external resolved:unit=>j="resolve"
let sequence=(xs,f)=>{
  let a:array<j>=cast(xs)
  a->Array.reduce(resolved(),(p,v)=>chain(p,_=>f(v)))
}
@scope("Reflect") @val external construct:(j,array<j>)=>j="construct"
let makeHost=(k,args)=>construct(globalValue(k),args)
let boolean=(b:bool):j=>cast(b)
let nullish=x=>x===nil()||missing(x)
let header=(h,k)=>call(h,"get",[strVal(k)])
@scope("Promise") @val external ok:j=>j="resolve"
@scope("Promise") @val external bad:j=>j="reject"
@send external recoverPromise:(j,j=>j)=>j="catch"
@send external handled:(j,j=>j,j=>j)=>j="then"
@val external timer:(j,j)=>j="setTimeout"
@val external stopTimer:j=>unit="clearTimeout"
let waitTime=ms=>promise((resolve,_)=>{let _=timer(resolve,ms);undef()})
@scope("Number") @val external isNumeric:j=>bool="isFinite"
let lastOrIndex=(xs,i)=>{let a:array<j>=cast(xs);at(xs,Math.Int.min(cast(i),Array.length(a)-1))}
let smaller=(a,b)=>(cast(a):float)<(cast(b):float)
let increment=a=>num((cast(a):int)+1)
let lesser=(a,b)=>smaller(a,b)?a:b
let retryTag=a=>strVal("retry-"++Int.toString(cast(a)))
@scope("Math") @val external pow:(float,float)=>float="pow"
let exponential=a=>cast(300.0 *. pow(2.0,cast(a)))
@send external test:(RegExp.t,string)=>bool="test"
@new external regex:(string,string)=>RegExp.t="RegExp"
@val external parseNumber:string=>float="Number"
let mimePattern=regex("^\\s*[^;]*\u002f(?:[^;]*[+.\\-])?json\\s*(?:;|$)","i")
let unsignedDigits=regex("^[0-9]+$","")
let jsonType=a=> !nullish(a) && test(mimePattern,cast(a))
let parseAfter=a=>{let raw=String.trim(cast(a));test(unsignedDigits,raw) ? cast(parseNumber(raw)*.1000.0) : nil()}
let canonicalMethod = method => switch String.toLowerCase(method) {
| "get" | "post" | "put" | "patch" | "head" | "delete" | "query" | "options" | "trace" => String.toUpperCase(method)
| _ => method
}
let allowsMethod=(c,m)=>{
  let xs=get(c,"methods")
  let methods:array<string>=missing(xs)?["get","put","head","delete","options","trace","query"]:cast(xs)
  let target=canonicalMethod(cast(m))
  methods->Array.some(x=>canonicalMethod(x)==target)
}
let allowsStatus=(c,s)=>{
 let xs=get(c,"statusCodes")
 let codes:array<int>=missing(xs)?[408,413,429,500,502,503,504]:cast(xs)
 codes->Array.includes(cast(s))
}

let makeError = (name: j, message: j): j => {
    let value = makeHost("Error", [message])
    let _ = setProp(value, "name", name)
    value
}
let buildRequest = (c: j): j => {
    let init = objectValue()
    let headers = makeHost("Headers", [get(c, "headers")])
    let method = get(c, "method")
    let _ = setProp(init, "method", strVal(nullish(method) ? "GET" : canonicalMethod(cast(method))))
    let _ = setProp(init, "headers", headers)
    let payload = get(c, "json")
    if (!missing(payload)) {
        let _ = setProp(init, "body", call(globalValue("JSON"), "stringify", [payload]))
        if (nullish(header(headers, "content-type"))) {
            let _ = call(headers, "set", [strVal("content-type"), strVal("application/json")])
        }
    }
    if (!truth(header(headers, "accept"))) {
        let mime = (equalText(get(c, "read"), "json") ? strVal("application/json") : strVal("text/*"))
        let _ = call(headers, "set", [strVal("accept"), mime])
    }
    makeHost("Request", [get(c, "url"), init])
}
let fixtureFetch = (c: j, req: j, user: j, count: j, trace: j): j => {
    let h = get(req, "headers")
    let respond = (body: j): j => {
        let _ = push(trace, pack([strVal("fetch"), get(req, "method"), get(req, "url"), header(h, "accept"), header(h, "content-type"), header(h, "x-matrix"), body]))
        if equalText(get(c, "abort"), "fetch") {
            let _ = call(user, "abort", [makeError(strVal("AbortError"), strVal("cancelled"))])
        }
        let signal = get(req, "signal")
        if truth(get(signal, "aborted")) {
            bad(get(signal, "reason"))
        } else {
            let reply = lastOrIndex(get(c, "replies"), count)
            if equalText(get(reply, "kind"), "network") {
                bad(makeHost("TypeError", [strVal("Failed to fetch")]))
            } else {
                if equalText(get(reply, "kind"), "stall") {
                    let stall = (resolve: j, reject: j): j => {
                        let aborted = (event: j): j => {
                            let _ = settle(reject, get(signal, "reason"))
                            undef()
                        }
                        let options = objectValue()
                        let _ = setProp(options, "once", boolean(true))
                        let _ = call(signal, "addEventListener", [strVal("abort"), callback(aborted), options])
                        undef()
                    }
                    promise(stall)
                } else {
                    let options = objectValue()
                    let _ = setProp(options, "status", get(reply, "status"))
                    let _ = setProp(options, "headers", get(reply, "headers"))
                    makeHost("Response", [get(reply, "body"), options])
                }
            }
        }
    }
    chain(call(req, "text", []), respond)
}
let dispatch = (c: j, base: j, user: j, count: j, trace: j): j => {
    let ctl = makeHost("AbortController", [])
    let signal = call(globalValue("AbortSignal"), "any", [pack([get(user, "signal"), get(ctl, "signal")])])
    let init = objectValue()
    let _ = setProp(init, "signal", signal)
    let req = makeHost("Request", [call(base, "clone", []), init])
    let fetching = fixtureFetch(c, req, user, count, trace)
    let timeout = get(c, "timeout")
    if isNumeric(timeout) {
        let holder = objectValue()
        let executor = (resolve: j, reject: j): j => {
            let expiry = (unused: j): j => {
                let _ = settle(reject, makeError(strVal("TimeoutError"), strVal("Request timed out")))
                let _ = call(ctl, "abort", [])
                undef()
            }
            let _ = setProp(holder, "timer", timer(callback(expiry), timeout))
            undef()
        }
        let expiryTask = promise(executor)
        let clear = (unused: j): j => {
            let _ = stopTimer(get(holder, "timer"))
            undef()
        }
        always(call(globalValue("Promise"), "race", [pack([fetching, expiryTask])]), clear)
    } else {
        fetching
    }
}
let parseJson = (body: j): j => {
    call(globalValue("JSON"), "parse", [body])
}
let errorData = (response: j): j => {
    let h = get(response, "headers")
    let read = (body: j): j => {
        if (!truth(body)) {
            nil()
        } else {
            if jsonType(header(h, "content-type")) {
                let invalid = (unused: j): j => {
                    nil()
                }
                recoverPromise(chain(ok(body), parseJson), invalid)
            } else {
                body
            }
        }
    }
    chain(call(response, "text", []), read)
}
let retryDelay = (c: j, method: j, count: j, fault: j, trace: j): j => {
    let name = at(fault, 0)
    let status = at(fault, 1)
    let raw = at(fault, 3)
    let eligible = (allowsMethod(c, method) && smaller(count, get(c, "limit")))
    let serverStatus = (equalNum(status, 413) || equalNum(status, 429) || equalNum(status, 503))
    let http = equalText(name, "HTTPError")
    let retriable = (http ? allowsStatus(c, status) : (equalText(name, "NetworkError") || (equalText(name, "TimeoutError") && truth(get(c, "retryOnTimeout")))))
    if ((!eligible) || (!retriable) || (http && equalNum(status, 413) && (!truth(raw)))) {
        nil()
    } else {
        let after = ((http && serverStatus && truth(raw)) ? parseAfter(raw) : nil())
        if (!nullish(after)) {
            lesser(after, get(c, "maxRetryAfter"))
        } else {
            let ms = exponential(count)
            let _ = push(trace, pack([strVal("backoff"), increment(count), ms]))
            lesser(ms, get(c, "backoffLimit"))
        }
    }
}
let outputError = (trace: j, count: j, f: j, hook: j): j => {
    if truth(hook) {
        let _ = push(trace, pack([strVal("beforeError"), count, at(f, 0)]))
    }
    pack([trace, pack([strVal("error"), at(f, 0), at(f, 1), at(f, 2)])])
}
let rec attempt = (c: j, base: j, user: j, count: j, trace: j): j => {
    let onFailure = (f: j): j => {
        let delay = retryDelay(c, get(base, "method"), count, f, trace)
        if nullish(delay) {
            outputError(trace, count, f, boolean(true))
        } else {
            let proceed = (unused: j): j => {
                let following = increment(count)
                let _ = push(trace, pack([strVal("beforeRetry"), following, at(f, 0)]))
                let _ = call(get(base, "headers"), "set", [strVal("x-matrix"), retryTag(following)])
                if equalText(get(c, "abort"), "retry") {
                    let _ = call(user, "abort", [makeError(strVal("AbortError"), strVal("cancelled"))])
                }
                attempt(c, base, user, following, trace)
            }
            chain(waitTime(delay), proceed)
        }
    }
    let success = (response: j): j => {
        let status = get(response, "status")
        let _ = push(trace, pack([strVal("afterResponse"), count, status]))
        if ((!truth(get(response, "ok"))) && truth(get(c, "throwHttpErrors"))) {
            let failed = (data: j): j => {
                onFailure(pack([strVal("HTTPError"), status, data, header(get(response, "headers"), "retry-after")]))
            }
            chain(errorData(response), failed)
        } else {
            let result = (payload: j): j => {
                pack([trace, pack([strVal("ok"), payload])])
            }
            let body = call(response, "text", [])
            if equalText(get(c, "read"), "json") {
                let invalid = (unused: j): j => {
                    outputError(trace, count, pack([strVal("SyntaxError"), nil(), nil(), nil()]), boolean(false))
                }
                recoverPromise(chain(chain(body, parseJson), result), invalid)
            } else {
                chain(body, result)
            }
        }
    }
    let failure = (error: j): j => {
        let name = get(error, "name")
        let wrapped = ((equalText(name, "TypeError") && equalText(get(error, "message"), "Failed to fetch")) ? strVal("NetworkError") : name)
        onFailure(pack([wrapped, nil(), nil(), nil()]))
    }
    handled(dispatch(c, base, user, count, trace), success, failure)
}
let requestCase = (c: j): j => {
    let trace = pack([pack([strVal("init")])])
    let user = makeHost("AbortController", [])
    if equalText(get(c, "abort"), "before") {
        let _ = call(user, "abort", [makeError(strVal("AbortError"), strVal("cancelled"))])
    }
    let ready = (base: j): j => {
        if truth(get(get(user, "signal"), "aborted")) {
            ok(outputError(trace, num(0), pack([strVal("AbortError"), nil(), nil(), nil()]), boolean(true)))
        } else {
            let _ = push(trace, pack([strVal("beforeRequest"), num(0)]))
            let _ = call(get(base, "headers"), "set", [strVal("x-matrix"), strVal("initial")])
            attempt(c, base, user, num(0), trace)
        }
    }
    let invalid = error => outputError(trace, num(0), pack([get(error, "name"), nil(), nil(), nil()]), boolean(false))
    handled(chain(ok(c), buildRequest), ready, invalid)
}
let run = (inputValue: j): j => {
    let results = pack([])
    let evaluate = (c: j): j => {
        let save = (result: j): j => {
            let _ = push(results, result)
            undef()
        }
        chain(requestCase(c), save)
    }
    let complete = (unused: j): j => {
        results
    }
    chain(sequence(get(inputValue, "cases"), evaluate), complete)
}
