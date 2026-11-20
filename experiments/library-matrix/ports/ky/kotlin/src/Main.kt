import kotlin.js.Promise
import kotlin.math.pow

@JsName("globalThis") private external val globals:dynamic
@JsName("Reflect") private external object HostReflect {
    fun apply(f:dynamic,receiver:dynamic,args:Array<dynamic>):dynamic
    fun construct(f:dynamic,args:Array<dynamic>):dynamic
}
@JsName("Boolean") private external fun truth(x:dynamic):Boolean
private fun globalValue(k:String):dynamic=globals[k]
private fun get(x:dynamic,k:String):dynamic=x[k]
private fun setProp(x:dynamic,k:String,v:dynamic){x[k]=v}
private fun at(x:dynamic,i:Int):dynamic=x[i]
private fun missing(x:dynamic)=jsTypeOf(x)=="undefined"
private fun equalText(x:dynamic,s:String):Boolean=x===s
private fun equalNum(x:dynamic,i:Int):Boolean=x===i
private fun strVal(s:String):dynamic=s
private fun num(i:Int):dynamic=i
private fun boolean(v:Boolean):dynamic=v
private fun nil():dynamic=null
private fun undef():dynamic=undefined
private fun nullish(v:dynamic):Boolean=v==null
private fun objectValue():dynamic=kotlin.js.json()
private fun pack(values:Array<dynamic>):dynamic=values
private fun push(a:dynamic,v:dynamic){a.push(v)}
private fun call(x:dynamic,k:String,args:Array<dynamic>):dynamic=HostReflect.apply(x[k],x,args)
private fun makeHost(k:String,args:Array<dynamic>):dynamic=HostReflect.construct(globals[k],args)
private fun callback(f:(dynamic)->dynamic):dynamic=f
private fun promise(f:(dynamic,dynamic)->dynamic):dynamic=Promise<dynamic>{resolve,reject->f(resolve,reject)}
private fun settle(f:dynamic,v:dynamic){f(v)}
private fun chain(p:dynamic,f:(dynamic)->dynamic):dynamic=p.then(f)
private fun always(p:dynamic,f:(dynamic)->dynamic):dynamic=p.finally(f)
private fun sequence(xs:dynamic,f:(dynamic)->dynamic):dynamic{
 var p:dynamic=Promise.resolve(Unit)
 for(i in 0 until (xs.length as Int)){val item=xs[i];p=p.then{_:dynamic->f(item)}}
 return p
}
private fun header(h:dynamic,k:String):dynamic=h.get(k)
private fun ok(v:dynamic):dynamic=globals.Promise.resolve(v)
private fun bad(v:dynamic):dynamic=globals.Promise.reject(v)
private fun recoverPromise(p:dynamic,f:(dynamic)->dynamic):dynamic=p.catch(f)
private fun handled(p:dynamic,s:(dynamic)->dynamic,f:(dynamic)->dynamic):dynamic=p.then(s,f)
private fun timer(f:dynamic,ms:dynamic):dynamic=globals.setTimeout(f,ms)
private fun stopTimer(id:dynamic){globals.clearTimeout(id)}
private fun waitTime(ms:dynamic):dynamic=promise{res,_->timer(res,ms);undef()}
private fun isNumeric(x:dynamic)=jsTypeOf(x)=="number"
private fun lastOrIndex(xs:dynamic,i:dynamic):dynamic=xs[minOf(i as Int,(xs.length as Int)-1)]
private fun smaller(a:dynamic,b:dynamic):Boolean=(a as Double)<(b as Double)
private fun increment(a:dynamic):dynamic=(a as Int)+1
private fun lesser(a:dynamic,b:dynamic):dynamic=if(smaller(a,b)) a else b
private fun retryTag(a:dynamic):dynamic="retry-"+(a as Int)
private fun exponential(a:dynamic):dynamic=300.0*2.0.pow(a as Int)
private fun jsonType(a:dynamic):Boolean{
 if(a==null)return false
 val m=(a as String).substringBefore(';').trim().lowercase()
 return m.contains('/') && listOf("/json","+json","-json",".json").any{m.endsWith(it)}
}
private fun parseAfter(a:dynamic):dynamic{
 val raw=(a as String).trim()
 return if(raw.isNotEmpty() && raw.all{it in '0'..'9'}) raw.toDouble()*1000 else null
}
private fun canonicalMethod(method:String):String = when (method.lowercase()) {
 "get","post","put","patch","head","delete","query","options","trace" -> method.uppercase()
 else -> method
}
private fun allowsMethod(c:dynamic,m:dynamic):Boolean{
 val methods=(c.methods as Array<String>?)?:arrayOf("get","put","head","delete","options","trace","query")
 return methods.any{canonicalMethod(it)==canonicalMethod(m as String)}
}
private fun allowsStatus(c:dynamic,s:dynamic):Boolean{
 val xs=(c.statusCodes as Array<Int>?)?:arrayOf(408,413,429,500,502,503,504)
 return s!=null && xs.contains(s as Int)
}

