package idb
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
private def abortError():J=js.Dynamic.newInstance(js.Dynamic.global.DOMException)("AbortError","AbortError")
private def errorName(x:J):J=strVal("!"+x.asInstanceOf[String])
private def uniqueName():J=strVal("mires-matrix-idb-"+js.Dynamic.global.crypto.randomUUID().asInstanceOf[String])

def request(req: J): J = {
    def executor(resolve: J, reject: J): J = {
        def success(event: J): J = {
            setProp(req, "onsuccess", nil())
            setProp(req, "onerror", nil())
            settle(resolve, get(req, "result"))
            undef()
        }
        def failure(event: J): J = {
            setProp(req, "onsuccess", nil())
            setProp(req, "onerror", nil())
            settle(reject, get(req, "error"))
            undef()
        }
        setProp(req, "onsuccess", callback(success))
        setProp(req, "onerror", callback(failure))
        undef()
    }
    promise(executor)
}
def clearTransaction(tx: J): J = {
    setProp(tx, "oncomplete", nil())
    setProp(tx, "onerror", nil())
    setProp(tx, "onabort", nil())
    undef()
}
def transactionDone(tx: J): J = {
    def executor(resolve: J, reject: J): J = {
        def complete(event: J): J = {
            clearTransaction(tx)
            settle(resolve, undef())
            undef()
        }
        def failure(event: J): J = {
            val reason = get(tx, "error")
            clearTransaction(tx)
            settle(reject, (if (truth(reason)) reason else abortError()))
            undef()
        }
        setProp(tx, "oncomplete", callback(complete))
        setProp(tx, "onerror", callback(failure))
        setProp(tx, "onabort", callback(failure))
        undef()
    }
    promise(executor)
}
def openDatabase(name: J): J = {
    val req = call(globalValue("indexedDB"), "open", js.Array[J](name, num(1)))
    def upgrade(event: J): J = {
        val options = objectValue()
        setProp(options, "keyPath", strVal("id"))
        val store = call(get(req, "result"), "createObjectStore", js.Array[J](strVal("rows"), options))
        call(store, "createIndex", js.Array[J](strVal("byGroup"), strVal("group")))
        undef()
    }
    setProp(req, "onupgradeneeded", callback(upgrade))
    request(req)
}
def writeBatch(db: J, batch: J): J = {
    val tx = call(db, "transaction", js.Array[J](strVal("rows"), strVal("readwrite")))
    val completion = transactionDone(tx)
    val store = call(tx, "objectStore", js.Array[J](strVal("rows")))
    def submit(row: J): J = {
        (if (truth(get(batch, "add"))) request(call(store, "add", js.Array[J](row))) else request(call(store, "put", js.Array[J](row))))
    }
    val promises = mapValues(get(batch, "rows"), submit)
    push(promises, completion)
    val settled = call(globalValue("Promise"), "allSettled", js.Array[J](promises))
    if (truth(get(batch, "abort"))) {
        call(tx, "abort", js.Array[J]())
    }
    def collect(items: J): J = {
        def outcome(item: J): J = {
            val value = get(item, "value")
            (if (equalText(get(item, "status"), "rejected")) errorName(get(get(item, "reason"), "name")) else (if (missing(value)) strVal("done") else value))
        }
        mapValues(items, outcome)
    }
    chain(settled, collect)
}
def lookup(source: J, keys: J): J = {
    def submit(key: J): J = {
        request(call(source, "get", js.Array[J](key)))
    }
    call(globalValue("Promise"), "all", js.Array[J](mapValues(keys, submit)))
}
def storedValues(rows: J): J = {
    def value(row: J): J = {
        (if (missing(row)) nil() else get(row, "value"))
    }
    mapValues(rows, value)
}
def readBack(db: J, inputValue: J, transactions: J): J = {
    val tx = call(db, "transaction", js.Array[J](strVal("rows")))
    val completion = transactionDone(tx)
    val store = call(tx, "objectStore", js.Array[J](strVal("rows")))
    val index = call(store, "index", js.Array[J](strVal("byGroup")))
    val reads = lookup(store, get(inputValue, "keys"))
    val indexReads = lookup(index, get(inputValue, "groups"))
    val source = (if (truth(get(inputValue, "index"))) index else store)
    val only = get(inputValue, "only")
    val bounds = (if (missing(only)) nil() else call(globalValue("IDBKeyRange"), "only", js.Array[J](only)))
    val direction = (if (truth(get(inputValue, "reverse"))) strVal("prev") else strVal("next"))
    val req = call(source, "openCursor", js.Array[J](bounds, direction))
    val step = get(inputValue, "step")
    val cursors = pack(js.Array[J]())
    def visit(cursor: J): J = {
        if (truth(cursor)) {
            push(cursors, pack(js.Array[J](get(cursor, "key"), get(cursor, "primaryKey"), get(get(cursor, "value"), "value"))))
            if (equalNum(step, 1)) {
                call(cursor, "continue", js.Array[J]())
            } else {
                call(cursor, "advance", js.Array[J](step))
            }
            chain(request(req), visit)
        } else {
            cursors
        }
    }
    val cursorDone = chain(request(req), visit)
    val awaited = call(globalValue("Promise"), "all", js.Array[J](pack(js.Array[J](reads, indexReads, cursorDone, completion))))
    def output(result: J): J = {
        pack(js.Array[J](transactions, storedValues(at(result, 0)), storedValues(at(result, 1)), at(result, 2)))
    }
    chain(awaited, output)
}
def exercise(db: J, inputValue: J): J = {
    val transactions = pack(js.Array[J]())
    def write(batch: J): J = {
        def saved(outcomes: J): J = {
            push(transactions, outcomes)
            undef()
        }
        chain(writeBatch(db, batch), saved)
    }
    def nextPhase(unused: J): J = {
        readBack(db, inputValue, transactions)
    }
    chain(sequence(get(inputValue, "batches"), write), nextPhase)
}
@JSExportTopLevel("run")
def run(inputValue: J): J = {
    val name = uniqueName()
    def opened(db: J): J = {
        def close(unused: J): J = {
            call(db, "close", js.Array[J]())
        }
        always(exercise(db, inputValue), close)
    }
    def cleanup(unused: J): J = {
        request(call(globalValue("indexedDB"), "deleteDatabase", js.Array[J](name)))
    }
    always(chain(openDatabase(name), opened), cleanup)
}
}
