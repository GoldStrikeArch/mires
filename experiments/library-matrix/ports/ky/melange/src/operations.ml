open Host

let makeError name message =
    let value = (makeHost ("Error") ([|message|])) in
    let _ = (setProp (value) ("name") (name)) in
    value
let buildRequest c =
    let init = (objectValue ()) in
    let headers = (makeHost ("Headers") ([|(get (c) ("headers"))|])) in
    let _ = (setProp (init) ("method") (let m = get c "method" in if nullish m then strVal "GET" else strVal (canonicalMethod (as_string m)))) in
    let _ = (setProp (init) ("headers") (headers)) in
    let payload = (get (c) ("json")) in
    let _ = if (not (missing (payload))) then (
        let _ = (setProp (init) ("body") ((call ((globalValue ("JSON"))) ("stringify") ([|payload|])))) in
        let _ = if (nullish (header headers "content-type")) then (
            let _ = (call (headers) ("set") ([|(strVal ("content-type")); (strVal ("application/json"))|])) in
            ()
        ) in
        ()
    ) in
    let _ = if (not (truth ((header (headers) ("accept"))))) then (
        let mime = (if (equalText ((get (c) ("read"))) ("json")) then (strVal ("application/json")) else (strVal ("text/*"))) in
        let _ = (call (headers) ("set") ([|(strVal ("accept")); mime|])) in
        ()
    ) in
    (makeHost ("Request") ([|(get (c) ("url")); init|]))
let fixtureFetch c req user count trace =
    let h = (get (req) ("headers")) in
    let respond body =
        let _ = (push (trace) ((pack ([|(strVal ("fetch")); (get (req) ("method")); (get (req) ("url")); (header (h) ("accept")); (header (h) ("content-type")); (header (h) ("x-matrix")); body|])))) in
        let _ = if (equalText ((get (c) ("abort"))) ("fetch")) then (
            let _ = (call (user) ("abort") ([|(makeError ((strVal ("AbortError"))) ((strVal ("cancelled"))))|])) in
            ()
        ) in
        let signal = (get (req) ("signal")) in
        if (truth ((get (signal) ("aborted")))) then (
            (bad ((get (signal) ("reason"))))
        ) else (
            let reply = (lastOrIndex ((get (c) ("replies"))) (count)) in
            if (equalText ((get (reply) ("kind"))) ("network")) then (
                (bad ((makeHost ("TypeError") ([|(strVal ("Failed to fetch"))|]))))
            ) else (
                if (equalText ((get (reply) ("kind"))) ("stall")) then (
                    let stall _resolve reject =
                        let aborted _ =
                            let _ = (settle (reject) ((get (signal) ("reason")))) in
                            (undef ())
                        in
                        let options = (objectValue ()) in
                        let _ = (setProp (options) ("once") ((boolean (true)))) in
                        let _ = (call (signal) ("addEventListener") ([|(strVal ("abort")); (callback (aborted)); options|])) in
                        (undef ())
                    in
                    (promise (stall))
                ) else (
                    let options = (objectValue ()) in
                    let _ = (setProp (options) ("status") ((get (reply) ("status")))) in
                    let _ = (setProp (options) ("headers") ((get (reply) ("headers")))) in
                    (makeHost ("Response") ([|(get (reply) ("body")); options|]))
                )
            )
        )
    in
    (chain ((call (req) ("text") ([||]))) (respond))
let dispatch c base user count trace =
    let ctl = (makeHost ("AbortController") ([||])) in
    let signal = (call ((globalValue ("AbortSignal"))) ("any") ([|(pack ([|(get (user) ("signal")); (get (ctl) ("signal"))|]))|])) in
    let init = (objectValue ()) in
    let _ = (setProp (init) ("signal") (signal)) in
    let req = (makeHost ("Request") ([|(call (base) ("clone") ([||])); init|])) in
    let fetching = (fixtureFetch (c) (req) (user) (count) (trace)) in
    let timeout = (get (c) ("timeout")) in
    if (isNumeric (timeout)) then (
        let holder = (objectValue ()) in
        let executor _resolve reject =
            let expiry _ =
                let _ = (settle (reject) ((makeError ((strVal ("TimeoutError"))) ((strVal ("Request timed out")))))) in
                let _ = (call (ctl) ("abort") ([||])) in
                (undef ())
            in
            let _ = (setProp (holder) ("timer") ((timer ((callback (expiry))) (timeout)))) in
            (undef ())
        in
        let expiryTask = (promise (executor)) in
        let clear _ =
            let _ = (stopTimer ((get (holder) ("timer")))) in
            (undef ())
        in
        (always ((call ((globalValue ("Promise"))) ("race") ([|(pack ([|fetching; expiryTask|]))|]))) (clear))
    ) else (
        fetching
    )
let parseJson body =
    (call ((globalValue ("JSON"))) ("parse") ([|body|]))
let errorData response =
    let h = (get (response) ("headers")) in
    let read body =
        if (not (truth (body))) then (
            (nil ())
        ) else (
            if (jsonType ((header (h) ("content-type")))) then (
                let invalid _ =
                    (nil ())
                in
                (recoverPromise ((chain ((ok (body))) (parseJson))) (invalid))
            ) else (
                body
            )
        )
    in
    (chain ((call (response) ("text") ([||]))) (read))
