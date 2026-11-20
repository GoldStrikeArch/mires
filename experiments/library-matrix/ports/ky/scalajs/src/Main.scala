package ky
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
private def num(i:Int):J=i.asInstanceOf[J]
private def nil():J=null.asInstanceOf[J]
private def undef():J=js.undefined.asInstanceOf[J]
private def objectValue():J=js.Dynamic.literal()
private def pack(values:js.Array[J]):J=values.asInstanceOf[J]
private def push(a:J,v:J):Unit={a.push(v);()}
private def call(x:J,k:String,args:js.Array[J]):J=js.Dynamic.global.Reflect.applyDynamic("apply")(get(x,k),x,args)
private def callback(f:J=>J):J=js.Any.fromFunction1(f).asInstanceOf[J]
private def promise(f:(J,J)=>J):J=js.Dynamic.newInstance(js.Dynamic.global.Promise)(js.Any.fromFunction2(f))
private def settle(f:J,v:J):Unit={f(v);()}
private def chain(p:J,f:J=>J):J=p.applyDynamic("then")(callback(f))
private def always(p:J,f:J=>J):J=p.applyDynamic("finally")(callback(f))
private def mapValues(xs:J,f:J=>J):J=xs.map(callback(f))
private def sequence(xs:J,f:J=>J):J=xs.asInstanceOf[js.Array[J]].foldLeft(js.Dynamic.global.Promise.resolve())((p,v)=>chain(p,_=>f(v)))
private def makeHost(k:String,args:js.Array[J]):J=js.Dynamic.global.Reflect.construct(globalValue(k),args)
private def nullish(a:J)=a==null || missing(a)
private def boolean(b:Boolean):J=b.asInstanceOf[J]
private def header(h:J,k:String):J=h.get(k)
private def ok(v:J):J=js.Dynamic.global.Promise.resolve(v)
private def bad(v:J):J=js.Dynamic.global.Promise.reject(v)
private def recoverPromise(p:J,f:J=>J):J=p.applyDynamic("catch")(callback(f))
private def handled(p:J,s:J=>J,f:J=>J):J=p.applyDynamic("then")(callback(s),callback(f))
private def timer(f:J,ms:J):J=js.Dynamic.global.setTimeout(f,ms)
private def stopTimer(id:J):Unit={js.Dynamic.global.clearTimeout(id);()}
private def waitTime(ms:J):J=promise((resolve,_)=>{timer(resolve,ms);undef()})
private def isNumeric(v:J):Boolean=js.typeOf(v)=="number"
private def lastOrIndex(xs:J,i:J):J=at(xs,math.min(i.asInstanceOf[Int],xs.length.asInstanceOf[Int]-1))
private def smaller(a:J,b:J)=a.asInstanceOf[Double]<b.asInstanceOf[Double]
private def increment(a:J)=num(a.asInstanceOf[Int]+1)
private def lesser(a:J,b:J)=if(smaller(a,b)) a else b
private def retryTag(a:J)=strVal("retry-"+a.asInstanceOf[Int])
private def exponential(a:J):J=(300*math.pow(2,a.asInstanceOf[Double])).asInstanceOf[J]
private def jsonType(a:J):Boolean=if(nullish(a)) false else {
 val m=a.asInstanceOf[String].takeWhile(_!=';').trim.toLowerCase
 m.contains("/") && List("/json","+json","-json",".json").exists(m.endsWith)
}
private def parseAfter(a:J):J={
 val s=a.asInstanceOf[String].trim
 if(s.nonEmpty && s.forall(c=>c>='0' && c<='9')) (s.toDouble*1000).asInstanceOf[J] else nil()
}
private def canonicalMethod(method:String):String=method.toLowerCase match {
 case "get" | "post" | "put" | "patch" | "head" | "delete" | "query" | "options" | "trace" => method.toUpperCase
 case _ => method
}
private def allowsMethod(c:J,m:J):Boolean={
 val k=canonicalMethod(m.asInstanceOf[String])
 if(missing(c.methods)) Set("GET","PUT","HEAD","DELETE","OPTIONS","TRACE","QUERY").contains(k)
 else c.methods.asInstanceOf[js.Array[String]].exists(x=>canonicalMethod(x)==k)
}
private def allowsStatus(c:J,s:J):Boolean=if(missing(c.statusCodes)) Set(408,413,429,500,502,503,504).contains(s.asInstanceOf[Int]) else c.statusCodes.includes(s).asInstanceOf[Boolean]

