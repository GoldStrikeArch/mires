open Js_of_ocaml
let get x k = Js.Unsafe.get x k
let optional x k = Js.Optdef.to_option (get x k)
let byte_array x k = Option.map Js.to_array (optional x k)
let run x =
  let chunks = match byte_array x "bytes" with
    | Some b -> [| b |]
    | None -> Dispatch.repeat (Option.value ~default:1 (optional x "repeat"))
        (Array.map Js.to_array (Js.to_array (get x "chunks")))
  in
  Js.string (Dispatch.run (Js.to_string (get x "algorithm")) chunks (byte_array x "key")
    (byte_array x "salt") (byte_array x "personalization") (optional x "dkLen"))
let () = Js.export "run" (Js.wrap_callback run)