let retryDelay c verb count fault trace =
    let name = (at (fault) (0)) in
    let status = (at (fault) (1)) in
    let raw = (at (fault) (3)) in
    let eligible = ((allowsMethod (c) (verb)) && (smaller (count) ((get (c) ("limit"))))) in
    let serverStatus = ((equalNum (status) (413)) || (equalNum (status) (429)) || (equalNum (status) (503))) in
    let http = (equalText (name) ("HTTPError")) in
    let retriable = (if http then (allowsStatus (c) (status)) else ((equalText (name) ("NetworkError")) || ((equalText (name) ("TimeoutError")) && (truth ((get (c) ("retryOnTimeout"))))))) in
    if ((not eligible) || (not retriable) || (http && (equalNum (status) (413)) && (not (truth (raw))))) then (
        (nil ())
    ) else (
        let after = (if (http && serverStatus && (truth (raw))) then (parseAfter (raw)) else (nil ())) in
        if (not (nullish (after))) then (
            (lesser (after) ((get (c) ("maxRetryAfter"))))
        ) else (
            let ms = (exponential (count)) in
            let _ = (push (trace) ((pack ([|(strVal ("backoff")); (increment (count)); ms|])))) in
            (lesser (ms) ((get (c) ("backoffLimit"))))
        )
    )
let outputError trace count f hook =
    let _ = if (truth (hook)) then (
        let _ = (push (trace) ((pack ([|(strVal ("beforeError")); count; (at (f) (0))|])))) in
        ()
    ) in
    (pack ([|trace; (pack ([|(strVal ("error")); (at (f) (0)); (at (f) (1)); (at (f) (2))|]))|]))
let rec attempt c base user count trace =
    let onFailure f =
        let delay = (retryDelay (c) ((get (base) ("method"))) (count) (f) (trace)) in
        if (nullish (delay)) then (
            (outputError (trace) (count) (f) ((boolean (true))))
        ) else (
            let proceed _ =
                let following = (increment (count)) in
                let _ = (push (trace) ((pack ([|(strVal ("beforeRetry")); following; (at (f) (0))|])))) in
                let _ = (call ((get (base) ("headers"))) ("set") ([|(strVal ("x-matrix")); (retryTag (following))|])) in
                let _ = if (equalText ((get (c) ("abort"))) ("retry")) then (
                    let _ = (call (user) ("abort") ([|(makeError ((strVal ("AbortError"))) ((strVal ("cancelled"))))|])) in
                    ()
                ) in
                (attempt (c) (base) (user) (following) (trace))
            in
            (chain ((waitTime (delay))) (proceed))
        )
    in
    let success response =
        let status = (get (response) ("status")) in
        let _ = (push (trace) ((pack ([|(strVal ("afterResponse")); count; status|])))) in
        if ((not (truth ((get (response) ("ok"))))) && (truth ((get (c) ("throwHttpErrors"))))) then (
            let failed data =
                (onFailure ((pack ([|(strVal ("HTTPError")); status; data; (header ((get (response) ("headers"))) ("retry-after"))|]))))
            in
            (chain ((errorData (response))) (failed))
        ) else (
            let result payload =
                (pack ([|trace; (pack ([|(strVal ("ok")); payload|]))|]))
            in
            let body = (call (response) ("text") ([||])) in
            if (equalText ((get (c) ("read"))) ("json")) then (
                let invalid _ =
                    (outputError (trace) (count) ((pack ([|(strVal ("SyntaxError")); (nil ()); (nil ()); (nil ())|]))) ((boolean (false))))
                in
                (recoverPromise ((chain ((chain (body) (parseJson))) (result))) (invalid))
            ) else (
                (chain (body) (result))
            )
        )
    in
    let failure error =
        let name = (get (error) ("name")) in
        let wrapped = (if ((equalText (name) ("TypeError")) && (equalText ((get (error) ("message"))) ("Failed to fetch"))) then (strVal ("NetworkError")) else name) in
        (onFailure ((pack ([|wrapped; (nil ()); (nil ()); (nil ())|]))))
    in
    (handled ((dispatch (c) (base) (user) (count) (trace))) (success) (failure))
let requestCase c =
    let trace = (pack ([|(pack ([|(strVal ("init"))|]))|])) in
    let user = (makeHost ("AbortController") ([||])) in
    let _ = if (equalText ((get (c) ("abort"))) ("before")) then (
        let _ = (call (user) ("abort") ([|(makeError ((strVal ("AbortError"))) ((strVal ("cancelled"))))|])) in
        ()
    ) in
    let ready base =
        if (truth ((get ((get (user) ("signal"))) ("aborted")))) then (
            (ok ((outputError (trace) ((num (0))) ((pack ([|(strVal ("AbortError")); (nil ()); (nil ()); (nil ())|]))) ((boolean (true))))))
        ) else (
            let _ = (push (trace) ((pack ([|(strVal ("beforeRequest")); (num (0))|])))) in
            let _ = (call ((get (base) ("headers"))) ("set") ([|(strVal ("x-matrix")); (strVal ("initial"))|])) in
            (attempt (c) (base) (user) ((num (0))) (trace))
        )
    in
    let invalid error = outputError trace (num 0) (pack [|get error "name"; nil (); nil (); nil ()|]) (boolean false) in
    handled (chain (ok c) buildRequest) ready invalid
let run inputValue =
    let results = (pack ([||])) in
    let evaluate c =
        let save result =
            let _ = (push (results) (result)) in
            (undef ())
        in
        (chain ((requestCase (c))) (save))
    in
    let complete _ =
        results
    in
    (chain ((sequence ((get (inputValue) ("cases"))) (evaluate))) (complete))
