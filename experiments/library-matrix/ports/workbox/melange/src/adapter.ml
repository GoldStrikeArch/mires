(* The adapter (CONTRACT.md "Steps", "Fixtures" and "Output"): runs one script of Workbox API calls
   against this port, with the fixture plugins and the stand-in events, and builds the canonical
   output. The scenario's caches are deleted on every path once it has reported, or failed. *)

open Workbox_core
open Strategies
open Precaching

let fixture = "experiments/library-matrix/libraries/workbox/"

(* Well-formed SRI metadata that matches no fixture, so the fetch fails integrity. *)
let wrong_integrity = "sha256-" ^ String.make 43 'A' ^ "="

let string_field value key = Option.bind (Host.field value key) Host.to_string
let list_field value key = Option.value (Option.bind (Host.field value key) Host.to_list) ~default:[]
let text value = Host.of_string value

(* The parameters of a match that is a plain `true`. *)
let no_params = { cache_key = None; integrity = None }

let run input =
  let base = Result.fold (Host.url "./" ~base:(Some (Host.location_href ()))) ~ok:Host.href ~error:(fun _ -> "") in
  let owned = Option.is_none (Host.field input "cachePrefix") in
  let prefix =
    match string_field input "cachePrefix" with Some prefix -> prefix | None -> "mires-workbox-" ^ Host.random_uuid ()
  in
  let without_prefix prefix name =
    if String.starts_with ~prefix name then
      Some (String.sub name (String.length prefix) (String.length name - String.length prefix))
    else None
  in
  let relative url = Option.value (without_prefix base url) ~default:url in
  let cache_name name = prefix ^ "-" ^ name in
  let cache_label name = Option.value (without_prefix (prefix ^ "-") name) ~default:name in
  let source = ref "alpha" in
  let controllers = ref [] in
  let routers = ref [] in
  let routes = ref [] in
  let results = ref [] in
  let events = Host.array [] in
  let handlers = Host.array [] in
  let find table kind id =
    match List.assoc_opt id !table with
    | Some value -> Ok value
    | None -> Error (Thrown (Host.type_error ("the script names no " ^ kind ^ " " ^ id)))
  in
  let controller_of value key = find controllers "controller" (Option.value (string_field value key) ~default:"") in
  let router_of value = find routers "router" (Option.value (string_field value "router") ~default:"") in

  (* Output records. A thrown non-Error (the fail fixture's `value`) has no name; it prints as its
     string. *)
  let name_of error = if Host.is_error error then Host.error_name error else Host.string_of error in
  let failure = function
    | Workbox_error error ->
        let message = Host.string_of (Host.error_message error) in
        let message = Host.replace_all (Host.replace_all message base "") (prefix ^ "-") "" in
        Host.obj [ ("error", text (Host.error_name error)); ("message", text message) ]
    | Thrown error when Host.is_error error -> Host.obj [ ("error", text (Host.error_name error)) ]
    | Thrown error -> Host.obj [ ("thrown", text (Host.string_of error)) ]
  in
  let status = function Some response -> Host.of_int (Host.status response) | None -> Host.null in
  let url request = text (relative (Host.request_url request)) in
  let urls urls = Host.array (List.map (fun url -> text (relative url)) urls) in
  let record response =
    let status = Host.of_int (Host.status response) in
    let url = text (relative (Host.response_url response)) in
    let*? body = host (Host.text response) in
    return (Host.obj [ ("status", status); ("url", url); ("body", body) ])
  in
  let dump () =
    let rec entries cache out = function
      | [] -> return (Host.array (List.rev out))
      | request :: rest ->
          let*? response = host (Host.cache_match cache request) in
          let missing = Thrown (Host.type_error "a cache entry without a response") in
          let*! response = Option.to_result response ~none:missing in
          let*? record = record response in
          entries cache (Host.array [ url request; record ] :: out) rest
    in
    let rec each out = function
      | [] -> return (Host.array (List.rev out))
      | name :: rest when Option.is_none (without_prefix (prefix ^ "-") name) -> each out rest
      | name :: rest ->
          let*? cache = host (Host.caches_open name) in
          let*? requests = host (Host.cache_keys cache) in
          let*? entries = entries cache [] requests in
          each (Host.array [ text (cache_label name); entries ] :: out) rest
    in
    let*? names = host (Host.caches_keys ()) in
    each [] names
  in

  (* Fixture plugins (CONTRACT.md "Fixtures"). *)
  let network_plugin () =
    let request_will_fetch _ ~request =
      if !source = "offline" then fail (Thrown (Host.type_error "offline"))
      else
        let file = if !source = "beta" then "beta" else "alpha" in
        let*! target = thrown (Host.url (fixture ^ file ^ ".mjs") ~base:(Some (Host.location_href ()))) in
        let integrity = if !source = "broken" then wrong_integrity else Host.request_integrity request in
        let*! request = thrown (Host.request (Host.href target) { Host.no_init with integrity = Some integrity }) in
        return request
    in
    let fetch_did_succeed _ ~request:_ ~response =
      match !source with
      | "missing" ->
          let*! response = thrown (Host.response (text "missing") ~status:(Host.of_int 404)) in
          return response
      | "redirected" -> return (Host.mark_redirected response)
      | _ -> return response
    in
    { plugin with request_will_fetch = Some request_will_fetch; fetch_did_succeed = Some fetch_did_succeed }
  in
  (* Records go only into a log the plugin's own handlerWillStart created: a plugin before it can
     fail that callback, and the trace must not throw in the callbacks that follow. *)
  let trace_plugin label =
    let record { state; _ } entry =
      let log = Option.bind state (fun state -> Host.field state "log") in
      Option.iter (fun log -> Host.push log (Host.array entry)) log
    in
    {
      plugin with
      handler_will_start =
        Some
          (fun { state; _ } ~request ->
            let log = Host.array [] in
            Option.iter (fun state -> Host.set state "log" log) state;
            Host.push handlers (Host.array [ text label; url request; log ]);
            return ());
      cache_key_will_be_used =
        Some
          (fun cx ~mode ~request ~params:_ ->
            record cx [ text "cacheKeyWillBeUsed"; text (mode_name mode); url request ];
            return request);
      cached_response_will_be_used =
        Some
          (fun cx ~cache_name ~request ~cached_response ->
            let cache = text (cache_label cache_name) in
            record cx [ text "cachedResponseWillBeUsed"; cache; url request; status cached_response ];
            return cached_response);
      request_will_fetch =
        Some
          (fun cx ~request ->
            record cx [ text "requestWillFetch"; url request ];
            return request);
      fetch_did_succeed =
        Some
          (fun cx ~request ~response ->
            record cx [ text "fetchDidSucceed"; url request; status (Some response) ];
            return response);
      fetch_did_fail =
        Some
          (fun cx ~error ~original_request ~request ->
            record cx [ text "fetchDidFail"; url original_request; url request; text (name_of (error_value error)) ];
            return ());
      cache_did_update =
        Some
          (fun cx ~cache_name ~request ~old_response ~new_response ->
            let cache = text (cache_label cache_name) in
            record cx [ text "cacheDidUpdate"; cache; url request; status old_response; status (Some new_response) ];
            return ());
      handler_will_respond =
        Some
          (fun cx ~request:_ ~response ->
            record cx [ text "handlerWillRespond"; status (Some response) ];
            return response);
      handler_did_respond =
        Some
          (fun cx ~request:_ ~response ->
            record cx [ text "handlerDidRespond"; status response ];
            return ());
      handler_did_complete =
        Some
          (fun cx ~request:_ ~response ~error ->
            let error = Option.fold error ~none:Host.null ~some:(fun error -> text (name_of (error_value error))) in
            record cx [ text "handlerDidComplete"; status response; error ];
            return ());
      handler_did_error =
        Some
          (fun cx ~request:_ ~error ->
            record cx [ text "handlerDidError"; text (name_of (error_value error)) ];
            return None);
    }
  in
  (* `fail`: only the callback named `on`, which throws `new Error('fail')`, or `value` itself. *)
  let fail_plugin on value =
    let throw () = fail (Thrown (match value with Some value -> value | None -> Host.new_error "fail")) in
    match on with
    | "handlerWillStart" -> Ok { plugin with handler_will_start = Some (fun _ ~request:_ -> throw ()) }
    | "cacheKeyWillBeUsed" ->
        Ok { plugin with cache_key_will_be_used = Some (fun _ ~mode:_ ~request:_ ~params:_ -> throw ()) }
    | "cachedResponseWillBeUsed" ->
        Ok
          {
            plugin with
            cached_response_will_be_used = Some (fun _ ~cache_name:_ ~request:_ ~cached_response:_ -> throw ());
          }
    | "cacheWillUpdate" -> Ok { plugin with cache_will_update = Some (fun _ ~request:_ ~response:_ -> throw ()) }
    | "cacheDidUpdate" ->
        Ok
          {
            plugin with
            cache_did_update = Some (fun _ ~cache_name:_ ~request:_ ~old_response:_ ~new_response:_ -> throw ());
          }
    | "requestWillFetch" -> Ok { plugin with request_will_fetch = Some (fun _ ~request:_ -> throw ()) }
    | "fetchDidSucceed" -> Ok { plugin with fetch_did_succeed = Some (fun _ ~request:_ ~response:_ -> throw ()) }
    | "fetchDidFail" ->
        Ok { plugin with fetch_did_fail = Some (fun _ ~error:_ ~original_request:_ ~request:_ -> throw ()) }
    | "handlerWillRespond" ->
        Ok { plugin with handler_will_respond = Some (fun _ ~request:_ ~response:_ -> throw ()) }
    | "handlerDidRespond" -> Ok { plugin with handler_did_respond = Some (fun _ ~request:_ ~response:_ -> throw ()) }
    | "handlerDidComplete" ->
        Ok { plugin with handler_did_complete = Some (fun _ ~request:_ ~response:_ ~error:_ -> throw ()) }
    | "handlerDidError" -> Ok { plugin with handler_did_error = Some (fun _ ~request:_ ~error:_ -> throw ()) }
    | on -> Error (Thrown (Host.type_error ("unknown plugin callback " ^ on)))
  in
  let fixture_plugin spec =
    match Option.value (string_field spec "kind") ~default:"" with
    | "trace" -> Ok (trace_plugin (Option.value (string_field spec "label") ~default:""))
    | "keep" -> Ok { plugin with cache_will_update = Some (fun _ ~request:_ ~response -> return (Some response)) }
    | "drop" -> Ok { plugin with cache_will_update = Some (fun _ ~request:_ ~response:_ -> return None) }
    | "fail" -> fail_plugin (Option.value (string_field spec "on") ~default:"") (Host.field spec "value")
    | "fallback" ->
        let fallback_url = Option.value (string_field spec "url") ~default:"" in
        Result.map
          (fun precache_controller -> precache_fallback_plugin ~fallback_url ~precache_controller)
          (controller_of spec "controller")
    | kind -> Error (Thrown (Host.type_error ("unknown plugin " ^ kind)))
  in
  let fixture_plugins specs =
    List.fold_left
      (fun plugins spec ->
        Result.bind plugins (fun plugins -> Result.map (fun plugin -> plugins @ [ plugin ]) (fixture_plugin spec)))
      (Ok []) specs
  in
  (* Every plugin list the adapter builds starts with the network plugin. *)
  let plugins specs = Result.map (fun plugins -> network_plugin () :: plugins) (fixture_plugins specs) in

  (* Route handlers and options (CONTRACT.md "Steps"). *)
  let handler spec =
    match (string_field spec "strategy", string_field spec "precache", string_field spec "bound") with
    | Some name, _, _ -> (
        let cache_name = cache_name (Option.value (string_field spec "cacheName") ~default:"") in
        let strategy make =
          Result.map (fun plugins -> Routing.Strategy (make ~cache_name ~plugins)) (plugins (list_field spec "plugins"))
        in
        match name with
        | "CacheFirst" -> strategy Cache_first.make
        | "CacheOnly" -> strategy Cache_only.make
        | "NetworkFirst" -> strategy Network_first.make
        | "NetworkOnly" -> strategy Network_only.make
        | name -> Error (Thrown (Host.type_error ("unknown strategy " ^ name))))
    | None, Some id, _ ->
        Result.map
          (fun controller -> Routing.Strategy controller.Precache_controller.strategy)
          (find controllers "controller" id)
    | None, None, Some url ->
        Result.bind (controller_of spec "controller") (fun controller ->
            Precache_controller.create_handler_bound_to_url controller url)
    | None, None, None ->
        let body = Host.get spec "respond" and status = Host.get spec "status" in
        Ok (Routing.Callback (fun _ -> Host.resolve (thrown (Host.response body ~status))))
  in
  let route_with_options options controller =
    match options with
    | None -> Ok (precache_route controller)
    | Some options ->
        let given key = Host.field options key in
        let directory_index =
          Option.map
            (fun index -> if Host.truthy index then Some (Host.string_of index) else None)
            (given "directoryIndex")
        in
        let clean_urls = Option.map Host.truthy (given "cleanURLs") in
        let compile sources =
          List.fold_left
            (fun compiled source ->
              Result.bind compiled (fun compiled ->
                  Result.map (fun regexp -> compiled @ [ regexp ]) (thrown (Host.regexp (Host.string_of source)))))
            (Ok []) sources
        in
        let url_manipulation =
          Option.map
            (fun suffixes url ->
              List.map
                (fun suffix ->
                  let variant = Host.copy_url url in
                  Host.set_pathname variant (Host.pathname variant ^ Host.string_of suffix);
                  variant)
                suffixes)
            (Option.bind (given "urlManipulation") Host.to_list)
        in
        let ignored = Option.bind (given "ignoreURLParametersMatching") Host.to_list in
        Result.map
          (fun ignore_url_parameters_matching ->
            precache_route ?ignore_url_parameters_matching ?directory_index ?clean_urls ?url_manipulation controller)
          (match ignored with None -> Ok None | Some sources -> Result.map Option.some (compile sources))
  in
  let register value route =
    Option.iter (fun id -> routes := (id, route) :: !routes) (string_field value "id");
    let*! router = router_of value in
    Routing.Router.register_route router route;
    return Host.null
  in

  (* One event per event-driven step: a fresh stand-in event whose lifetime promises the step
     awaits before it reports, or the host event the caller supplied (service-worker mode). *)
  let lifetime event_type action =
    match Host.field input "event" with
    | Some native -> action { event_type = Host.event_type native; wait_until = Host.wait_until native }
    | None ->
        let pending = ref [] in
        let* outcome = action { event_type; wait_until = (fun promise -> pending := !pending @ [ promise ]) } in
        let rec settle count =
          match !pending with
          | [] -> Host.resolve count
          | batch ->
              pending := [];
              let* () = Host.all_settled batch in
              settle (count + List.length batch)
        in
        let* count = settle 0 in
        Host.push events (Host.array [ text event_type; Host.of_int count ]);
        Host.resolve outcome
  in

  let step value =
    match Option.value (string_field value "op") ~default:"" with
    | "network" ->
        source := Option.value (string_field value "source") ~default:"alpha";
        return Host.null
    | "controller" ->
        let fallback_to_network =
          Option.map (fun fallback -> Host.to_bool fallback <> Some false) (Host.field value "fallbackToNetwork")
        in
        let cache_name = cache_name (Option.value (string_field value "cacheName") ~default:"") in
        let*! plugins = plugins (list_field value "plugins") in
        let controller = Precache_controller.make ?fallback_to_network ~cache_name ~plugins () in
        controllers := (Option.value (string_field value "id") ~default:"", controller) :: !controllers;
        return Host.null
    | "addToCacheList" ->
        let*! controller = controller_of value "controller" in
        let*! () = Precache_controller.add_to_cache_list controller (list_field value "entries") in
        return Host.null
    | "addPlugins" ->
        let*! controller = controller_of value "controller" in
        let*! added = fixture_plugins (list_field value "plugins") in
        controller.strategy.plugins <- controller.strategy.plugins @ added;
        return Host.null
    | "install" ->
        let*! controller = controller_of value "controller" in
        let*? ({ updated_urls; not_updated_urls } : install_result) =
          lifetime "install" (Precache_controller.install controller)
        in
        return (Host.obj [ ("updatedURLs", urls updated_urls); ("notUpdatedURLs", urls not_updated_urls) ])
    | "activate" ->
        let*! controller = controller_of value "controller" in
        let*? { deleted_urls } = lifetime "activate" (Precache_controller.activate controller) in
        return (Host.obj [ ("deletedURLs", urls deleted_urls) ])
    | "getCachedURLs" ->
        let*! controller = controller_of value "controller" in
        return (urls (Precache_controller.get_cached_urls controller))
    | "getURLsToCacheKeys" ->
        let*! controller = controller_of value "controller" in
        let pairs = Precache_controller.get_urls_to_cache_keys controller in
        let pair (url, key) = Host.array [ text (relative url); text (relative key) ] in
        return (Host.array (List.map pair pairs))
    | "getCacheKeyForURL" -> (
        let*! controller = controller_of value "controller" in
        let url = Option.value (string_field value "url") ~default:"" in
        let*! key = Precache_controller.get_cache_key_for_url controller url in
        match key with Some key -> return (text (relative key)) | None -> return Host.null)
    | "matchPrecache" -> (
        let*! controller = controller_of value "controller" in
        let url = Option.value (string_field value "url") ~default:"" in
        let*! request =
          if Host.truthy (Host.get value "request") then
            Result.map (fun request -> Request request) (thrown (Host.request url Host.no_init))
          else Ok (Url url)
        in
        let*? response = Precache_controller.match_precache controller request in
        match response with Some response -> record response | None -> return Host.null)
    | "router" ->
        routers := (Option.value (string_field value "id") ~default:"", Routing.Router.make ()) :: !routers;
        return Host.null
    | "precacheRoute" ->
        let*! controller = controller_of value "controller" in
        let*! route = route_with_options (Host.field value "options") controller in
        register value route
    | "route" ->
        let match_ =
          if Host.truthy (Host.get value "crossOrigin") then fun ({ same_origin; _ } : Routing.match_options) ->
            Ok (if same_origin then None else Some no_params)
          else
            let prefix = Option.value (string_field value "prefix") ~default:"" in
            fun ({ url; _ } : Routing.match_options) ->
              Ok (if String.starts_with ~prefix (relative (Host.href url)) then Some no_params else None)
        in
        let*! handler = handler (Host.get value "handler") in
        register value (Routing.Route.make ?method_:(string_field value "method") ~match_ handler)
    | "defaultHandler" ->
        let*! router = router_of value in
        let*! handler = handler (Host.get value "handler") in
        Routing.Router.set_default_handler ?method_:(string_field value "method") router handler;
        return Host.null
    | "catchHandler" ->
        let*! target =
          match string_field value "route" with
          | None -> Result.map Routing.Router.set_catch_handler (router_of value)
          | Some id -> Result.map Routing.Route.set_catch_handler (find routes "route" id)
        in
        let*! handler = handler (Host.get value "handler") in
        target handler;
        return Host.null
    | "unregisterRoute" ->
        let*! router = router_of value in
        let*! route = find routes "route" (Option.value (string_field value "route") ~default:"") in
        let*! () = Routing.Router.unregister_route router route in
        return Host.null
    | "fetch" -> (
        let*! request =
          match string_field value "url" with
          | None -> Ok (Host.of_request (Host.get input "request"))
          | Some url ->
              let init =
                {
                  Host.no_init with
                  method_ = string_field value "method";
                  mode = string_field value "mode";
                  integrity = string_field value "integrity";
                }
              in
              thrown (Host.request url init)
        in
        let*! router = router_of value in
        let*? response =
          lifetime "fetch" (fun event ->
              match Routing.Router.handle_request router ~request ~event with
              | Error error -> fail error
              | Ok None -> return None
              | Ok (Some response) ->
                  let*? response = response in
                  return (Some response))
        in
        match response with Some response -> record response | None -> return Host.null)
    | "caches" -> dump ()
    | op -> fail (Thrown (Host.type_error ("unknown step " ^ op)))
  in

  let rec steps = function
    | [] ->
        let*? caches = dump () in
        let results = Host.array (List.rev !results) in
        return (Host.obj [ ("results", results); ("events", events); ("handlers", handlers); ("caches", caches) ])
    | value :: rest ->
        let* outcome = step value in
        results := (match outcome with Ok result -> result | Error error -> failure error) :: !results;
        steps rest
  in
  (* As the reference's `finally`: the scenario's caches go whatever happened, then the outcome. *)
  let delete_caches () =
    let rec each = function
      | [] -> return ()
      | name :: rest when Option.is_some (without_prefix (prefix ^ "-") name) ->
          let*? _ = host (Host.caches_delete name) in
          each rest
      | _ :: rest -> each rest
    in
    let*? names = host (Host.caches_keys ()) in
    each names
  in
  let output =
    let* outcome = steps (list_field input "steps") in
    let* deleted = if owned then delete_caches () else return () in
    Host.resolve (Result.map_error error_value (Result.bind deleted (fun () -> outcome)))
  in
  Host.rejecting output