fun makeError(name: dynamic, message: dynamic): dynamic {
    val value = makeHost("Error", arrayOf<dynamic>(message))
    setProp(value, "name", name)
    return value
}
fun buildRequest(c: dynamic): dynamic {
    val init = objectValue()
    val headers = makeHost("Headers", arrayOf<dynamic>(get(c, "headers")))
    setProp(init, "method", canonicalMethod((c.method as String?) ?: "GET"))
    setProp(init, "headers", headers)
    val payload = get(c, "json")
    if ((!missing(payload))) {
        setProp(init, "body", call(globalValue("JSON"), "stringify", arrayOf<dynamic>(payload)))
        if ((nullish(header(headers, "content-type")))) {
            call(headers, "set", arrayOf<dynamic>(strVal("content-type"), strVal("application/json")))
        }
    }
    if ((!truth(header(headers, "accept")))) {
        val mime = (if (equalText(get(c, "read"), "json")) strVal("application/json") else strVal("text/*"))
        call(headers, "set", arrayOf<dynamic>(strVal("accept"), mime))
    }
    return makeHost("Request", arrayOf<dynamic>(get(c, "url"), init))
}
fun fixtureFetch(c: dynamic, req: dynamic, user: dynamic, count: dynamic, trace: dynamic): dynamic {
    val h = get(req, "headers")
    fun respond(body: dynamic): dynamic {
        push(trace, pack(arrayOf<dynamic>(strVal("fetch"), get(req, "method"), get(req, "url"), header(h, "accept"), header(h, "content-type"), header(h, "x-matrix"), body)))
        if (equalText(get(c, "abort"), "fetch")) {
            call(user, "abort", arrayOf<dynamic>(makeError(strVal("AbortError"), strVal("cancelled"))))
        }
        val signal = get(req, "signal")
        if (truth(get(signal, "aborted"))) {
            return bad(get(signal, "reason"))
        } else {
            val reply = lastOrIndex(get(c, "replies"), count)
            if (equalText(get(reply, "kind"), "network")) {
                return bad(makeHost("TypeError", arrayOf<dynamic>(strVal("Failed to fetch"))))
            } else {
                if (equalText(get(reply, "kind"), "stall")) {
                    fun stall(resolve: dynamic, reject: dynamic): dynamic {
                        fun aborted(event: dynamic): dynamic {
                            settle(reject, get(signal, "reason"))
                            return undef()
                        }
                        val options = objectValue()
                        setProp(options, "once", boolean(true))
                        call(signal, "addEventListener", arrayOf<dynamic>(strVal("abort"), callback(::aborted), options))
                        return undef()
                    }
                    return promise(::stall)
                } else {
                    val options = objectValue()
                    setProp(options, "status", get(reply, "status"))
                    setProp(options, "headers", get(reply, "headers"))
                    return makeHost("Response", arrayOf<dynamic>(get(reply, "body"), options))
                }
            }
        }
    }
    return chain(call(req, "text", arrayOf<dynamic>()), ::respond)
}
fun dispatch(c: dynamic, base: dynamic, user: dynamic, count: dynamic, trace: dynamic): dynamic {
    val ctl = makeHost("AbortController", arrayOf<dynamic>())
    val signal = call(globalValue("AbortSignal"), "any", arrayOf<dynamic>(pack(arrayOf<dynamic>(get(user, "signal"), get(ctl, "signal")))))
    val init = objectValue()
    setProp(init, "signal", signal)
    val req = makeHost("Request", arrayOf<dynamic>(call(base, "clone", arrayOf<dynamic>()), init))
    val fetching = fixtureFetch(c, req, user, count, trace)
    val timeout = get(c, "timeout")
    if (isNumeric(timeout)) {
        val holder = objectValue()
        fun executor(resolve: dynamic, reject: dynamic): dynamic {
            fun expiry(unused: dynamic): dynamic {
                settle(reject, makeError(strVal("TimeoutError"), strVal("Request timed out")))
                call(ctl, "abort", arrayOf<dynamic>())
                return undef()
            }
            setProp(holder, "timer", timer(callback(::expiry), timeout))
            return undef()
        }
        val expiryTask = promise(::executor)
        fun clear(unused: dynamic): dynamic {
            stopTimer(get(holder, "timer"))
            return undef()
        }
        return always(call(globalValue("Promise"), "race", arrayOf<dynamic>(pack(arrayOf<dynamic>(fetching, expiryTask)))), ::clear)
    } else {
        return fetching
    }
}
fun parseJson(body: dynamic): dynamic {
    return call(globalValue("JSON"), "parse", arrayOf<dynamic>(body))
}
fun errorData(response: dynamic): dynamic {
    val h = get(response, "headers")
    fun read(body: dynamic): dynamic {
        if ((!truth(body))) {
            return nil()
        } else {
            if (jsonType(header(h, "content-type"))) {
                fun invalid(unused: dynamic): dynamic {
                    return nil()
                }
                return recoverPromise(chain(ok(body), ::parseJson), ::invalid)
            } else {
                return body
            }
        }
    }
    return chain(call(response, "text", arrayOf<dynamic>()), ::read)
}
fun retryDelay(c: dynamic, method: dynamic, count: dynamic, fault: dynamic, trace: dynamic): dynamic {
    val name = at(fault, 0)
    val status = at(fault, 1)
    val raw = at(fault, 3)
    val eligible = (allowsMethod(c, method) && smaller(count, get(c, "limit")))
    val serverStatus = (equalNum(status, 413) || equalNum(status, 429) || equalNum(status, 503))
    val http = equalText(name, "HTTPError")
    val retriable = (if (http) allowsStatus(c, status) else (equalText(name, "NetworkError") || (equalText(name, "TimeoutError") && truth(get(c, "retryOnTimeout")))))
    if (((!eligible) || (!retriable) || (http && equalNum(status, 413) && (!truth(raw))))) {
        return nil()
    } else {
        val after = (if ((http && serverStatus && truth(raw))) parseAfter(raw) else nil())
        if ((!nullish(after))) {
            return lesser(after, get(c, "maxRetryAfter"))
        } else {
            val ms = exponential(count)
            push(trace, pack(arrayOf<dynamic>(strVal("backoff"), increment(count), ms)))
            return lesser(ms, get(c, "backoffLimit"))
        }
    }
}
fun outputError(trace: dynamic, count: dynamic, f: dynamic, hook: dynamic): dynamic {
    if (truth(hook)) {
        push(trace, pack(arrayOf<dynamic>(strVal("beforeError"), count, at(f, 0))))
    }
    return pack(arrayOf<dynamic>(trace, pack(arrayOf<dynamic>(strVal("error"), at(f, 0), at(f, 1), at(f, 2)))))
}
fun attempt(c: dynamic, base: dynamic, user: dynamic, count: dynamic, trace: dynamic): dynamic {
    fun onFailure(f: dynamic): dynamic {
        val delay = retryDelay(c, get(base, "method"), count, f, trace)
        if (nullish(delay)) {
            return outputError(trace, count, f, boolean(true))
        } else {
            fun proceed(unused: dynamic): dynamic {
                val following = increment(count)
                push(trace, pack(arrayOf<dynamic>(strVal("beforeRetry"), following, at(f, 0))))
                call(get(base, "headers"), "set", arrayOf<dynamic>(strVal("x-matrix"), retryTag(following)))
                if (equalText(get(c, "abort"), "retry")) {
                    call(user, "abort", arrayOf<dynamic>(makeError(strVal("AbortError"), strVal("cancelled"))))
                }
                return attempt(c, base, user, following, trace)
            }
            return chain(waitTime(delay), ::proceed)
        }
    }
    fun success(response: dynamic): dynamic {
        val status = get(response, "status")
        push(trace, pack(arrayOf<dynamic>(strVal("afterResponse"), count, status)))
        if (((!truth(get(response, "ok"))) && truth(get(c, "throwHttpErrors")))) {
            fun failed(data: dynamic): dynamic {
                return onFailure(pack(arrayOf<dynamic>(strVal("HTTPError"), status, data, header(get(response, "headers"), "retry-after"))))
            }
            return chain(errorData(response), ::failed)
        } else {
            fun result(payload: dynamic): dynamic {
                return pack(arrayOf<dynamic>(trace, pack(arrayOf<dynamic>(strVal("ok"), payload))))
            }
            val body = call(response, "text", arrayOf<dynamic>())
            if (equalText(get(c, "read"), "json")) {
                fun invalid(unused: dynamic): dynamic {
                    return outputError(trace, count, pack(arrayOf<dynamic>(strVal("SyntaxError"), nil(), nil(), nil())), boolean(false))
                }
                return recoverPromise(chain(chain(body, ::parseJson), ::result), ::invalid)
            } else {
                return chain(body, ::result)
            }
        }
    }
    fun failure(error: dynamic): dynamic {
        val name = get(error, "name")
        val wrapped = (if ((equalText(name, "TypeError") && equalText(get(error, "message"), "Failed to fetch"))) strVal("NetworkError") else name)
        return onFailure(pack(arrayOf<dynamic>(wrapped, nil(), nil(), nil())))
    }
    return handled(dispatch(c, base, user, count, trace), ::success, ::failure)
}
fun requestCase(c: dynamic): dynamic {
    val trace = pack(arrayOf<dynamic>(pack(arrayOf<dynamic>(strVal("init")))))
    val user = makeHost("AbortController", arrayOf<dynamic>())
    if (equalText(get(c, "abort"), "before")) {
        call(user, "abort", arrayOf<dynamic>(makeError(strVal("AbortError"), strVal("cancelled"))))
    }
    fun ready(base: dynamic): dynamic {
        if (truth(get(get(user, "signal"), "aborted"))) {
            return ok(outputError(trace, num(0), pack(arrayOf<dynamic>(strVal("AbortError"), nil(), nil(), nil())), boolean(true)))
        } else {
            push(trace, pack(arrayOf<dynamic>(strVal("beforeRequest"), num(0))))
            call(get(base, "headers"), "set", arrayOf<dynamic>(strVal("x-matrix"), strVal("initial")))
            return attempt(c, base, user, num(0), trace)
        }
    }
    fun invalid(error: dynamic): dynamic = outputError(trace, num(0), pack(arrayOf<dynamic>(get(error, "name"), nil(), nil(), nil())), boolean(false))
    return handled(chain(ok(c), ::buildRequest), ::ready, ::invalid)
}
@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(inputValue: dynamic): dynamic {
    val results = pack(arrayOf<dynamic>())
    fun evaluate(c: dynamic): dynamic {
        fun save(result: dynamic): dynamic {
            push(results, result)
            return undef()
        }
        return chain(requestCase(c), ::save)
    }
    fun complete(unused: dynamic): dynamic {
        return results
    }
    return chain(sequence(get(inputValue, "cases"), ::evaluate), ::complete)
}
