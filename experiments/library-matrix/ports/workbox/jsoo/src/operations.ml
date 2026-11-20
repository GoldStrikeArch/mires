open Host

let absolute path base =
    (get ((makeHost ("URL") ([|path; base|]))) ("href"))
let network source base =
    if (equalText (source) ("offline")) then (
        (bad ((strVal ("offline"))))
    ) else (
        let req = (makeHost ("Request") ([|(absolute ((suffix (source) (".mjs"))) (base))|])) in
        (fetchRequest (req))
    )
let precacheKey entries path base =
    let url = (makeHost ("URL") ([|path; base|])) in
    let _ = (setProp (url) ("hash") ((strVal ("")))) in
    let exact = (call (entries) ("get") ([|(get (url) ("href"))|])) in
    if (not (missing (exact))) then (
        exact
    ) else (
        let params = (get (url) ("searchParams")) in
        let keys = (collect ((call (params) ("keys") ([||])))) in
        let ignore key =
            let _ = if ((equalText (key) ("fbclid")) || (starts (key) ("utm_"))) then (
                let _ = (call (params) ("delete") ([|key|])) in
                ()
            ) in
            (undef ())
        in
        let _ = (mapValues (keys) (ignore)) in
        let stripped = (call (entries) ("get") ([|(get (url) ("href"))|])) in
        if (not (missing (stripped))) then (
            stripped
        ) else (
            let original = (get (url) ("pathname")) in
            let _ = (setProp (url) ("pathname") ((suffix (original) ("index.html")))) in
            let index = (if (ends (original) ("/")) then (call (entries) ("get") ([|(get (url) ("href"))|])) else (undef ())) in
            if (not (missing (index))) then (
                index
            ) else (
                let _ = (setProp (url) ("pathname") ((suffix (original) (".html")))) in
                (call (entries) ("get") ([|(get (url) ("href"))|]))
            )
        )
    )
let install entries cache source base =
    let updated = (pack ([||])) in
    let unchanged = (pack ([||])) in
    let check entry =
        let original = (at (entry) (0)) in
        let key = (at (entry) (1)) in
        let found hit =
            if (not (missing (hit))) then (
                let _ = (push (unchanged) ((relative (original) (base)))) in
                (undef ())
            ) else (
                let received res =
                    if (badStatus ((get (res) ("status")))) then (
                        (bad ((strVal ("bad-precaching-response"))))
                    ) else (
                        let stored _ =
                            let _ = (push (updated) ((relative (original) (base)))) in
                            (undef ())
                        in
                        (chain ((call (cache) ("put") ([|key; (call (res) ("clone") ([||]))|]))) (stored))
                    )
                in
                (chain ((network (source) (base))) (received))
            )
        in
        (chain ((call (cache) ("match") ([|key|]))) (found))
    in
    let complete _ =
        (pack ([|updated; unchanged|]))
    in
    (chain ((sequence ((collect ((call (entries) ("entries") ([||]))))) (check))) (complete))
let activate entries cache base =
    let active = (collect ((call (entries) ("values") ([||])))) in
    let removed = (pack ([||])) in
    let listKeys keys =
        let remove req =
            let path = (get (req) ("url")) in
            if (truth ((call (active) ("includes") ([|path|])))) then (
                (undef ())
            ) else (
                let deleted _ =
                    let _ = (push (removed) ((relative (path) (base)))) in
                    (undef ())
                in
                (chain ((call (cache) ("delete") ([|path|]))) (deleted))
            )
        in
        let complete _ =
            (call (removed) ("sort") ([||]))
        in
        (chain ((sequence (keys) (remove))) (complete))
    in
    (chain ((call (cache) ("keys") ([||]))) (listKeys))
let strategy entries cache name path source base =
    let key = (precacheKey (entries) (path) (base)) in
    let route = (relative (path) (base)) in
    if (not (missing (key))) then (
        let precached hit =
            (if (missing (hit)) then (network (source) (base)) else hit)
        in
        (chain ((call (cache) ("match") ([|key|]))) (precached))
    ) else (
        if (starts (route) ("network/")) then (
            (network (source) (base))
        ) else (
            if ((starts (route) ("first/")) || (starts (route) ("only/"))) then (
                let opened runtime =
                    let matched hit =
                        if (not (missing (hit))) then (
                            hit
                        ) else (
                            if (starts (route) ("only/")) then (
                                (bad ((strVal ("no-response"))))
                            ) else (
                                let fetched response =
                                    if (equalNum ((get (response) ("status"))) (200)) then (
                                        let stored _ =
                                            response
                                        in
                                        (chain ((call (runtime) ("put") ([|path; (call (response) ("clone") ([||]))|]))) (stored))
                                    ) else (
                                        response
                                    )
                                in
                                (chain ((network (source) (base))) (fetched))
                            )
                        )
                    in
                    (chain ((call (runtime) ("match") ([|path|]))) (matched))
                in
                (chain ((call ((globalValue ("caches"))) ("open") ([|(suffix (name) ("-runtime"))|]))) (opened))
            ) else (
                (ok ((nil ())))
            )
        )
    )
