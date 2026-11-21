(* workbox-routing 7.4.1: `Route` and `Router` (routes per method, the http(s) guard, matching with
   parameter normalisation, default handlers, route and global catch handlers). *)

open Workbox_core
open Strategies

(* A route handler as `normalizeHandler` leaves it: an object with `handle` (a strategy), or a
   handler callback. *)
type handler = Strategy of strategy | Callback of (handler_options -> Host.response outcome)

let handle handler options =
  match
    Host.catching (fun () ->
        match handler with Strategy strategy -> Strategy.handle strategy options | Callback callback -> callback options)
  with
  | Ok promise -> promise
  | Error error -> fail (Thrown error)

(* What a match callback receives. No matcher in the subset reads the event Workbox also passes. *)
type match_options = { url : Host.url; same_origin : bool; request : Host.request }

module Route = struct
  (* `match_` answers [None] for no match, else the match's parameters, which are empty for a
     plain `true`. It can throw, as a JavaScript match callback can. *)
  type t = {
    handler : handler;
    match_ : match_options -> (params option, error) result;
    method_ : string;
    mutable catch_handler : handler option;
  }

  let make ?(method_ = "GET") ~match_ handler = { handler; match_; method_; catch_handler = None }
  let set_catch_handler route handler = route.catch_handler <- Some handler
end

module Router = struct
  type t = {
    mutable routes : (string * Route.t list) list;
    mutable default_handler_map : (string * handler) list;
    mutable catch_handler : handler option;
  }

  let make () = { routes = []; default_handler_map = []; catch_handler = None }

  (* `findMatchingRoute`: the first route for the request's method that matches. Parameters that
     are empty (a plain `true`) become undefined. *)
  let find_matching_route router (options : match_options) =
    let rec find = function
      | [] -> Ok None
      | (route : Route.t) :: rest -> (
          match route.match_ options with
          | Error error -> Error error
          | Ok None -> find rest
          | Ok (Some { cache_key = None; integrity = None }) -> Ok (Some (route, None))
          | Ok (Some params) -> Ok (Some (route, Some params)))
    in
    find (Option.value (List.assoc_opt (Host.request_method options.request) router.routes) ~default:[])

  (* `handleRequest({request, event})`: [None] when the request is not http(s) or nothing handles
     it, else the response, behind the catch handlers when there are any. A synchronous throw (a
     match callback's) is [Error]. *)
  let handle_request router ~request ~event =
    let respond route params handler =
      let response = handle handler { request; event; params } in
      let catch_handler = Option.join (Option.map (fun route -> route.Route.catch_handler) route) in
      (* The global catch handler is the one set when the rejection arrives; it gets no params. *)
      let recover error =
        match router.catch_handler with
        | Some global_handler -> handle global_handler { request; event; params = None }
        | None -> fail error
      in
      if Option.is_none router.catch_handler && Option.is_none catch_handler then response
      else
        Host.bind response (function
          | Ok response -> return response
          | Error error -> (
              match catch_handler with
              | None -> recover error
              | Some catch_handler -> (
                  let* caught = handle catch_handler { request; event; params } in
                  match caught with
                  | Ok response -> return response
                  (* An Error from the route's catch handler replaces the original. *)
                  | Error catch_error -> recover (if is_error catch_error then catch_error else error))))
    in
    match thrown (Host.url (Host.request_url request) ~base:(Some (Host.location_href ()))) with
    | Error error -> Error error
    | Ok url when not (String.starts_with ~prefix:"http" (Host.protocol url)) -> Ok None
    | Ok url -> (
        let same_origin = Host.origin url = Host.location_origin () in
        match find_matching_route router { url; same_origin; request } with
        | Error error -> Error error
        | Ok matched ->
            let route = Option.map fst matched in
            let params = Option.join (Option.map snd matched) in
            let handler =
              match route with
              | Some route -> Some route.Route.handler
              | None -> List.assoc_opt (Host.request_method request) router.default_handler_map
            in
            Ok (Option.map (respond route params) handler))

  let set_default_handler ?(method_ = "GET") router handler =
    router.default_handler_map <- (method_, handler) :: List.remove_assoc method_ router.default_handler_map

  let set_catch_handler router handler = router.catch_handler <- Some handler

  (* `registerRoute(route)`: after the routes already registered for its method. *)
  let register_route router (route : Route.t) =
    let routes = Option.value (List.assoc_opt route.method_ router.routes) ~default:[] in
    router.routes <- (route.method_, routes @ [ route ]) :: List.remove_assoc route.method_ router.routes

  let unregister_route router (route : Route.t) =
    match List.assoc_opt route.method_ router.routes with
    | None ->
        let details = Host.obj [ ("method", Host.of_string route.method_) ] in
        Error (workbox_error "unregister-route-but-not-found-with-method" ~details)
    | Some routes when List.memq route routes ->
        let rec without_first = function
          | [] -> []
          | known :: rest -> if known == route then rest else known :: without_first rest
        in
        router.routes <- (route.method_, without_first routes) :: List.remove_assoc route.method_ router.routes;
        Ok ()
    | Some _ -> Error (workbox_error "unregister-route-route-not-registered")
end
