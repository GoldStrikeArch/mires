(* workbox-strategies 7.4.1: the plugin interface, `Strategy` with its `handle`/`handleAll`
   lifecycle, `StrategyHandler`, and the CacheFirst, CacheOnly, NetworkFirst and NetworkOnly
   strategies. *)

open Workbox_core

type mode = Read | Write

let mode_name = function Read -> "read" | Write -> "write"

(* Route parameters: a PrecacheRoute match (`{cacheKey, integrity}`), or the `{cacheKey}` that
   install and createHandlerBoundToURL pass. *)
type params = { cache_key : string option; integrity : string option }

(* What every callback receives besides its own arguments: the event, and the plugin's state in
   this handler (a plain object, as Workbox's MapLikeObject), which is absent for a plugin the
   handler did not start with. *)
type cx = { event : event; state : Host.value option }

(* `WorkboxPlugin`: a record of optional lifecycle callbacks. Workbox calls only the callbacks a
   plugin defines. *)
type plugin = {
  handler_will_start : (cx -> request:Host.request -> unit outcome) option;
  cache_key_will_be_used :
    (cx -> mode:mode -> request:Host.request -> params:params option -> Host.request outcome) option;
  cached_response_will_be_used :
    (cx ->
    cache_name:string ->
    request:Host.request ->
    cached_response:Host.response option ->
    Host.response option outcome)
    option;
  cache_will_update :
    (cx -> request:Host.request -> response:Host.response -> Host.response option outcome) option;
  cache_did_update :
    (cx ->
    cache_name:string ->
    request:Host.request ->
    old_response:Host.response option ->
    new_response:Host.response ->
    unit outcome)
    option;
  request_will_fetch : (cx -> request:Host.request -> Host.request outcome) option;
  fetch_did_succeed :
    (cx -> request:Host.request -> response:Host.response -> Host.response outcome) option;
  fetch_did_fail :
    (cx -> error:error -> original_request:Host.request -> request:Host.request -> unit outcome)
    option;
  handler_will_respond :
    (cx -> request:Host.request -> response:Host.response -> Host.response outcome) option;
  handler_did_respond : (cx -> request:Host.request -> response:Host.response option -> unit outcome) option;
  handler_did_complete :
    (cx -> request:Host.request -> response:Host.response option -> error:error option -> unit outcome)
    option;
  handler_did_error : (cx -> request:Host.request -> error:error -> Host.response option outcome) option;
}

(* A plugin with no callbacks, to build plugins from. *)
let plugin =
  {
    handler_will_start = None;
    cache_key_will_be_used = None;
    cached_response_will_be_used = None;
    cache_will_update = None;
    cache_did_update = None;
    request_will_fetch = None;
    fetch_did_succeed = None;
    fetch_did_fail = None;
    handler_will_respond = None;
    handler_did_respond = None;
    handler_did_complete = None;
    handler_did_error = None;
  }

(* `Strategy`: its cache name, its plugin list (which Workbox and the caller change), and the
   subclass's `_handle`, which receives the strategy as `this`. *)
type strategy = {
  cache_name : string;
  mutable plugins : plugin list;
  handle : strategy -> Host.request -> handler -> Host.response outcome;
}

(* `StrategyHandler`: one request's run through a strategy. *)
and handler = {
  request : Host.request;
  event : event;
  params : params option;
  strategy : strategy;
  mutable cache_keys : (string * Host.request) list;
  mutable extend_lifetime_promises : unit outcome list;
  handler_deferred : unit Host.promise * (unit -> unit);
  plugin_state_map : (plugin * Host.value) list;
}

(* What `handle` and `handleAll` take (`HandlerCallbackOptions`). No handler in the subset reads its
   `url` member, so it is left out. *)
type handler_options = { request : Host.request; event : event; params : params option }

module Strategy_handler = struct
  (* The constructor: a fresh state object for each plugin the strategy has now, then the
     handler's own promise extends the event's lifetime. *)
  let make strategy ({ request; event; params } : handler_options) =
    let handler =
      {
        request;
        event;
        params;
        strategy;
        cache_keys = [];
        extend_lifetime_promises = [];
        handler_deferred = Host.deferred ();
        plugin_state_map = List.map (fun plugin -> (plugin, Host.empty ())) strategy.plugins;
      }
    in
    event.wait_until (Host.to_value (fst handler.handler_deferred));
    handler

  (* `iterateCallbacks(name)`: the plugins that define the callback, in order, each with what it
     receives. Like the generator's `for…of`, each step reads the strategy's live list, so a
     plugin added meanwhile is reached, without state. *)
  let callbacks (handler : handler) select =
    let rec from index () =
      match List.nth_opt handler.strategy.plugins index with
      | None -> Seq.Nil
      | Some plugin -> (
          match select plugin with
          | Some callback ->
              let state = List.assq_opt plugin handler.plugin_state_map in
              Seq.Cons ((callback, { event = handler.event; state }), from (index + 1))
          | None -> from (index + 1) ())
    in
    from 0

  (* `runCallbacks(name, param)`: every callback in turn. *)
  let rec run_callbacks callbacks call =
    match callbacks () with
    | Seq.Nil -> return ()
    | Seq.Cons ((callback, cx), rest) ->
        let*? () = call callback cx in
        run_callbacks rest call

  (* The loops that pass each callback's result to the next one. *)
  let rec chain_callbacks callbacks value call =
    match callbacks () with
    | Seq.Nil -> return value
    | Seq.Cons ((callback, cx), rest) ->
        let*? value = call callback cx value in
        chain_callbacks rest value call

  (* `hasCallback(name)`: `name in plugin`, over the strategy's current plugins. *)
  let has_callback (handler : handler) select =
    List.exists (fun plugin -> Option.is_some (select plugin)) handler.strategy.plugins

  (* `waitUntil(promise)`: extends the handler's lifetime until the promise settles. *)
  let wait_until (handler : handler) promise =
    let lifetime = Host.bind promise (fun outcome -> Host.resolve (Result.map ignore outcome)) in
    handler.extend_lifetime_promises <- handler.extend_lifetime_promises @ [ lifetime ];
    promise

  (* `getCacheKey(request, mode)`: cacheKeyWillBeUsed, once per URL and mode for this handler. *)
  let get_cache_key (handler : handler) request mode =
    let key = Host.request_url request ^ " | " ^ mode_name mode in
    match List.assoc_opt key handler.cache_keys with
    | Some effective_request -> return effective_request
    | None ->
        let*? effective_request =
          chain_callbacks
            (callbacks handler (fun plugin -> plugin.cache_key_will_be_used))
            request
            (fun callback cx request -> callback cx ~mode ~request ~params:handler.params)
        in
        handler.cache_keys <- (key, effective_request) :: handler.cache_keys;
        return effective_request

  (* `fetch(input)`: requestWillFetch, the network, then fetchDidSucceed, or fetchDidFail. The
     `navigate` branches (navigation preload, dropping fetchOptions) are out of the subset. *)
  let fetch (handler : handler) input =
    let original_request =
      if has_callback handler (fun plugin -> plugin.fetch_did_fail) then Some (Host.clone_request input)
      else None
    in
    let rec will_fetch callbacks request =
      match callbacks () with
      | Seq.Nil -> return request
      | Seq.Cons ((callback, cx), rest) -> (
          let* outcome = callback cx ~request:(Host.clone_request request) in
          match outcome with
          | Ok request -> will_fetch rest request
          | Error error when is_error error ->
              let message = Host.error_message (error_value error) in
              let details = Host.obj [ ("thrownErrorMessage", message) ] in
              fail (workbox_error "plugin-error-request-will-fetch" ~details)
          (* A thrown non-Error ends the loop, and the request goes out as it is. *)
          | Error _ -> return request)
    in
    let*? request = will_fetch (callbacks handler (fun plugin -> plugin.request_will_fetch)) input in
    let plugin_filtered_request = Host.clone_request request in
    let* outcome =
      let*? fetch_response = host (Host.fetch request) in
      chain_callbacks
        (callbacks handler (fun plugin -> plugin.fetch_did_succeed))
        fetch_response
        (fun callback cx response -> callback cx ~request:plugin_filtered_request ~response)
    in
    match (outcome, original_request) with
    | Ok response, _ -> return response
    | Error error, None -> fail error
    | Error error, Some original_request ->
        (* One clone of each for every fetchDidFail callback, as runCallbacks passes one param. *)
        let original_request = Host.clone_request original_request in
        let request = Host.clone_request plugin_filtered_request in
        let*? () =
          run_callbacks
            (callbacks handler (fun plugin -> plugin.fetch_did_fail))
            (fun callback cx -> callback cx ~error ~original_request ~request)
        in
        fail error

  (* `_ensureResponseSafeToCache(response)`: each cacheWillUpdate in turn, until one refuses; with
     none, only status 200. *)
  let ensure_response_safe_to_cache (handler : handler) response =
    let rec will_update callbacks response ~plugins_used =
      match callbacks () with
      | Seq.Nil -> return (if plugins_used || Host.status response = 200 then Some response else None)
      | Seq.Cons ((callback, cx), rest) -> (
          let*? response_to_cache = callback cx ~request:handler.request ~response in
          match response_to_cache with
          | Some response -> will_update rest response ~plugins_used:true
          | None -> return None)
    in
    will_update (callbacks handler (fun plugin -> plugin.cache_will_update)) response ~plugins_used:false

  (* `cacheMatch(key)`: `caches.match` of the read cache key in the strategy's cache, which opens
     no cache, then cachedResponseWillBeUsed. *)
  let cache_match (handler : handler) key =
    let cache_name = handler.strategy.cache_name in
    let*? effective_request = get_cache_key handler key Read in
    let*? cached_response = host (Host.caches_match effective_request ~cache_name) in
    chain_callbacks
      (callbacks handler (fun plugin -> plugin.cached_response_will_be_used))
      cached_response
      (fun callback cx cached_response ->
        callback cx ~cache_name ~request:effective_request ~cached_response)

  (* `cachePut(key, response)`: one timer task later, the write cache key, the cacheability check,
     the put and cacheDidUpdate. Answers whether it cached. `cache-put-with-no-response` cannot
     happen: the response is a Response by type. *)
  let cache_put (handler : handler) key response =
    let* () = timeout 0 in
    let*? effective_request = get_cache_key handler key Write in
    let*? response_to_cache = ensure_response_safe_to_cache handler response in
    match response_to_cache with
    | None -> return false
    | Some response_to_cache -> (
        let cache_name = handler.strategy.cache_name in
        let*? cache = host (Host.caches_open cache_name) in
        let has_cache_update_callback = has_callback handler (fun plugin -> plugin.cache_did_update) in
        let*? old_response =
          if has_cache_update_callback then
            cache_match_ignore_params cache (Host.clone_request effective_request) [ "__WB_REVISION__" ]
          else return None
        in
        let stored =
          if has_cache_update_callback then Host.clone_response response_to_cache else response_to_cache
        in
        let* put = host (Host.cache_put cache effective_request stored) in
        match put with
        (* An Error is rethrown (a QuotaExceededError after the registered quota callbacks, of which
           there are none here); any other thrown value is ignored. *)
        | Error error when is_error error -> fail error
        | Ok () | Error _ ->
            let*? () =
              run_callbacks
                (callbacks handler (fun plugin -> plugin.cache_did_update))
                (fun callback cx ->
                  callback cx ~cache_name ~request:effective_request ~old_response
                    ~new_response:(Host.clone_response response_to_cache))
            in
            return true)

  (* `fetchAndCachePut(input)`: the response now, and its cachePut as an extended lifetime. *)
  let fetch_and_cache_put (handler : handler) input =
    let*? response = fetch handler input in
    let response_clone = Host.clone_response response in
    ignore (wait_until handler (cache_put handler input response_clone));
    return response

  (* `doneWaiting()`: settles every extended lifetime in batches, including ones added meanwhile,
     and fails with the first rejection of a batch. *)
  let rec done_waiting (handler : handler) =
    match handler.extend_lifetime_promises with
    | [] -> return ()
    | promises ->
        handler.extend_lifetime_promises <- [];
        let rec settle first = function
          | [] -> ( match first with Some error -> fail error | None -> done_waiting handler)
          | promise :: rest -> (
              let* outcome = promise in
              match (first, outcome) with
              | None, Error error -> settle (Some error) rest
              | _ -> settle first rest)
        in
        settle None promises

  let destroy (handler : handler) = snd handler.handler_deferred ()
end

let no_response ?error request =
  let url = ("url", Host.of_string (Host.request_url request)) in
  let error = Option.map (fun error -> ("error", error_value error)) error in
  workbox_error "no-response" ~details:(Host.obj (url :: Option.to_list error))

(* The error Workbox keeps from a catch: only an Error. *)
let kept error = if is_error error then Some error else None

module Strategy = struct
  open Strategy_handler

  (* `new Strategy({cacheName, plugins})`. The cache name is always supplied, so
     `cacheNames.getRuntimeName(cacheName)` returns it. *)
  let make ~cache_name ~plugins ~handle = { cache_name; plugins; handle }

  (* `_getResponse`: handlerWillStart, `_handle`, handlerDidError on failure, handlerWillRespond. *)
  let get_response (handler : handler) request =
    let*? () =
      run_callbacks
        (callbacks handler (fun plugin -> plugin.handler_will_start))
        (fun callback cx -> callback cx ~request)
    in
    let* handled = handler.strategy.handle handler.strategy request handler in
    let caught =
      match handled with
      | Ok response when Host.response_type response = "error" -> Error (Some response, no_response request)
      | Ok response -> Ok response
      | Error error -> Error (None, error)
    in
    let*? response =
      match caught with
      | Ok response -> return response
      | Error (response, error) -> (
          let rec did_error callbacks response =
            match callbacks () with
            | Seq.Nil -> return response
            | Seq.Cons ((callback, cx), rest) -> (
                let*? response = callback cx ~request ~error in
                match response with Some _ -> return response | None -> did_error rest response)
          in
          let*? response =
            if is_error error then did_error (callbacks handler (fun plugin -> plugin.handler_did_error)) response
            else return response
          in
          (* A response of type "error" that no handlerDidError replaced is returned, not rethrown. *)
          match response with Some response -> return response | None -> fail error)
    in
    chain_callbacks
      (callbacks handler (fun plugin -> plugin.handler_will_respond))
      response
      (fun callback cx response -> callback cx ~request ~response)

  (* `_awaitComplete`: handlerDidRespond and the handler's extended lifetimes, then
     handlerDidComplete with the Error among them, if any, then the handler's own promise. *)
  let await_complete response_done (handler : handler) request =
    let* response = response_done in
    let response = Result.to_option response in
    let* finished =
      let*? () =
        run_callbacks
          (callbacks handler (fun plugin -> plugin.handler_did_respond))
          (fun callback cx -> callback cx ~request ~response)
      in
      done_waiting handler
    in
    (* A thrown non-Error is dropped. *)
    let error = match finished with Ok () -> None | Error error -> kept error in
    let*? () =
      run_callbacks
        (callbacks handler (fun plugin -> plugin.handler_did_complete))
        (fun callback cx -> callback cx ~request ~response ~error)
    in
    destroy handler;
    match error with Some error -> fail error | None -> return ()

  (* `handleAll(options)`: a handler for the request, and two promises that both start now: the
     response, and the handler's completion. *)
  let handle_all strategy (options : handler_options) =
    let handler = Strategy_handler.make strategy options in
    let response_done = get_response handler options.request in
    let handler_done = await_complete response_done handler options.request in
    (response_done, handler_done)

  (* `handle(options)`: the response; the completion keeps running. *)
  let handle strategy options = fst (handle_all strategy options)
end

module Cache_first = struct
  let handle _ request handler =
    let*? cached = Strategy_handler.cache_match handler request in
    let* response =
      match cached with
      | Some response -> return response
      | None -> Strategy_handler.fetch_and_cache_put handler request
    in
    match response with
    | Ok response -> return response
    | Error error -> fail (no_response request ?error:(kept error))

  let make ~cache_name ~plugins = Strategy.make ~cache_name ~plugins ~handle
end

module Cache_only = struct
  let handle _ request handler =
    let*? response = Strategy_handler.cache_match handler request in
    match response with Some response -> return response | None -> fail (no_response request)

  let make ~cache_name ~plugins = Strategy.make ~cache_name ~plugins ~handle
end

(* `cacheOkAndOpaquePlugin`: status 200, or 0 for an opaque response. *)
let cache_ok_and_opaque_plugin =
  {
    plugin with
    cache_will_update =
      Some
        (fun _ ~request:_ ~response ->
          return (if Host.status response = 200 || Host.status response = 0 then Some response else None));
  }

module Network_first = struct
  (* `_getNetworkPromise`: fetchAndCachePut, and the cache when that failed or answered nothing.
     There is no timeout to clear. *)
  let get_network_promise request handler =
    (* Only an Error is kept from the catch, but both branches go to the cache. *)
    let* outcome = Strategy_handler.fetch_and_cache_put handler request in
    match outcome with
    | Ok response -> return (Some response)
    | Error _ -> Strategy_handler.cache_match handler request

  (* `_handle` without networkTimeoutSeconds: the network promise raced alone, then itself again,
     and both waits extend the handler's lifetime. `Promise.race([a])` settles as `a` does. *)
  let handle _ request handler =
    let network_promise = get_network_promise request handler in
    let raced () =
      let*? response = Strategy_handler.wait_until handler network_promise in
      match response with Some _ -> return response | None -> network_promise
    in
    let*? response = Strategy_handler.wait_until handler (raced ()) in
    match response with Some response -> return response | None -> fail (no_response request)

  (* The constructor puts its 200-or-opaque plugin first unless a plugin has a cacheWillUpdate. *)
  let make ~cache_name ~plugins =
    let strategy = Strategy.make ~cache_name ~plugins ~handle in
    if not (List.exists (fun plugin -> Option.is_some plugin.cache_will_update) strategy.plugins) then
      strategy.plugins <- cache_ok_and_opaque_plugin :: strategy.plugins;
    strategy
end

module Network_only = struct
  (* `_handle` without networkTimeoutSeconds: the race holds the fetch alone. *)
  let handle _ request handler =
    let* response = Strategy_handler.fetch handler request in
    match response with
    | Ok response -> return response
    | Error error -> fail (no_response request ?error:(kept error))

  let make ~cache_name ~plugins = Strategy.make ~cache_name ~plugins ~handle
end
