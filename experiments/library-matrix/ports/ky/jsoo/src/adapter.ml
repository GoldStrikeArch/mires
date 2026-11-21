(* What libraries/ky/reference.mjs does, on the port's Ky. Each request case gets its own abort
   controller, transport and hooks, which record the trace.

   The boundary: each corpus case is read in place, field by field. The option values Ky
   validates stay the JS values they are until Ky's own checks read them, and each result is the
   JS array [trace, outcome]. *)

let cancel controller =
  let reason = Host.new_error "cancelled" in
  Host.set_name reason "AbortError";
  Host.abort_with controller reason

let nullable = function Some text -> Host.of_string text | None -> Host.null
let text record key = Option.value (Option.bind (Host.field record key) Host.to_string) ~default:""

let request_case case =
  let trace = ref [] in
  let record entry = trace := Host.array entry :: !trace in
  let controller = Host.abort_controller () in
  let abort_at point = if text case "abort" = point then cancel controller in
  abort_at "before";
  let replies = Array.of_list (Option.value (Host.to_list (Host.get case "replies")) ~default:[]) in
  let sent = ref 0 in
  (* The injected transport: it records what it was sent and answers with the next reply. *)
  let fetch request =
    Host.bind (Host.request_text request) (fun body ->
        let headers = Host.request_headers request in
        record
          [
            Host.of_string "fetch";
            Host.of_string (Host.request_method request);
            Host.of_string (Host.request_url request);
            nullable (Host.header headers "accept");
            nullable (Host.header headers "content-type");
            nullable (Host.header headers "x-matrix");
            Host.of_string body;
          ];
        abort_at "fetch";
        let signal = Host.request_signal request in
        match Host.throw_if_aborted signal with
        | Error reason -> Host.reject reason
        | Ok () -> (
            let reply = replies.(min !sent (Array.length replies - 1)) in
            incr sent;
            match text reply "kind" with
            | "network" -> Host.reject (Host.type_error "Failed to fetch")
            | "stall" -> Host.promise (fun _ reject -> Host.on_abort signal (fun () -> reject (Host.reason signal)))
            | _ ->
                Host.bind
                  (Host.response (Host.get reply "body") ~status:(Host.get reply "status")
                     ~headers:(Host.get reply "headers"))
                  (function Ok response -> Host.resolve response | Error error -> Host.reject error)))
  in
  let delay attempt =
    let milliseconds = 300. *. Host.math_pow 2. (float_of_int (attempt - 1)) in
    record [ Host.of_string "backoff"; Host.of_int attempt; Host.of_float milliseconds ];
    milliseconds
  in
  (* A valid name and value: setting the header cannot fail, and it is set before set_header
     answers, so the hook's answer can be dropped. *)
  let mark request label =
    ignore (Host.set_header (Host.request_headers request) "x-matrix" (Host.of_string label))
  in
  let hooks : Ky.hooks =
    {
      init = [ (fun () -> record [ Host.of_string "init" ]) ];
      before_request =
        [
          (fun ~request ~retry_count ->
            record [ Host.of_string "beforeRequest"; Host.of_int retry_count ];
            mark request "initial");
        ];
      after_response =
        [
          (fun ~request:_ ~response ~retry_count ->
            record [ Host.of_string "afterResponse"; Host.of_int retry_count; Host.of_int (Host.status response) ]);
        ];
      before_retry =
        [
          (fun ~request ~error ~retry_count ->
            record [ Host.of_string "beforeRetry"; Host.of_int retry_count; Host.of_string (Ky.error_name error) ];
            mark request ("retry-" ^ string_of_int retry_count);
            abort_at "retry");
        ];
      before_error =
        [
          (fun ~request:_ ~error ~retry_count ->
            record [ Host.of_string "beforeError"; Host.of_int retry_count; Host.of_string (Ky.error_name error) ]);
        ];
    }
  in
  let option key = Host.field case key in
  let options : Ky.options =
    {
      method_ = Option.bind (option "method") Host.to_string;
      headers = option "headers";
      json = option "json";
      retry =
        {
          limit = option "limit";
          methods = option "methods";
          status_codes = option "statusCodes";
          max_retry_after = option "maxRetryAfter";
          backoff_limit = option "backoffLimit";
          retry_on_timeout = option "retryOnTimeout";
          delay;
        };
      timeout = option "timeout";
      throw_http_errors = option "throwHttpErrors";
      hooks;
      fetch;
      signal = Some (Host.controller_signal controller);
    }
  in
  let shortcut = if text case "read" = "json" then Ky.Json else Ky.Text in
  Host.bind (Ky.request (text case "url") options shortcut) (fun result ->
      let outcome =
        match result with
        | Ok value -> [ Host.of_string "ok"; value ]
        | Error (Ky.Http_error { response; data; _ }) ->
            [
              Host.of_string "error";
              Host.of_string "HTTPError";
              Host.of_int (Host.status response);
              Option.value data ~default:Host.null;
            ]
        | Error error -> [ Host.of_string "error"; Host.of_string (Ky.error_name error); Host.null; Host.null ]
      in
      Host.resolve (Host.array [ Host.array (List.rev !trace); Host.array outcome ]))

let run input =
  let rec each results = function
    | [] -> Host.resolve (Host.array (List.rev results))
    | case :: rest -> Host.bind (request_case case) (fun result -> each (result :: results) rest)
  in
  Host.to_value (each [] (Option.value (Host.to_list (Host.get input "cases")) ~default:[]))
