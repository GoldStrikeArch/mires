(* workbox-precaching 7.4.1: `createCacheKey`, `PrecacheController`, `PrecacheStrategy`,
   `PrecacheRoute` with its URL variations, and the precache plugins (cache key, install report,
   fallback, default cacheability and redirect copy). *)

open Workbox_core
open Strategies

let revision_search_param = "__WB_REVISION__"

(* `new URL(url, location.href)`. *)
let resolve url = thrown (Host.url url ~base:(Some (Host.location_href ())))

(* `createCacheKey(entry)`: the entry's URL resolved against the location, and for a truthy
   revision a cache key that carries it in `__WB_REVISION__`. The entry is the caller's value: a
   string, null, or a `{url, revision, integrity}` object. *)
let create_cache_key entry =
  let ( let+ ) result f = Result.map f result in
  let unexpected_type () =
    Error (workbox_error "add-to-cache-list-unexpected-type" ~details:(Host.obj [ ("entry", entry) ]))
  in
  if not (Host.truthy entry) then unexpected_type ()
  else
    match Host.to_string entry with
    | Some url ->
        let+ url = resolve url in
        (Host.href url, Host.href url)
    | None -> (
        let revision = Host.get entry "revision" in
        match Host.get entry "url" with
        | url when not (Host.truthy url) -> unexpected_type ()
        | url when not (Host.truthy revision) ->
            (* A URL that is not a string (outside the domain) goes through ToString. *)
            let+ url = resolve (Host.string_of url) in
            (Host.href url, Host.href url)
        | url -> (
            match (resolve (Host.string_of url), resolve (Host.string_of url)) with
            | Ok cache_key_url, Ok original_url ->
                Host.set_search_param cache_key_url revision_search_param revision;
                Ok (Host.href cache_key_url, Host.href original_url)
            | Error error, _ | _, Error error -> Error error))

(* A JS Map from strings as a list: setting a known key keeps its place. *)
let set_entry key value entries =
  if List.mem_assoc key entries then
    List.map (fun (known, old) -> (known, if known = key then value else old)) entries
  else entries @ [ (key, value) ]

(* `PrecacheInstallReportPlugin`: an install-time cache hit is not updated, a miss is. *)
type install_report = { mutable updated_urls : string list; mutable not_updated_urls : string list }

let install_report_plugin () =
  let report = { updated_urls = []; not_updated_urls = [] } in
  let handler_will_start { state; _ } ~request =
    Option.iter (fun state -> Host.set state "originalRequest" (Host.request_value request)) state;
    return ()
  in
  let cached_response_will_be_used { event; state } ~cache_name:_ ~request:_ ~cached_response =
    let original_request = Option.bind state (fun state -> Host.field state "originalRequest") in
    let original_request = Option.bind original_request Host.as_request in
    (match original_request with
    | Some original_request when event.event_type = "install" -> (
        let url = Host.request_url original_request in
        match cached_response with
        | Some _ -> report.not_updated_urls <- report.not_updated_urls @ [ url ]
        | None -> report.updated_urls <- report.updated_urls @ [ url ])
    | _ -> ());
    return cached_response
  in
  ( report,
    {
      plugin with
      handler_will_start = Some handler_will_start;
      cached_response_will_be_used = Some cached_response_will_be_used;
    } )

