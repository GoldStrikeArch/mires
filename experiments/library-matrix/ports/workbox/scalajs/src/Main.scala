package workbox
import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

object Main {
type J=js.Dynamic
private def globalValue(k:String):J=js.Dynamic.global.globalThis.selectDynamic(k)
private def get(x:J,k:String):J=x.selectDynamic(k)
private def setProp(x:J,k:String,v:J):Unit=x.updateDynamic(k)(v)
private def at(x:J,i:Int):J=x.asInstanceOf[js.Array[J]](i)
private def missing(x:J):Boolean=js.isUndefined(x)
private def truth(x:J):Boolean=js.Dynamic.global.Boolean(x).asInstanceOf[Boolean]
private def equalText(x:J,s:String):Boolean=x.asInstanceOf[String]==s
private def equalNum(x:J,i:Int):Boolean=x.asInstanceOf[Int]==i
private def strVal(s:String):J=s.asInstanceOf[J]
private def nil():J=null.asInstanceOf[J]
private def undef():J=js.undefined.asInstanceOf[J]
private def objectValue():J=js.Dynamic.literal()
private def pack(values:js.Array[J]):J=values.asInstanceOf[J]
private def push(a:J,v:J):Unit={a.push(v);()}
private def call(x:J,k:String,args:js.Array[J]):J=js.Dynamic.global.Reflect.applyDynamic("apply")(get(x,k),x,args)
private def callback(f:J=>J):J=js.Any.fromFunction1(f).asInstanceOf[J]
private def chain(p:J,f:J=>J):J=p.applyDynamic("then")(callback(f))
private def always(p:J,f:J=>J):J=p.applyDynamic("finally")(callback(f))
private def mapValues(xs:J,f:J=>J):J=xs.map(callback(f))
private def sequence(xs:J,f:J=>J):J=xs.asInstanceOf[js.Array[J]].foldLeft(js.Dynamic.global.Promise.resolve())((p,v)=>chain(p,_=>f(v)))
private def makeHost(k:String,args:js.Array[J]):J=js.Dynamic.global.Reflect.construct(globalValue(k),args)
private def nullish(v:J)=v==null || missing(v)
private def boolean(b:Boolean):J=b.asInstanceOf[J]
private def ok(v:J):J=js.Dynamic.global.Promise.resolve(v)
private def bad(v:J):J=js.Dynamic.global.Promise.reject(v)
private def recoverPromise(p:J,f:J=>J):J=p.applyDynamic("catch")(callback(f))
private def suffix(s:J,tail:String)=strVal(s.asInstanceOf[String]+tail)
private def relative(url:J,base:J)=strVal(url.asInstanceOf[String].stripPrefix(base.asInstanceOf[String]))
private def starts(s:J,k:String)=s.asInstanceOf[String].startsWith(k)
private def ends(s:J,k:String)=s.asInstanceOf[String].endsWith(k)
private def collect(it:J):J=js.Dynamic.global.Array.from(it)
private def badStatus(s:J)=s.asInstanceOf[Int]>=400
private def uniqueName():J=strVal("mires-workbox-"+js.Dynamic.global.crypto.randomUUID().asInstanceOf[String])
private def skipWaiting():J=js.Dynamic.global.skipWaiting()
private def fetchRequest(req:J):J=js.Dynamic.global.fetch(req)
private def defaulted(x:J,y:J):J=if(missing(x)) y else x
private def same(x:J,y:J)=js.Dynamic.global.Object.is(x,y).asInstanceOf[Boolean]

def absolute(path: J, base: J): J = {
    get(makeHost("URL", js.Array[J](path, base)), "href")
}
def network(source: J, base: J): J = {
    if (equalText(source, "offline")) {
        bad(strVal("offline"))
    } else {
        val req = makeHost("Request", js.Array[J](absolute(suffix(source, ".mjs"), base)))
        fetchRequest(req)
    }
}
def precacheKey(entries: J, path: J, base: J): J = {
    val url = makeHost("URL", js.Array[J](path, base))
    setProp(url, "hash", strVal(""))
    val exact = call(entries, "get", js.Array[J](get(url, "href")))
    if ((!missing(exact))) {
        exact
    } else {
        val params = get(url, "searchParams")
        val keys = collect(call(params, "keys", js.Array[J]()))
        def ignore(key: J): J = {
            if ((equalText(key, "fbclid") || starts(key, "utm_"))) {
                call(params, "delete", js.Array[J](key))
            }
            undef()
        }
        mapValues(keys, ignore)
        val stripped = call(entries, "get", js.Array[J](get(url, "href")))
        if ((!missing(stripped))) {
            stripped
        } else {
            val original = get(url, "pathname")
            setProp(url, "pathname", suffix(original, "index.html"))
            val index = (if (ends(original, "/")) call(entries, "get", js.Array[J](get(url, "href"))) else undef())
            if ((!missing(index))) {
                index
            } else {
                setProp(url, "pathname", suffix(original, ".html"))
                call(entries, "get", js.Array[J](get(url, "href")))
            }
        }
    }
}
def install(entries: J, cache: J, source: J, base: J): J = {
    val updated = pack(js.Array[J]())
    val unchanged = pack(js.Array[J]())
    def check(entry: J): J = {
        val original = at(entry, 0)
        val key = at(entry, 1)
        def found(hit: J): J = {
            if ((!missing(hit))) {
                push(unchanged, relative(original, base))
                undef()
            } else {
                def received(res: J): J = {
                    if (badStatus(get(res, "status"))) {
                        bad(strVal("bad-precaching-response"))
                    } else {
                        def stored(unused: J): J = {
                            push(updated, relative(original, base))
                            undef()
                        }
                        chain(call(cache, "put", js.Array[J](key, call(res, "clone", js.Array[J]()))), stored)
                    }
                }
                chain(network(source, base), received)
            }
        }
        chain(call(cache, "match", js.Array[J](key)), found)
    }
    def complete(unused: J): J = {
        pack(js.Array[J](updated, unchanged))
    }
    chain(sequence(collect(call(entries, "entries", js.Array[J]())), check), complete)
}
def activate(entries: J, cache: J, base: J): J = {
    val active = collect(call(entries, "values", js.Array[J]()))
    val removed = pack(js.Array[J]())
    def listKeys(keys: J): J = {
        def remove(req: J): J = {
            val path = get(req, "url")
            if (truth(call(active, "includes", js.Array[J](path)))) {
                undef()
            } else {
                def deleted(unused: J): J = {
                    push(removed, relative(path, base))
                    undef()
                }
                chain(call(cache, "delete", js.Array[J](path)), deleted)
            }
        }
        def complete(unused: J): J = {
            call(removed, "sort", js.Array[J]())
        }
        chain(sequence(keys, remove), complete)
    }
    chain(call(cache, "keys", js.Array[J]()), listKeys)
}
def strategy(entries: J, cache: J, name: J, path: J, source: J, base: J): J = {
    val key = precacheKey(entries, path, base)
    val route = relative(path, base)
    if ((!missing(key))) {
        def precached(hit: J): J = {
            (if (missing(hit)) network(source, base) else hit)
        }
        chain(call(cache, "match", js.Array[J](key)), precached)
    } else {
        if (starts(route, "network/")) {
            network(source, base)
        } else {
            if ((starts(route, "first/") || starts(route, "only/"))) {
                def opened(runtime: J): J = {
                    def matched(hit: J): J = {
                        if ((!missing(hit))) {
                            hit
                        } else {
                            if (starts(route, "only/")) {
                                bad(strVal("no-response"))
                            } else {
                                def fetched(response: J): J = {
                                    if (equalNum(get(response, "status"), 200)) {
                                        def stored(unused: J): J = {
                                            response
                                        }
                                        chain(call(runtime, "put", js.Array[J](path, call(response, "clone", js.Array[J]()))), stored)
                                    } else {
                                        response
                                    }
                                }
                                chain(network(source, base), fetched)
                            }
                        }
                    }
                    chain(call(runtime, "match", js.Array[J](path)), matched)
                }
                chain(call(globalValue("caches"), "open", js.Array[J](suffix(name, "-runtime"))), opened)
            } else {
                ok(nil())
            }
        }
    }
}
def fetchRequestForStep(step: J, base: J): J = {
    val req = get(step, "request")
    if (truth(req)) req else {
        val init = objectValue()
        val method = get(step, "method")
        if (truth(method)) setProp(init, "method", method)
        makeHost("Request", js.Array[J](absolute(get(step, "path"), base), init))
    }
}

def phase(step: J, name: J, items: J, base: J): J = {
    val entries = makeHost("Map", js.Array[J]())
    val state = objectValue()
    def add(item: J): J = {
        val url = makeHost("URL", js.Array[J](get(item, "path"), base))
        val original = get(url, "href")
        val revision = get(item, "revision")
        if (truth(revision)) {
            call(get(url, "searchParams"), "set", js.Array[J](strVal("__WB_REVISION__"), revision))
        }
        val key = get(url, "href")
        val previous = call(entries, "get", js.Array[J](original))
        if (((!missing(previous)) && (!same(previous, key)))) {
            setProp(state, "conflict", boolean(true))
        }
        call(entries, "set", js.Array[J](original, key))
        undef()
    }
    mapValues(items, add)
    if (truth(get(state, "conflict"))) {
        bad(strVal("conflicting revisions"))
    } else {
        val op = get(step, "phase")
        val source = defaulted(get(step, "source"), strVal("alpha"))
        def opened(cache: J): J = {
            if (equalText(op, "install")) {
                install(entries, cache, source, base)
            } else {
                if (equalText(op, "activate")) {
                    activate(entries, cache, base)
                } else {
                    if (equalText(op, "message")) {
                        if (equalText(get(step, "message"), "SKIP_WAITING")) {
                            def skipped(unused: J): J = {
                                boolean(true)
                            }
                            chain(skipWaiting(), skipped)
                        } else {
                            boolean(false)
                        }
                    } else {
                        val req = fetchRequestForStep(step, base)
                        val method = get(req, "method")
                        if ((!equalText(method, "GET"))) {
                            nil()
                        } else {
                            val path = get(req, "url")
                            def received(response: J): J = {
                                if (nullish(response)) {
                                    nil()
                                } else {
                                    def body(text: J): J = {
                                        pack(js.Array[J](get(response, "status"), text))
                                    }
                                    chain(call(response, "text", js.Array[J]()), body)
                                }
                            }
                            def failed(unused: J): J = {
                                pack(js.Array[J](strVal("no-response")))
                            }
                            recoverPromise(chain(strategy(entries, cache, name, path, source, base), received), failed)
                        }
                    }
                }
            }
        }
        chain(call(globalValue("caches"), "open", js.Array[J](name)), opened)
    }
}
@JSExportTopLevel("run")
def run(inputValue: J): J = {
    val base = absolute(strVal("experiments/library-matrix/libraries/workbox/"), get(globalValue("location"), "href"))
    if ((!missing(get(inputValue, "phase")))) {
        val task = phase(inputValue, get(inputValue, "name"), defaulted(get(inputValue, "manifest"), pack(js.Array[J]())), base)
        val event = get(inputValue, "event")
        if (truth(event)) {
            call(event, "waitUntil", js.Array[J](task))
        }
        task
    } else {
        val name = uniqueName()
        val state = objectValue()
        setProp(state, "manifest", pack(js.Array[J]()))
        val results = pack(js.Array[J]())
        def execute(step: J): J = {
            val items = defaulted(get(step, "manifest"), get(state, "manifest"))
            setProp(state, "manifest", items)
            def collectResult(result: J): J = {
                push(results, result)
                undef()
            }
            chain(phase(step, name, items, base), collectResult)
        }
        def complete(unused: J): J = {
            results
        }
        def cleanup(unused: J): J = {
            val caches = globalValue("caches")
            call(globalValue("Promise"), "all", js.Array[J](pack(js.Array[J](call(caches, "delete", js.Array[J](name)), call(caches, "delete", js.Array[J](suffix(name, "-runtime")))))))
        }
        always(chain(sequence(get(inputValue, "steps"), execute), complete), cleanup)
    }
}
}
