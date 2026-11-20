type j
@val external globals:j="globalThis"
@get_index external get:(j,string)=>j=""
@set_index external setProp:(j,string,j)=>unit=""
@get_index external at:(j,int)=>j=""
external cast:'a=>'b="%identity"
let globalValue=k=>get(globals,k)
let strVal=(s:string):j=>cast(s)
let nil=():j=>cast(Nullable.null)
let undef=():j=>cast(None)
let missing=x=>x===undef()
@val external truth:j=>bool="Boolean"
let equalText=(x,s)=>x===strVal(s)
@new external objectValue:unit=>j="Object"
let pack=(a:array<j>):j=>cast(a)
@send external push:(j,j)=>unit="push"
@scope("Reflect") @val external apply:(j,j,array<j>)=>j="apply"
let call=(x,k,args)=>apply(get(x,k),x,args)
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
let boolean=(v:bool):j=>cast(v)
let nullish=v=>v===nil()||missing(v)
let equalNum=(x,n)=>x===(cast(n):j)
@scope("Promise") @val external ok:j=>j="resolve"
@scope("Promise") @val external bad:j=>j="reject"
@send external recoverPromise:(j,j=>j)=>j="catch"
let suffix=(s,k)=>strVal((cast(s):string)++k)
@send external starts:(j,string)=>bool="startsWith"
@send external ends:(j,string)=>bool="endsWith"
@send external drop:(j,int)=>j="slice"
let relative=(s,base)=>drop(s,String.length(cast(base)))
@scope("Array") @val external collect:j=>j="from"
let badStatus=s=>(cast(s):int)>=400
let uniqueName=()=>strVal("mires-workbox-"++(cast(call(globalValue("crypto"),"randomUUID",[])):string))
@val external skipWaiting:unit=>j="skipWaiting"
@val external fetchRequest:j=>j="fetch"
let defaulted=(x,y)=>missing(x)?y:x
let same=(x,y)=>x===y

let absolute = (path: j, base: j): j => {
    get(makeHost("URL", [path, base]), "href")
}
let network = (source: j, base: j): j => {
    if equalText(source, "offline") {
        bad(strVal("offline"))
    } else {
        let req = makeHost("Request", [absolute(suffix(source, ".mjs"), base)])
        fetchRequest(req)
    }
}
let precacheKey = (entries: j, path: j, base: j): j => {
    let url = makeHost("URL", [path, base])
    let _ = setProp(url, "hash", strVal(""))
    let exact = call(entries, "get", [get(url, "href")])
    if (!missing(exact)) {
        exact
    } else {
        let params = get(url, "searchParams")
        let keys = collect(call(params, "keys", []))
        let ignore = (key: j): j => {
            if (equalText(key, "fbclid") || starts(key, "utm_")) {
                let _ = call(params, "delete", [key])
            }
            undef()
        }
        let _ = mapValues(keys, ignore)
        let stripped = call(entries, "get", [get(url, "href")])
        if (!missing(stripped)) {
            stripped
        } else {
            let original = get(url, "pathname")
            let _ = setProp(url, "pathname", suffix(original, "index.html"))
            let index = (ends(original, "/") ? call(entries, "get", [get(url, "href")]) : undef())
            if (!missing(index)) {
                index
            } else {
                let _ = setProp(url, "pathname", suffix(original, ".html"))
                call(entries, "get", [get(url, "href")])
            }
        }
    }
}
let install = (entries: j, cache: j, source: j, base: j): j => {
    let updated = pack([])
    let unchanged = pack([])
    let check = (entry: j): j => {
        let original = at(entry, 0)
        let key = at(entry, 1)
        let found = (hit: j): j => {
            if (!missing(hit)) {
                let _ = push(unchanged, relative(original, base))
                undef()
            } else {
                let received = (res: j): j => {
                    if badStatus(get(res, "status")) {
                        bad(strVal("bad-precaching-response"))
                    } else {
                        let stored = (unused: j): j => {
                            let _ = push(updated, relative(original, base))
                            undef()
                        }
                        chain(call(cache, "put", [key, call(res, "clone", [])]), stored)
                    }
                }
                chain(network(source, base), received)
            }
        }
        chain(call(cache, "match", [key]), found)
    }
    let complete = (unused: j): j => {
        pack([updated, unchanged])
    }
    chain(sequence(collect(call(entries, "entries", [])), check), complete)
}
let activate = (entries: j, cache: j, base: j): j => {
    let active = collect(call(entries, "values", []))
    let removed = pack([])
    let listKeys = (keys: j): j => {
        let remove = (req: j): j => {
            let path = get(req, "url")
            if truth(call(active, "includes", [path])) {
                undef()
            } else {
                let deleted = (unused: j): j => {
                    let _ = push(removed, relative(path, base))
                    undef()
                }
                chain(call(cache, "delete", [path]), deleted)
            }
        }
        let complete = (unused: j): j => {
            call(removed, "sort", [])
        }
        chain(sequence(keys, remove), complete)
    }
    chain(call(cache, "keys", []), listKeys)
}
let strategy = (entries: j, cache: j, name: j, path: j, source: j, base: j): j => {
    let key = precacheKey(entries, path, base)
    let route = relative(path, base)
    if (!missing(key)) {
        let precached = (hit: j): j => {
            (missing(hit) ? network(source, base) : hit)
        }
        chain(call(cache, "match", [key]), precached)
    } else {
        if starts(route, "network/") {
            network(source, base)
        } else {
            if (starts(route, "first/") || starts(route, "only/")) {
                let opened = (runtime: j): j => {
                    let matched = (hit: j): j => {
                        if (!missing(hit)) {
                            hit
                        } else {
                            if starts(route, "only/") {
                                bad(strVal("no-response"))
                            } else {
                                let fetched = (response: j): j => {
                                    if equalNum(get(response, "status"), 200) {
                                        let stored = (unused: j): j => {
                                            response
                                        }
                                        chain(call(runtime, "put", [path, call(response, "clone", [])]), stored)
                                    } else {
                                        response
                                    }
                                }
                                chain(network(source, base), fetched)
                            }
                        }
                    }
                    chain(call(runtime, "match", [path]), matched)
                }
                chain(call(globalValue("caches"), "open", [suffix(name, "-runtime")]), opened)
            } else {
                ok(nil())
            }
        }
    }
}
let fetchRequestForStep = (step, base) => {
    let req = get(step, "request")
    if truth(req) { req } else {
        let init = objectValue()
        let method = get(step, "method")
        if truth(method) { let _ = setProp(init, "method", method) }
        makeHost("Request", [absolute(get(step, "path"), base), init])
    }
}

