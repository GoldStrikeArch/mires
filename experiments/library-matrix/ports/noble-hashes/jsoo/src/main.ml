(* The js_of_ocaml boundary: the corpus input is a JS object read through a class type. Its JS
   arrays are copied into OCaml int arrays ([Js.to_array]) and the hex digest is returned as a JS
   string. [run] is published with [Js.export]. *)

open Js_of_ocaml

type bytes = int Js.js_array Js.t

class type input = object
  method algorithm : Js.js_string Js.t Js.readonly_prop
  method bytes : bytes Js.optdef Js.readonly_prop
  method chunks : bytes Js.js_array Js.t Js.optdef Js.readonly_prop
  method repeat : int Js.optdef Js.readonly_prop
  method dkLen : Js.number Js.t Js.optdef Js.readonly_prop
  method key : bytes Js.optdef Js.readonly_prop
  method salt : bytes Js.optdef Js.readonly_prop
  method personalization : bytes Js.optdef Js.readonly_prop
end

let run (input : input Js.t) =
  let bytes field = Option.map Js.to_array (Js.Optdef.to_option field) in
  let opts : Blake2.opts =
    {
      dk_len = Option.map Js.to_float (Js.Optdef.to_option input##.dkLen);
      key = bytes input##.key;
      salt = bytes input##.salt;
      personalization = bytes input##.personalization;
    }
  in
  let message : Hashes.message =
    match bytes input##.bytes with
    | Some bytes -> Bytes bytes
    | None ->
      let chunks = Js.to_array (Js.Optdef.get input##.chunks (fun () -> invalid_arg "expected bytes or chunks")) in
      Chunks
        { chunks = Array.map Js.to_array chunks; repeat = Js.Optdef.get input##.repeat (fun () -> 1) }
  in
  Js.string (Hashes.run (Js.to_string input##.algorithm) opts message)

let () = Js.export "run" (Js.wrap_callback run)
