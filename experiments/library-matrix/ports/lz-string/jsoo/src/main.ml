open Js_of_ocaml
let get v k = Js.Unsafe.get v k
let run v =
  let raw = get v "data" in
  let data = if Js.Unsafe.equals raw Js.null then None else Some (Js.to_array raw) in
  let repeat = Js.Optdef.get (get v "repeat") (fun () -> 1) in
  match Codec.run (Js.to_string (get v "op")) (Js.to_string (get v "format")) data repeat with
  | None -> Js.null
  | Some a -> Js.some (Js.array a)
let () = Js.export "run" (Js.wrap_callback run)