let phase = (step: j, name: j, items: j, base: j): j => {
    let entries = makeHost("Map", [])
    let state = objectValue()
    let add = (item: j): j => {
        let url = makeHost("URL", [get(item, "path"), base])
        let original = get(url, "href")
        let revision = get(item, "revision")
        if truth(revision) {
            let _ = call(get(url, "searchParams"), "set", [strVal("__WB_REVISION__"), revision])
        }
        let key = get(url, "href")
        let previous = call(entries, "get", [original])
        if ((!missing(previous)) && (!same(previous, key))) {
            let _ = setProp(state, "conflict", boolean(true))
        }
        let _ = call(entries, "set", [original, key])
        undef()
    }
    let _ = mapValues(items, add)
    if truth(get(state, "conflict")) {
        bad(strVal("conflicting revisions"))
    } else {
        let op = get(step, "phase")
        let source = defaulted(get(step, "source"), strVal("alpha"))
        let opened = (cache: j): j => {
            if equalText(op, "install") {
                install(entries, cache, source, base)
            } else {
                if equalText(op, "activate") {
                    activate(entries, cache, base)
                } else {
                    if equalText(op, "message") {
                        if equalText(get(step, "message"), "SKIP_WAITING") {
                            let skipped = (unused: j): j => {
                                boolean(true)
                            }
                            chain(skipWaiting(), skipped)
                        } else {
                            boolean(false)
                        }
                    } else {
                        let req = fetchRequestForStep(step, base)
                        let method = get(req, "method")
                        if (!equalText(method, "GET")) {
                            nil()
                        } else {
                            let path = get(req, "url")
                            let received = (response: j): j => {
                                if nullish(response) {
                                    nil()
                                } else {
                                    let body = (text: j): j => {
                                        pack([get(response, "status"), text])
                                    }
                                    chain(call(response, "text", []), body)
                                }
                            }
                            let failed = (unused: j): j => {
                                pack([strVal("no-response")])
                            }
                            recoverPromise(chain(strategy(entries, cache, name, path, source, base), received), failed)
                        }
                    }
                }
            }
        }
        chain(call(globalValue("caches"), "open", [name]), opened)
    }
}
let run = (inputValue: j): j => {
    let base = absolute(strVal("experiments/library-matrix/libraries/workbox/"), get(globalValue("location"), "href"))
    if (!missing(get(inputValue, "phase"))) {
        let task = phase(inputValue, get(inputValue, "name"), defaulted(get(inputValue, "manifest"), pack([])), base)
        let event = get(inputValue, "event")
        if truth(event) {
            let _ = call(event, "waitUntil", [task])
        }
        task
    } else {
        let name = uniqueName()
        let state = objectValue()
        let _ = setProp(state, "manifest", pack([]))
        let results = pack([])
        let execute = (step: j): j => {
            let items = defaulted(get(step, "manifest"), get(state, "manifest"))
            let _ = setProp(state, "manifest", items)
            let collectResult = (result: j): j => {
                let _ = push(results, result)
                undef()
            }
            chain(phase(step, name, items, base), collectResult)
        }
        let complete = (unused: j): j => {
            results
        }
        let cleanup = (unused: j): j => {
            let caches = globalValue("caches")
            call(globalValue("Promise"), "all", [pack([call(caches, "delete", [name]), call(caches, "delete", [suffix(name, "-runtime")])])])
        }
        always(chain(sequence(get(inputValue, "steps"), execute), complete), cleanup)
    }
}