let fetchRequestForStep step base =
    let request = get step "request" in
    if truth request then request else
      let init = objectValue () in
      let verb = get step "method" in
      if truth verb then setProp init "method" verb;
      makeHost "Request" [|absolute (get step "path") base; init|]

let phase step name items base =
    let entries = (makeHost ("Map") ([||])) in
    let state = (objectValue ()) in
    let add item =
        let url = (makeHost ("URL") ([|(get (item) ("path")); base|])) in
        let original = (get (url) ("href")) in
        let revision = (get (item) ("revision")) in
        let _ = if (truth (revision)) then (
            let _ = (call ((get (url) ("searchParams"))) ("set") ([|(strVal ("__WB_REVISION__")); revision|])) in
            ()
        ) in
        let key = (get (url) ("href")) in
        let previous = (call (entries) ("get") ([|original|])) in
        let _ = if ((not (missing (previous))) && (not (same (previous) (key)))) then (
            let _ = (setProp (state) ("conflict") ((boolean (true)))) in
            ()
        ) in
        let _ = (call (entries) ("set") ([|original; key|])) in
        (undef ())
    in
    let _ = (mapValues (items) (add)) in
    if (truth ((get (state) ("conflict")))) then (
        (bad ((strVal ("conflicting revisions"))))
    ) else (
        let op = (get (step) ("phase")) in
        let source = (defaulted ((get (step) ("source"))) ((strVal ("alpha")))) in
        let opened cache =
            if (equalText (op) ("install")) then (
                (install (entries) (cache) (source) (base))
            ) else (
                if (equalText (op) ("activate")) then (
                    (activate (entries) (cache) (base))
                ) else (
                    if (equalText (op) ("message")) then (
                        if (equalText ((get (step) ("message"))) ("SKIP_WAITING")) then (
                            let skipped _ =
                                (boolean (true))
                            in
                            (chain ((skipWaiting ())) (skipped))
                        ) else (
                            (boolean (false))
                        )
                    ) else (
                        let req = fetchRequestForStep step base in
                        let verb = get req "method" in
                        if (not (equalText (verb) ("GET"))) then (
                            (nil ())
                        ) else (
                            let path = get req "url" in
                            let received response =
                                if (nullish (response)) then (
                                    (nil ())
                                ) else (
                                    let body text =
                                        (pack ([|(get (response) ("status")); text|]))
                                    in
                                    (chain ((call (response) ("text") ([||]))) (body))
                                )
                            in
                            let failed _ =
                                (pack ([|(strVal ("no-response"))|]))
                            in
                            (recoverPromise ((chain ((strategy (entries) (cache) (name) (path) (source) (base))) (received))) (failed))
                        )
                    )
                )
            )
        in
        (chain ((call ((globalValue ("caches"))) ("open") ([|name|]))) (opened))
    )
let run inputValue =
    let base = (absolute ((strVal ("experiments/library-matrix/libraries/workbox/"))) ((get ((globalValue ("location"))) ("href")))) in
    if (not (missing ((get (inputValue) ("phase"))))) then (
        let task = (phase (inputValue) ((get (inputValue) ("name"))) ((defaulted ((get (inputValue) ("manifest"))) ((pack ([||]))))) (base)) in
        let event = (get (inputValue) ("event")) in
        let _ = if (truth (event)) then (
            let _ = (call (event) ("waitUntil") ([|task|])) in
            ()
        ) in
        task
    ) else (
        let name = (uniqueName ()) in
        let state = (objectValue ()) in
        let _ = (setProp (state) ("manifest") ((pack ([||])))) in
        let results = (pack ([||])) in
        let execute step =
            let items = (defaulted ((get (step) ("manifest"))) ((get (state) ("manifest")))) in
            let _ = (setProp (state) ("manifest") (items)) in
            let collectResult result =
                let _ = (push (results) (result)) in
                (undef ())
            in
            (chain ((phase (step) (name) (items) (base))) (collectResult))
        in
        let complete _ =
            results
        in
        let cleanup _ =
            let caches = (globalValue ("caches")) in
            (call ((globalValue ("Promise"))) ("all") ([|(pack ([|(call (caches) ("delete") ([|name|])); (call (caches) ("delete") ([|(suffix (name) ("-runtime"))|]))|]))|]))
        in
        (always ((chain ((sequence ((get (inputValue) ("steps"))) (execute))) (complete))) (cleanup))
    )
