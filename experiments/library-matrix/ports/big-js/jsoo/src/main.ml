(* The boundary (js_of_ocaml): what libraries/big-js/reference.mjs does. dp, rm and repeat may be
   undefined or null, which select the defaults, as the adapter's ?? does. n and mode are optional
   (undefined) and reach the methods as upstream's optional arguments. Strings cross with
   Js.to_string (UTF-16 to UTF-8); a non-ASCII operand stays non-ASCII and is invalid, as in
   big.js. *)

open Js_of_ocaml

class type operation = object
  method op : Js.js_string Js.t Js.readonly_prop
  method x : Js.Unsafe.top Js.t Js.readonly_prop
  method y : Js.Unsafe.top Js.t Js.readonly_prop
  method n : Js.number Js.t Js.Optdef.t Js.readonly_prop
  method mode : Js.number Js.t Js.Optdef.t Js.readonly_prop
  method dp : Js.number Js.t Js.Opt.t Js.Optdef.t Js.readonly_prop
  method rm : Js.number Js.t Js.Opt.t Js.Optdef.t Js.readonly_prop
end

class type input = object
  method cases : operation Js.t Js.js_array Js.t Js.readonly_prop
  method repeat : Js.number Js.t Js.Opt.t Js.Optdef.t Js.readonly_prop
end

let optional v = Option.map Js.float_of_number (Js.Optdef.to_option v)
let option_or v default =
  match Option.bind (Js.Optdef.to_option v) Js.Opt.to_option with
  | Some n -> Js.float_of_number n
  | None -> default

(* The Big constructor's conversion: a number goes through String(), except that -0 becomes
   "-0". *)
let text (v : Js.Unsafe.top Js.t) =
  match Js.to_string (Js.typeof v) with
  | "string" -> Js.to_string (Js.Unsafe.coerce v)
  | "number" when let n = Js.float_of_number (Js.Unsafe.coerce v) in n = 0. && 1. /. n < 0. -> "-0"
  | _ -> Js.to_string (Js.Unsafe.fun_call Js.Unsafe.global##._String [| Js.Unsafe.inject v |])

let run (input : input Js.t) =
  let outputs = ref [] in
  for _ = 1 to int_of_float (option_or input##.repeat 1.) do
    Js.to_array input##.cases
    |> Array.iter (fun c ->
           let result =
             try
               Big.operate ~op:(Js.to_string c##.op) ~x:(text c##.x) ~y:(text c##.y)
                 ~n:(optional c##.n) ~mode:(optional c##.mode) ~dp:(option_or c##.dp 20.)
                 ~rm:(option_or c##.rm 1.)
             with Big.Big_error message -> message
           in
           outputs := result :: !outputs)
  done;
  Js.string (String.concat "\n" (List.rev !outputs))

let () = Js.export "run" (Js.wrap_callback run)
