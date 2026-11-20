open Host

let request req =
    let executor resolve reject =
        let success _ =
            let _ = (setProp (req) ("onsuccess") ((nil ()))) in
            let _ = (setProp (req) ("onerror") ((nil ()))) in
            let _ = (settle (resolve) ((get (req) ("result")))) in
            (undef ())
        in
        let failure _ =
            let _ = (setProp (req) ("onsuccess") ((nil ()))) in
            let _ = (setProp (req) ("onerror") ((nil ()))) in
            let _ = (settle (reject) ((get (req) ("error")))) in
            (undef ())
        in
        let _ = (setProp (req) ("onsuccess") ((callback (success)))) in
        let _ = (setProp (req) ("onerror") ((callback (failure)))) in
        (undef ())
    in
    (promise (executor))
let clearTransaction tx =
    let _ = (setProp (tx) ("oncomplete") ((nil ()))) in
    let _ = (setProp (tx) ("onerror") ((nil ()))) in
    let _ = (setProp (tx) ("onabort") ((nil ()))) in
    (undef ())
let transactionDone tx =
    let executor resolve reject =
        let complete _ =
            let _ = (clearTransaction (tx)) in
            let _ = (settle (resolve) ((undef ()))) in
            (undef ())
        in
        let failure _ =
            let reason = (get (tx) ("error")) in
            let _ = (clearTransaction (tx)) in
            let _ = (settle (reject) ((if (truth (reason)) then reason else (abortError ())))) in
            (undef ())
        in
        let _ = (setProp (tx) ("oncomplete") ((callback (complete)))) in
        let _ = (setProp (tx) ("onerror") ((callback (failure)))) in
        let _ = (setProp (tx) ("onabort") ((callback (failure)))) in
        (undef ())
    in
    (promise (executor))
let openDatabase name =
    let req = (call ((globalValue ("indexedDB"))) ("open") ([|name; (num (1))|])) in
    let upgrade _ =
        let options = (objectValue ()) in
        let _ = (setProp (options) ("keyPath") ((strVal ("id")))) in
        let store = (call ((get (req) ("result"))) ("createObjectStore") ([|(strVal ("rows")); options|])) in
        let _ = (call (store) ("createIndex") ([|(strVal ("byGroup")); (strVal ("group"))|])) in
        (undef ())
    in
    let _ = (setProp (req) ("onupgradeneeded") ((callback (upgrade)))) in
    (request (req))
let writeBatch db batch =
    let tx = (call (db) ("transaction") ([|(strVal ("rows")); (strVal ("readwrite"))|])) in
    let completion = (transactionDone (tx)) in
    let store = (call (tx) ("objectStore") ([|(strVal ("rows"))|])) in
    let submit row =
        (if (truth ((get (batch) ("add")))) then (request ((call (store) ("add") ([|row|])))) else (request ((call (store) ("put") ([|row|])))))
    in
    let promises = (mapValues ((get (batch) ("rows"))) (submit)) in
    let _ = (push (promises) (completion)) in
    let settled = (call ((globalValue ("Promise"))) ("allSettled") ([|promises|])) in
    let _ = if (truth ((get (batch) ("abort")))) then (
        let _ = (call (tx) ("abort") ([||])) in
        ()
    ) in
    let collect items =
        let outcome item =
            let value = (get (item) ("value")) in
            (if (equalText ((get (item) ("status"))) ("rejected")) then (errorName ((get ((get (item) ("reason"))) ("name")))) else (if (missing (value)) then (strVal ("done")) else value))
        in
        (mapValues (items) (outcome))
    in
    (chain (settled) (collect))
let lookup source keys =
    let submit key =
        (request ((call (source) ("get") ([|key|]))))
    in
    (call ((globalValue ("Promise"))) ("all") ([|(mapValues (keys) (submit))|]))
let storedValues rows =
    let value row =
        (if (missing (row)) then (nil ()) else (get (row) ("value")))
    in
    (mapValues (rows) (value))
let readBack db inputValue transactions =
    let tx = (call (db) ("transaction") ([|(strVal ("rows"))|])) in
    let completion = (transactionDone (tx)) in
    let store = (call (tx) ("objectStore") ([|(strVal ("rows"))|])) in
    let index = (call (store) ("index") ([|(strVal ("byGroup"))|])) in
    let reads = (lookup (store) ((get (inputValue) ("keys")))) in
    let indexReads = (lookup (index) ((get (inputValue) ("groups")))) in
    let source = (if (truth ((get (inputValue) ("index")))) then index else store) in
    let only = (get (inputValue) ("only")) in
    let bounds = (if (missing (only)) then (nil ()) else (call ((globalValue ("IDBKeyRange"))) ("only") ([|only|]))) in
    let direction = (if (truth ((get (inputValue) ("reverse")))) then (strVal ("prev")) else (strVal ("next"))) in
    let req = (call (source) ("openCursor") ([|bounds; direction|])) in
    let step = (get (inputValue) ("step")) in
    let cursors = (pack ([||])) in
    let rec visit cursor =
        if (truth (cursor)) then (
            let _ = (push (cursors) ((pack ([|(get (cursor) ("key")); (get (cursor) ("primaryKey")); (get ((get (cursor) ("value"))) ("value"))|])))) in
            let _ = if (equalNum (step) (1)) then (
                let _ = (call (cursor) ("continue") ([||])) in
                ()
            ) else (
                let _ = (call (cursor) ("advance") ([|step|])) in
                ()
            ) in
            (chain ((request (req))) (visit))
        ) else (
            cursors
        )
    in
    let cursorDone = (chain ((request (req))) (visit)) in
    let awaited = (call ((globalValue ("Promise"))) ("all") ([|(pack ([|reads; indexReads; cursorDone; completion|]))|])) in
    let output result =
        (pack ([|transactions; (storedValues ((at (result) (0)))); (storedValues ((at (result) (1)))); (at (result) (2))|]))
    in
    (chain (awaited) (output))
let exercise db inputValue =
    let transactions = (pack ([||])) in
    let write batch =
        let saved outcomes =
            let _ = (push (transactions) (outcomes)) in
            (undef ())
        in
        (chain ((writeBatch (db) (batch))) (saved))
    in
    let nextPhase _ =
        (readBack (db) (inputValue) (transactions))
    in
    (chain ((sequence ((get (inputValue) ("batches"))) (write))) (nextPhase))
let run inputValue =
    let name = (uniqueName ()) in
    let opened db =
        let close _ =
            (call (db) ("close") ([||]))
        in
        (always ((exercise (db) (inputValue))) (close))
    in
    let cleanup _ =
        (request ((call ((globalValue ("indexedDB"))) ("deleteDatabase") ([|name|]))))
    in
    (always ((chain ((openDatabase (name))) (opened))) (cleanup))