(* `PrecacheCacheKeyPlugin`: the route's cache key, else the manifest's key for the exact URL. It
   holds the controller's `getCacheKeyForURL`: the controller is being built when it is made. *)
let precache_cache_key_plugin ~get_cache_key_for_url =
  let cache_key_will_be_used _ ~mode:_ ~request ~params =
    let cache_key =
      match Option.bind params (fun params -> params.cache_key) with
      | Some cache_key when cache_key <> "" -> Ok (Some cache_key)
      | _ -> get_cache_key_for_url (Host.request_url request)
    in
    let*! cache_key = cache_key in
    match cache_key with
    | Some cache_key ->
        let init = { Host.no_init with headers = Some (Host.request_headers request) } in
        let*! request = thrown (Host.request cache_key init) in
        return request
    | None -> return request
  in
  { plugin with cache_key_will_be_used = Some cache_key_will_be_used }

module Precache_strategy = struct
  (* `PrecacheStrategy.defaultPrecacheCacheabilityPlugin`: statuses below 400. *)
  let default_precache_cacheability_plugin =
    {
      plugin with
      cache_will_update =
        Some (fun _ ~request:_ ~response -> return (if Host.status response >= 400 then None else Some response));
    }

  (* `PrecacheStrategy.copyRedirectedCacheableResponsesPlugin`. *)
  let copy_redirected_cacheable_responses_plugin =
    {
      plugin with
      cache_will_update =
        Some
          (fun _ ~request:_ ~response ->
            if Host.redirected response then
              let*? response = copy_response response in
              return (Some response)
            else return (Some response));
    }

  (* `_useDefaultCacheabilityPluginIfNeeded`: one cacheWillUpdate besides the redirect copy. It
     adds the default plugin when there is none, and removes it when there are more. *)
  let use_default_cacheability_plugin_if_needed strategy =
    let default_plugin_index = ref None in
    let cache_will_update_plugin_count = ref 0 in
    List.iteri
      (fun index plugin ->
        if plugin != copy_redirected_cacheable_responses_plugin then begin
          if plugin == default_precache_cacheability_plugin then default_plugin_index := Some index;
          if Option.is_some plugin.cache_will_update then incr cache_will_update_plugin_count
        end)
      strategy.plugins;
    if !cache_will_update_plugin_count = 0 then
      strategy.plugins <- strategy.plugins @ [ default_precache_cacheability_plugin ]
    else if !cache_will_update_plugin_count > 1 then
      Option.iter
        (fun default_index ->
          strategy.plugins <- List.filteri (fun index _ -> index <> default_index) strategy.plugins)
        !default_plugin_index

  (* `_handleInstall`: fetch, then cache, and fail the install when the response may not be cached. *)
  let handle_install strategy request handler =
    use_default_cacheability_plugin_if_needed strategy;
    let*? response = Strategy_handler.fetch handler request in
    let*? was_cached = Strategy_handler.cache_put handler request (Host.clone_response response) in
    if not was_cached then
      let details =
        Host.obj
          [ ("url", Host.of_string (Host.request_url request)); ("status", Host.of_int (Host.status response)) ]
      in
      fail (workbox_error "bad-precaching-response" ~details)
    else return response

  (* `_handleFetch`: the network, with the manifest's integrity unless the request is no-cors, and
     a cache repair only when that integrity vouches for the response. *)
  let handle_fetch strategy ~fallback_to_network request (handler : handler) =
    let params = Option.value handler.params ~default:{ cache_key = None; integrity = None } in
    if not fallback_to_network then
      let details =
        Host.obj
          [ ("cacheName", Host.of_string strategy.cache_name); ("url", Host.of_string (Host.request_url request)) ]
      in
      fail (workbox_error "missing-precache-entry" ~details)
    else
      let integrity_in_manifest = params.integrity in
      let integrity_in_request = Host.request_integrity request in
      let no_integrity_conflict = integrity_in_request = "" || Some integrity_in_request = integrity_in_manifest in
      let cors = Host.request_mode request <> "no-cors" in
      let integrity =
        match (cors, integrity_in_request) with
        | false, _ -> None
        | true, "" -> integrity_in_manifest
        | true, integrity -> Some integrity
      in
      let*! fetched = thrown (Host.copy_request request { Host.no_init with integrity }) in
      let*? response = Strategy_handler.fetch handler fetched in
      if Option.is_some integrity_in_manifest && no_integrity_conflict && cors then begin
        use_default_cacheability_plugin_if_needed strategy;
        let*? _was_cached = Strategy_handler.cache_put handler request (Host.clone_response response) in
        return response
      end
      else return response

  (* `_handle`: the cache, then install-time population, then the network fallback. *)
  let handle ~fallback_to_network strategy request (handler : handler) =
    let*? cached = Strategy_handler.cache_match handler request in
    match cached with
    | Some response -> return response
    | None when handler.event.event_type = "install" -> handle_install strategy request handler
    | None -> handle_fetch strategy ~fallback_to_network request handler

  (* `new PrecacheStrategy({cacheName, plugins, fallbackToNetwork})`, which appends the redirect
     copy plugin. *)
  let make ~cache_name ~plugins ~fallback_to_network =
    Strategy.make ~cache_name
      ~plugins:(plugins @ [ copy_redirected_cacheable_responses_plugin ])
      ~handle:(handle ~fallback_to_network)
end

type install_result = { updated_urls : string list; not_updated_urls : string list }
type cleanup_result = { deleted_urls : string list }

(* `string | Request`, as matchPrecache takes it. *)
type request_info = Url of string | Request of Host.request

module Precache_controller = struct
  (* The manifest's URL-to-key map is shared with the cache-key plugin, which needs it while the
     controller is being built. *)
  type t = {
    urls_to_cache_keys : (string * string) list ref;
    mutable urls_to_cache_modes : (string * string) list;
    mutable cache_keys_to_integrities : (string * string) list;
    strategy : strategy;
  }

  let lookup urls_to_cache_keys url =
    Result.map (fun url -> List.assoc_opt (Host.href url) urls_to_cache_keys) (resolve url)

  (* `new PrecacheController({cacheName, plugins, fallbackToNetwork = true})`: a PrecacheStrategy
     whose plugins end with this controller's cache-key plugin. *)
  let make ?(fallback_to_network = true) ~cache_name ~plugins () =
    let urls_to_cache_keys = ref [] in
    let get_cache_key_for_url url = lookup !urls_to_cache_keys url in
    let cache_key_plugin = precache_cache_key_plugin ~get_cache_key_for_url in
    {
      urls_to_cache_keys;
      urls_to_cache_modes = [];
      cache_keys_to_integrities = [];
      strategy = Precache_strategy.make ~cache_name ~fallback_to_network ~plugins:(plugins @ [ cache_key_plugin ]);
    }

  (* `getCacheKeyForURL(url)`: the exact URL resolved against the location, without variations. *)
  let get_cache_key_for_url controller url = lookup !(controller.urls_to_cache_keys) url

  let get_urls_to_cache_keys controller = !(controller.urls_to_cache_keys)
  let get_cached_urls controller = List.map fst !(controller.urls_to_cache_keys)
  let get_integrity_for_cache_key controller cache_key = List.assoc_opt cache_key controller.cache_keys_to_integrities

  (* `addToCacheList(entries)`: entry by entry, so the entries before a failing one stay added. The
     revisionless warning goes to the console after each entry, as in the production build. *)
  let add_to_cache_list controller entries =
    let urls_to_warn_about = ref [] in
    let add entry =
      (match Host.to_string entry with
      | Some url -> urls_to_warn_about := !urls_to_warn_about @ [ url ]
      | None when Host.truthy entry && Option.is_none (Host.field entry "revision") ->
          let url = Option.fold (Host.field entry "url") ~none:"" ~some:Host.string_of in
          urls_to_warn_about := !urls_to_warn_about @ [ url ]
      | None -> ());
      let ( let* ) = Result.bind in
      let* cache_key, url = create_cache_key entry in
      let revisioned = Option.is_none (Host.to_string entry) && Host.truthy (Host.get entry "revision") in
      let cache_mode = if revisioned then "reload" else "default" in
      let* () =
        match List.assoc_opt url !(controller.urls_to_cache_keys) with
        | Some first_entry when first_entry <> cache_key ->
            let details =
              Host.obj [ ("firstEntry", Host.of_string first_entry); ("secondEntry", Host.of_string cache_key) ]
            in
            Error (workbox_error "add-to-cache-list-conflicting-entries" ~details)
        | _ -> Ok ()
      in
      let* () =
        match Option.bind (Host.field entry "integrity") Host.to_string with
        | Some integrity when Option.is_none (Host.to_string entry) && integrity <> "" -> (
            match List.assoc_opt cache_key controller.cache_keys_to_integrities with
            | Some known when known <> integrity ->
                let details = Host.obj [ ("url", Host.of_string url) ] in
                Error (workbox_error "add-to-cache-list-conflicting-integrities" ~details)
            | _ ->
                controller.cache_keys_to_integrities <-
                  set_entry cache_key integrity controller.cache_keys_to_integrities;
                Ok ())
        | _ -> Ok ()
      in
      controller.urls_to_cache_keys := set_entry url cache_key !(controller.urls_to_cache_keys);
      controller.urls_to_cache_modes <- set_entry url cache_mode controller.urls_to_cache_modes;
      if !urls_to_warn_about <> [] then
        Host.warn
          ("Workbox is precaching URLs without revision info: " ^ String.concat ", " !urls_to_warn_about
         ^ "\nThis is generally NOT safe. Learn more at https://bit.ly/wb-precache");
      Ok ()
    in
    List.fold_left (fun result entry -> Result.bind result (fun () -> add entry)) (Ok ()) entries

  (* `install(event)`: each URL in turn through the strategy, under a new install report plugin,
     with the event's lifetime extended until it is done. The manifest is iterated live, as
     `for…of` iterates the Map: setting a known URL keeps its place. *)
  let install controller event =
    wait_until event (fun () ->
        let report, report_plugin = install_report_plugin () in
        controller.strategy.plugins <- controller.strategy.plugins @ [ report_plugin ];
        let rec each index =
          match List.nth_opt !(controller.urls_to_cache_keys) index with
          | None -> return { updated_urls = report.updated_urls; not_updated_urls = report.not_updated_urls }
          | Some (url, cache_key) ->
              let integrity = List.assoc_opt cache_key controller.cache_keys_to_integrities in
              let cache = List.assoc_opt url controller.urls_to_cache_modes in
              let init = { Host.no_init with integrity; cache; credentials = Some "same-origin" } in
              let*! request = thrown (Host.request url init) in
              let params = Some { cache_key = Some cache_key; integrity = None } in
              let response_done, handler_done = Strategy.handle_all controller.strategy { request; event; params } in
              (* Promise.all: the response's failure, else the completion's. *)
              let*? _ = response_done in
              let*? () = handler_done in
              each (index + 1)
        in
        each 0)

  (* `activate(event)`: deletes every request in the precache whose URL is no current cache key. *)
  let activate controller event =
    wait_until event (fun () ->
        let*? cache = host (Host.caches_open controller.strategy.cache_name) in
        let*? currently_cached_requests = host (Host.cache_keys cache) in
        let expected_cache_keys = List.map snd !(controller.urls_to_cache_keys) in
        let rec each deleted_urls = function
          | [] -> return { deleted_urls = List.rev deleted_urls }
          | request :: rest when List.mem (Host.request_url request) expected_cache_keys -> each deleted_urls rest
          | request :: rest ->
              let*? _ = host (Host.cache_delete cache request) in
              each (Host.request_url request :: deleted_urls) rest
        in
        each [] currently_cached_requests)

  (* `matchPrecache(request)`: opens the precache only for a URL in the manifest. *)
  let match_precache controller request =
    let url = match request with Request request -> Host.request_url request | Url url -> url in
    let*! cache_key = get_cache_key_for_url controller url in
    match cache_key with
    | Some cache_key ->
        let*? cache = host (Host.caches_open controller.strategy.cache_name) in
        host (Host.cache_match_url cache cache_key)
    | None -> return None

  (* `createHandlerBoundToURL(url)`: a handler callback that answers any request with this
     precached URL's entry, with `{cacheKey, ...params}`. *)
  let create_handler_bound_to_url controller url =
    let ( let* ) = Result.bind in
    let* cache_key = get_cache_key_for_url controller url in
    match cache_key with
    | None -> Error (workbox_error "non-precached-url" ~details:(Host.obj [ ("url", Host.of_string url) ]))
    | Some cache_key ->
        Ok
          (Routing.Callback
             (fun (options : handler_options) ->
               let*! request = thrown (Host.request url Host.no_init) in
               let params =
                 match options.params with
                 | None -> { cache_key = Some cache_key; integrity = None }
                 | Some params ->
                     let cache_key = if Option.is_some params.cache_key then params.cache_key else Some cache_key in
                     { cache_key; integrity = params.integrity }
               in
               Strategy.handle controller.strategy { options with request; params = Some params }))
end

(* `PrecacheFallbackPlugin({fallbackURL, precacheController})`: a failed strategy answers with a
   precached URL, when it is there. *)
let precache_fallback_plugin ~fallback_url ~precache_controller =
  let handler_did_error _ ~request:_ ~error:_ =
    Precache_controller.match_precache precache_controller (Url fallback_url)
  in
  { plugin with handler_did_error = Some handler_did_error }

(* `removeIgnoredSearchParams(urlObject, ignoreURLParametersMatching)`, on the URL itself. *)
let remove_ignored_search_params url_object ignore_url_parameters_matching =
  List.iter
    (fun param_name ->
      if List.exists (fun regexp -> Host.test regexp param_name) ignore_url_parameters_matching then
        Host.delete_search_param url_object param_name)
    (Host.search_param_names url_object);
  url_object

let default_ignore_url_parameters_matching () =
  List.filter_map (fun source -> Result.to_option (Host.regexp source)) [ "^utm_"; "^fbclid$" ]

(* `generateURLVariations(url, options)` as a lazy sequence, with the options' defaults: the route
   stops at the first variation in the manifest, and the later ones (and urlManipulation) are
   never computed. *)
let generate_url_variations ?ignore_url_parameters_matching ?(directory_index = Some "index.html")
    ?(clean_urls = true) ?url_manipulation url_object : string Seq.t =
 fun () ->
  Host.set_hash url_object "";
  let rest () =
    let ignore_url_parameters_matching =
      match ignore_url_parameters_matching with
      | Some patterns -> patterns
      | None -> default_ignore_url_parameters_matching ()
    in
    (* The URL object itself loses its ignored parameters; urlManipulation receives it so. *)
    let url_without_ignored_params = remove_ignored_search_params url_object ignore_url_parameters_matching in
    let with_suffix suffix () =
      let variant = Host.copy_url url_without_ignored_params in
      Host.set_pathname variant (Host.pathname variant ^ suffix);
      Seq.Cons (Host.href variant, Seq.empty)
    in
    let directory_url () =
      match directory_index with
      | Some directory_index when String.ends_with ~suffix:"/" (Host.pathname url_without_ignored_params) ->
          with_suffix directory_index ()
      | _ -> Seq.Nil
    in
    let clean_url () = if clean_urls then with_suffix ".html" () else Seq.Nil in
    let manipulated () =
      match url_manipulation with
      | Some url_manipulation -> List.to_seq (List.map Host.href (url_manipulation url_object)) ()
      | None -> Seq.Nil
    in
    Seq.Cons (Host.href url_without_ignored_params, Seq.append directory_url (Seq.append clean_url manipulated))
  in
  Seq.Cons (Host.href url_object, rest)

(* `new PrecacheRoute(controller, options)`: a GET route to the controller's strategy that matches
   the first variation of the request URL in the manifest, with `{cacheKey, integrity}`. *)
let precache_route ?ignore_url_parameters_matching ?directory_index ?clean_urls ?url_manipulation controller =
  let match_ ({ request; _ } : Routing.match_options) =
    let urls_to_cache_keys = Precache_controller.get_urls_to_cache_keys controller in
    Result.map
      (fun url_object ->
        url_object
        |> generate_url_variations ?ignore_url_parameters_matching ?directory_index ?clean_urls ?url_manipulation
        |> Seq.find_map (fun possible_url ->
               match List.assoc_opt possible_url urls_to_cache_keys with
               | Some cache_key when cache_key <> "" ->
                   let integrity = Precache_controller.get_integrity_for_cache_key controller cache_key in
                   Some { cache_key = Some cache_key; integrity }
               | _ -> None))
      (resolve (Host.request_url request))
  in
  Routing.Route.make ~match_ (Routing.Strategy controller.Precache_controller.strategy)
