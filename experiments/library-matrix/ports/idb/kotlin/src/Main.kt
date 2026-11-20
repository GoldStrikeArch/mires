import kotlin.js.Promise

@JsName("globalThis") private external val globals:dynamic
@JsName("Boolean") private external fun truth(x:dynamic):Boolean
@JsName("DOMException") private external class HostError(message:String,name:String)
private fun globalValue(k:String):dynamic = globals[k]
private fun get(x:dynamic,k:String):dynamic=x[k]
private fun setProp(x:dynamic,k:String,v:dynamic){x[k]=v}
private fun at(x:dynamic,i:Int):dynamic=x[i]
private fun missing(x:dynamic)=jsTypeOf(x)=="undefined"
private fun equalText(x:dynamic,s:String):Boolean=x===s
private fun equalNum(x:dynamic,i:Int):Boolean=x===i
private fun strVal(s:String):dynamic=s
private fun num(i:Int):dynamic=i
private fun nil():dynamic=null
private fun undef():dynamic=undefined
private fun objectValue():dynamic=kotlin.js.json()
private fun pack(values:Array<dynamic>):dynamic=values
private fun push(a:dynamic,v:dynamic){a.push(v)}
private fun call(x:dynamic,k:String,args:Array<dynamic>):dynamic=x[k].apply(x,args)
private fun callback(f:(dynamic)->dynamic):dynamic=f
private fun promise(f:(dynamic,dynamic)->dynamic):dynamic=Promise<dynamic>{resolve,reject->f(resolve,reject)}
private fun settle(f:dynamic,v:dynamic){f(v)}
private fun chain(p:dynamic,f:(dynamic)->dynamic):dynamic=p.then(f)
private fun always(p:dynamic,f:(dynamic)->dynamic):dynamic=p.finally(f)
private fun mapValues(xs:dynamic,f:(dynamic)->dynamic):dynamic=xs.map(f)
private fun sequence(xs:dynamic,f:(dynamic)->dynamic):dynamic{
    var p:dynamic=Promise.resolve(Unit)
    for(i in 0 until (xs.length as Int)){val item=xs[i];p=p.then{_:dynamic->f(item)}}
    return p
}
private fun abortError():dynamic=HostError("AbortError","AbortError")
private fun errorName(name:dynamic):dynamic="!"+(name as String)
private fun uniqueName():dynamic="mires-matrix-idb-"+(globalValue("crypto").randomUUID() as String)