def makeError(name: J, message: J): J = {
    val value = makeHost("Error", js.Array[J](message))
    setProp(value, "name", name)
    value
}
def buildRequest(c: J): J = {
    val init = objectValue()
    val headers = makeHost("Headers", js.Array[J](get(c, "headers")))
    setProp(init, "method", strVal(if (nullish(get(c, "method"))) "GET" else canonicalMethod(c.method.asInstanceOf[String])))
    setProp(init, "headers", headers)
    val payload = get(c, "json")
    if ((!missing(payload))) {
        setProp(init, "body", call(globalValue("JSON"), "stringify", js.Array[J](payload)))
        if ((nullish(header(headers, "content-type")))) {
            call(headers, "set", js.Array[J](strVal("content-type"), strVal("application/json")))
        }
    }
    if ((!truth(header(headers, "accept")))) {
        val mime = (if (equalText(get(c, "read"), "json")) strVal("application/json") else strVal("text/*"))
        call(headers, "set", js.Array[J](strVal("accept"), mime))
    }
    makeHost("Request", js.Array[J](get(c, "url"), init))
}
def fixtureFetch(c: J, req: J, user: J, count: J, trace: J): J = {
    val h = get(req, "headers")
    def respond(body: J): J = {
        push(trace, pack(js.Array[J](strVal("fetch"), get(req, "method"), get(req, "url"), header(h, "accept"), header(h, "content-type"), header(h, "x-matrix"), body)))
        if (equalText(get(c, "abort"), "fetch")) {
            call(user, "abort", js.Array[J](makeError(strVal("AbortError"), strVal("cancelled"))))
        }
        val signal = get(req, "signal")
        if (truth(get(signal, "aborted"))) {
            bad(get(signal, "reason"))
        } else {
            val reply = lastOrIndex(get(c, "replies"), count)
            if (equalText(get(reply, "kind"), "network")) {
                bad(makeHost("TypeError", js.Array[J](strVal("Failed to fetch"))))
            } else {
                if (equalText(get(reply, "kind"), "stall")) {
                    def stall(resolve: J, reject: J): J = {
                        def aborted(event: J): J = {
                            settle(reject, get(signal, "reason"))
                            undef()
                        }
                        val options = objectValue()
                        setProp(options, "once", boolean(true))
                        call(signal, "addEventListener", js.Array[J](strVal("abort"), callback(aborted), options))
                        undef()
                    }
                    promise(stall)
                } else {
                    val options = objectValue()
                    setProp(options, "status", get(reply, "status"))
                    setProp(options, "headers", get(reply, "headers"))
                    makeHost("Response", js.Array[J](get(reply, "body"), options))
                }
            }
        }
    }
    chain(call(req, "text", js.Array[J]()), respond)
}
def dispatch(c: J, base: J, user: J, count: J, trace: J): J = {
    val ctl = makeHost("AbortController", js.Array[J]())
    val signal = call(globalValue("AbortSignal"), "any", js.Array[J](pack(js.Array[J](get(user, "signal"), get(ctl, "signal")))))
    val init = objectValue()
    setProp(init, "signal", signal)
    val req = makeHost("Request", js.Array[J](call(base, "clone", js.Array[J]()), init))
    val fetching = fixtureFetch(c, req, user, count, trace)
    val timeout = get(c, "timeout")
    if (isNumeric(timeout)) {
        val holder = objectValue()
        def executor(resolve: J, reject: J): J = {
            def expiry(unused: J): J = {
                settle(reject, makeError(strVal("TimeoutError"), strVal("Request timed out")))
                call(ctl, "abort", js.Array[J]())
                undef()
            }
            setProp(holder, "timer", timer(callback(expiry), timeout))
            undef()
        }
        val expiryTask = promise(executor)
        def clear(unused: J): J = {
            stopTimer(get(holder, "timer"))
            undef()
        }
        always(call(globalValue("Promise"), "race", js.Array[J](pack(js.Array[J](fetching, expiryTask)))), clear)
    } else {
        fetching
    }
}
def parseJson(body: J): J = {
    call(globalValue("JSON"), "parse", js.Array[J](body))
}
def errorData(response: J): J = {
    val h = get(response, "headers")
    def read(body: J): J = {
        if ((!truth(body))) {
            nil()
        } else {
            if (jsonType(header(h, "content-type"))) {
                def invalid(unused: J): J = {
                    nil()
                }
                recoverPromise(chain(ok(body), parseJson), invalid)
            } else {
                body
            }
        }
    }
    chain(call(response, "text", js.Array[J]()), read)
}
def retryDelay(c: J, method: J, count: J, fault: J, trace: J): J = {
    val name = at(fault, 0)
    val status = at(fault, 1)
    val raw = at(fault, 3)
    val eligible = (allowsMethod(c, method) && smaller(count, get(c, "limit")))
    val serverStatus = (equalNum(status, 413) || equalNum(status, 429) || equalNum(status, 503))
    val http = equalText(name, "HTTPError")
    val retriable = (if (http) allowsStatus(c, status) else (equalText(name, "NetworkError") || (equalText(name, "TimeoutError") && truth(get(c, "retryOnTimeout")))))
    if (((!eligible) || (!retriable) || (http && equalNum(status, 413) && (!truth(raw))))) {
        nil()
    } else {
        val after = (if ((http && serverStatus && truth(raw))) parseAfter(raw) else nil())
        if ((!nullish(after))) {
            lesser(after, get(c, "maxRetryAfter"))
        } else {
            val ms = exponential(count)
            push(trace, pack(js.Array[J](strVal("backoff"), increment(count), ms)))
            lesser(ms, get(c, "backoffLimit"))
        }
    }
}
def outputError(trace: J, count: J, f: J, hook: J): J = {
    if (truth(hook)) {
        push(trace, pack(js.Array[J](strVal("beforeError"), count, at(f, 0))))
    }
    pack(js.Array[J](trace, pack(js.Array[J](strVal("error"), at(f, 0), at(f, 1), at(f, 2)))))
}
def attempt(c: J, base: J, user: J, count: J, trace: J): J = {
    def onFailure(f: J): J = {
        val delay = retryDelay(c, get(base, "method"), count, f, trace)
        if (nullish(delay)) {
            outputError(trace, count, f, boolean(true))
        } else {
            def proceed(unused: J): J = {
                val following = increment(count)
                push(trace, pack(js.Array[J](strVal("beforeRetry"), following, at(f, 0))))
                call(get(base, "headers"), "set", js.Array[J](strVal("x-matrix"), retryTag(following)))
                if (equalText(get(c, "abort"), "retry")) {
                    call(user, "abort", js.Array[J](makeError(strVal("AbortError"), strVal("cancelled"))))
                }
                attempt(c, base, user, following, trace)
            }
            chain(waitTime(delay), proceed)
        }
    }
    def success(response: J): J = {
        val status = get(response, "status")
        push(trace, pack(js.Array[J](strVal("afterResponse"), count, status)))
        if (((!truth(get(response, "ok"))) && truth(get(c, "throwHttpErrors")))) {
            def failed(data: J): J = {
                onFailure(pack(js.Array[J](strVal("HTTPError"), status, data, header(get(response, "headers"), "retry-after"))))
            }
            chain(errorData(response), failed)
        } else {
            def result(payload: J): J = {
                pack(js.Array[J](trace, pack(js.Array[J](strVal("ok"), payload))))
            }
            val body = call(response, "text", js.Array[J]())
            if (equalText(get(c, "read"), "json")) {
                def invalid(unused: J): J = {
                    outputError(trace, count, pack(js.Array[J](strVal("SyntaxError"), nil(), nil(), nil())), boolean(false))
                }
                recoverPromise(chain(chain(body, parseJson), result), invalid)
            } else {
                chain(body, result)
            }
        }
    }
    def failure(error: J): J = {
        val name = get(error, "name")
        val wrapped = (if ((equalText(name, "TypeError") && equalText(get(error, "message"), "Failed to fetch"))) strVal("NetworkError") else name)
        onFailure(pack(js.Array[J](wrapped, nil(), nil(), nil())))
    }
    handled(dispatch(c, base, user, count, trace), success, failure)
}
def requestCase(c: J): J = {
    val trace = pack(js.Array[J](pack(js.Array[J](strVal("init")))))
    val user = makeHost("AbortController", js.Array[J]())
    if (equalText(get(c, "abort"), "before")) {
        call(user, "abort", js.Array[J](makeError(strVal("AbortError"), strVal("cancelled"))))
    }
    def ready(base: J): J = {
        if (truth(get(get(user, "signal"), "aborted"))) {
            ok(outputError(trace, num(0), pack(js.Array[J](strVal("AbortError"), nil(), nil(), nil())), boolean(true)))
        } else {
            push(trace, pack(js.Array[J](strVal("beforeRequest"), num(0))))
            call(get(base, "headers"), "set", js.Array[J](strVal("x-matrix"), strVal("initial")))
            attempt(c, base, user, num(0), trace)
        }
    }
    def invalid(error: J): J = outputError(trace, num(0), pack(js.Array[J](get(error, "name"), nil(), nil(), nil())), boolean(false))
    handled(chain(ok(c), buildRequest), ready, invalid)
}
@JSExportTopLevel("run")
def run(inputValue: J): J = {
    val results = pack(js.Array[J]())
    def evaluate(c: J): J = {
        def save(result: J): J = {
            push(results, result)
            undef()
        }
        chain(requestCase(c), save)
    }
    def complete(unused: J): J = {
        results
    }
    chain(sequence(get(inputValue, "cases"), evaluate), complete)
}
}
