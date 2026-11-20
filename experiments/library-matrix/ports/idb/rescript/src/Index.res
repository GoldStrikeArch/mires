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
@new external domException:(string,string)=>j="DOMException"
let abortError=()=>domException("AbortError","AbortError")
let errorName=x=>strVal("!"++(cast(x):string))
let uniqueName=()=>strVal("mires-matrix-idb-"++(cast(call(globalValue("crypto"),"randomUUID",[])):string))

let request = (req: j): j => {
    let executor = (resolve: j, reject: j): j => {
        let success = (event: j): j => {
            let _ = setProp(req, "onsuccess", nil())
            let _ = setProp(req, "onerror", nil())
            let _ = settle(resolve, get(req, "result"))
            undef()
        }
        let failure = (event: j): j => {
            let _ = setProp(req, "onsuccess", nil())
            let _ = setProp(req, "onerror", nil())
            let _ = settle(reject, get(req, "error"))
            undef()
        }
        let _ = setProp(req, "onsuccess", callback(success))
        let _ = setProp(req, "onerror", callback(failure))
        undef()
    }
    promise(executor)
}
let clearTransaction = (tx: j): j => {
    let _ = setProp(tx, "oncomplete", nil())
    let _ = setProp(tx, "onerror", nil())
    let _ = setProp(tx, "onabort", nil())
    undef()
}
let transactionDone = (tx: j): j => {
    let executor = (resolve: j, reject: j): j => {
        let complete = (event: j): j => {
            let _ = clearTransaction(tx)
            let _ = settle(resolve, undef())
            undef()
        }
        let failure = (event: j): j => {
            let reason = get(tx, "error")
            let _ = clearTransaction(tx)
            let _ = settle(reject, (truth(reason) ? reason : abortError()))
            undef()
        }
        let _ = setProp(tx, "oncomplete", callback(complete))
        let _ = setProp(tx, "onerror", callback(failure))
        let _ = setProp(tx, "onabort", callback(failure))
        undef()
    }
    promise(executor)
}
let openDatabase = (name: j): j => {
    let req = call(globalValue("indexedDB"), "open", [name, num(1)])
    let upgrade = (event: j): j => {
        let options = objectValue()
        let _ = setProp(options, "keyPath", strVal("id"))
        let store = call(get(req, "result"), "createObjectStore", [strVal("rows"), options])
        let _ = call(store, "createIndex", [strVal("byGroup"), strVal("group")])
        undef()
    }
    let _ = setProp(req, "onupgradeneeded", callback(upgrade))
    request(req)
}
let writeBatch = (db: j, batch: j): j => {
    let tx = call(db, "transaction", [strVal("rows"), strVal("readwrite")])
    let completion = transactionDone(tx)
    let store = call(tx, "objectStore", [strVal("rows")])
    let submit = (row: j): j => {
        (truth(get(batch, "add")) ? request(call(store, "add", [row])) : request(call(store, "put", [row])))
    }
    let promises = mapValues(get(batch, "rows"), submit)
    let _ = push(promises, completion)
    let settled = call(globalValue("Promise"), "allSettled", [promises])
    if truth(get(batch, "abort")) {
        let _ = call(tx, "abort", [])
    }
    let collect = (items: j): j => {
        let outcome = (item: j): j => {
            let value = get(item, "value")
            (equalText(get(item, "status"), "rejected") ? errorName(get(get(item, "reason"), "name")) : (missing(value) ? strVal("done") : value))
        }
        mapValues(items, outcome)
    }
    chain(settled, collect)
}
let lookup = (source: j, keys: j): j => {
    let submit = (key: j): j => {
        request(call(source, "get", [key]))
    }
    call(globalValue("Promise"), "all", [mapValues(keys, submit)])
}
let storedValues = (rows: j): j => {
    let value = (row: j): j => {
        (missing(row) ? nil() : get(row, "value"))
    }
    mapValues(rows, value)
}
let readBack = (db: j, inputValue: j, transactions: j): j => {
    let tx = call(db, "transaction", [strVal("rows")])
    let completion = transactionDone(tx)
    let store = call(tx, "objectStore", [strVal("rows")])
    let index = call(store, "index", [strVal("byGroup")])
    let reads = lookup(store, get(inputValue, "keys"))
    let indexReads = lookup(index, get(inputValue, "groups"))
    let source = (truth(get(inputValue, "index")) ? index : store)
    let only = get(inputValue, "only")
    let bounds = (missing(only) ? nil() : call(globalValue("IDBKeyRange"), "only", [only]))
    let direction = (truth(get(inputValue, "reverse")) ? strVal("prev") : strVal("next"))
    let req = call(source, "openCursor", [bounds, direction])
    let step = get(inputValue, "step")
    let cursors = pack([])
    let rec visit = (cursor: j): j => {
        if truth(cursor) {
            let _ = push(cursors, pack([get(cursor, "key"), get(cursor, "primaryKey"), get(get(cursor, "value"), "value")]))
            if equalNum(step, 1) {
                let _ = call(cursor, "continue", [])
            } else {
                let _ = call(cursor, "advance", [step])
            }
            chain(request(req), visit)
        } else {
            cursors
        }
    }
    let cursorDone = chain(request(req), visit)
    let awaited = call(globalValue("Promise"), "all", [pack([reads, indexReads, cursorDone, completion])])
    let output = (result: j): j => {
        pack([transactions, storedValues(at(result, 0)), storedValues(at(result, 1)), at(result, 2)])
    }
    chain(awaited, output)
}
let exercise = (db: j, inputValue: j): j => {
    let transactions = pack([])
    let write = (batch: j): j => {
        let saved = (outcomes: j): j => {
            let _ = push(transactions, outcomes)
            undef()
        }
        chain(writeBatch(db, batch), saved)
    }
    let nextPhase = (unused: j): j => {
        readBack(db, inputValue, transactions)
    }
    chain(sequence(get(inputValue, "batches"), write), nextPhase)
}
let run = (inputValue: j): j => {
    let name = uniqueName()
    let opened = (db: j): j => {
        let close = (unused: j): j => {
            call(db, "close", [])
        }
        always(exercise(db, inputValue), close)
    }
    let cleanup = (unused: j): j => {
        request(call(globalValue("indexedDB"), "deleteDatabase", [name]))
    }
    always(chain(openDatabase(name), opened), cleanup)
}