fun request(req: dynamic): dynamic {
    fun executor(resolve: dynamic, reject: dynamic): dynamic {
        fun success(event: dynamic): dynamic {
            setProp(req, "onsuccess", nil())
            setProp(req, "onerror", nil())
            settle(resolve, get(req, "result"))
            return undef()
        }
        fun failure(event: dynamic): dynamic {
            setProp(req, "onsuccess", nil())
            setProp(req, "onerror", nil())
            settle(reject, get(req, "error"))
            return undef()
        }
        setProp(req, "onsuccess", callback(::success))
        setProp(req, "onerror", callback(::failure))
        return undef()
    }
    return promise(::executor)
}
fun clearTransaction(tx: dynamic): dynamic {
    setProp(tx, "oncomplete", nil())
    setProp(tx, "onerror", nil())
    setProp(tx, "onabort", nil())
    return undef()
}
fun transactionDone(tx: dynamic): dynamic {
    fun executor(resolve: dynamic, reject: dynamic): dynamic {
        fun complete(event: dynamic): dynamic {
            clearTransaction(tx)
            settle(resolve, undef())
            return undef()
        }
        fun failure(event: dynamic): dynamic {
            val reason = get(tx, "error")
            clearTransaction(tx)
            settle(reject, (if (truth(reason)) reason else abortError()))
            return undef()
        }
        setProp(tx, "oncomplete", callback(::complete))
        setProp(tx, "onerror", callback(::failure))
        setProp(tx, "onabort", callback(::failure))
        return undef()
    }
    return promise(::executor)
}
fun openDatabase(name: dynamic): dynamic {
    val req = call(globalValue("indexedDB"), "open", arrayOf<dynamic>(name, num(1)))
    fun upgrade(event: dynamic): dynamic {
        val options = objectValue()
        setProp(options, "keyPath", strVal("id"))
        val store = call(get(req, "result"), "createObjectStore", arrayOf<dynamic>(strVal("rows"), options))
        call(store, "createIndex", arrayOf<dynamic>(strVal("byGroup"), strVal("group")))
        return undef()
    }
    setProp(req, "onupgradeneeded", callback(::upgrade))
    return request(req)
}
fun writeBatch(db: dynamic, batch: dynamic): dynamic {
    val tx = call(db, "transaction", arrayOf<dynamic>(strVal("rows"), strVal("readwrite")))
    val completion = transactionDone(tx)
    val store = call(tx, "objectStore", arrayOf<dynamic>(strVal("rows")))
    fun submit(row: dynamic): dynamic {
        return (if (truth(get(batch, "add"))) request(call(store, "add", arrayOf<dynamic>(row))) else request(call(store, "put", arrayOf<dynamic>(row))))
    }
    val promises = mapValues(get(batch, "rows"), ::submit)
    push(promises, completion)
    val settled = call(globalValue("Promise"), "allSettled", arrayOf<dynamic>(promises))
    if (truth(get(batch, "abort"))) {
        call(tx, "abort", arrayOf<dynamic>())
    }
    fun collect(items: dynamic): dynamic {
        fun outcome(item: dynamic): dynamic {
            val value = get(item, "value")
            return (if (equalText(get(item, "status"), "rejected")) errorName(get(get(item, "reason"), "name")) else (if (missing(value)) strVal("done") else value))
        }
        return mapValues(items, ::outcome)
    }
    return chain(settled, ::collect)
}
fun lookup(source: dynamic, keys: dynamic): dynamic {
    fun submit(key: dynamic): dynamic {
        return request(call(source, "get", arrayOf<dynamic>(key)))
    }
    return call(globalValue("Promise"), "all", arrayOf<dynamic>(mapValues(keys, ::submit)))
}
fun storedValues(rows: dynamic): dynamic {
    fun value(row: dynamic): dynamic {
        return (if (missing(row)) nil() else get(row, "value"))
    }
    return mapValues(rows, ::value)
}
fun readBack(db: dynamic, inputValue: dynamic, transactions: dynamic): dynamic {
    val tx = call(db, "transaction", arrayOf<dynamic>(strVal("rows")))
    val completion = transactionDone(tx)
    val store = call(tx, "objectStore", arrayOf<dynamic>(strVal("rows")))
    val index = call(store, "index", arrayOf<dynamic>(strVal("byGroup")))
    val reads = lookup(store, get(inputValue, "keys"))
    val indexReads = lookup(index, get(inputValue, "groups"))
    val source = (if (truth(get(inputValue, "index"))) index else store)
    val only = get(inputValue, "only")
    val bounds = (if (missing(only)) nil() else call(globalValue("IDBKeyRange"), "only", arrayOf<dynamic>(only)))
    val direction = (if (truth(get(inputValue, "reverse"))) strVal("prev") else strVal("next"))
    val req = call(source, "openCursor", arrayOf<dynamic>(bounds, direction))
    val step = get(inputValue, "step")
    val cursors = pack(arrayOf<dynamic>())
    fun visit(cursor: dynamic): dynamic {
        if (truth(cursor)) {
            push(cursors, pack(arrayOf<dynamic>(get(cursor, "key"), get(cursor, "primaryKey"), get(get(cursor, "value"), "value"))))
            if (equalNum(step, 1)) {
                call(cursor, "continue", arrayOf<dynamic>())
            } else {
                call(cursor, "advance", arrayOf<dynamic>(step))
            }
            return chain(request(req), ::visit)
        } else {
            return cursors
        }
    }
    val cursorDone = chain(request(req), ::visit)
    val awaited = call(globalValue("Promise"), "all", arrayOf<dynamic>(pack(arrayOf<dynamic>(reads, indexReads, cursorDone, completion))))
    fun output(result: dynamic): dynamic {
        return pack(arrayOf<dynamic>(transactions, storedValues(at(result, 0)), storedValues(at(result, 1)), at(result, 2)))
    }
    return chain(awaited, ::output)
}
fun exercise(db: dynamic, inputValue: dynamic): dynamic {
    val transactions = pack(arrayOf<dynamic>())
    fun write(batch: dynamic): dynamic {
        fun saved(outcomes: dynamic): dynamic {
            push(transactions, outcomes)
            return undef()
        }
        return chain(writeBatch(db, batch), ::saved)
    }
    fun nextPhase(unused: dynamic): dynamic {
        return readBack(db, inputValue, transactions)
    }
    return chain(sequence(get(inputValue, "batches"), ::write), ::nextPhase)
}
@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(inputValue: dynamic): dynamic {
    val name = uniqueName()
    fun opened(db: dynamic): dynamic {
        fun close(unused: dynamic): dynamic {
            return call(db, "close", arrayOf<dynamic>())
        }
        return always(exercise(db, inputValue), ::close)
    }
    fun cleanup(unused: dynamic): dynamic {
        return request(call(globalValue("indexedDB"), "deleteDatabase", arrayOf<dynamic>(name)))
    }
    return always(chain(openDatabase(name), ::opened), ::cleanup)
}
