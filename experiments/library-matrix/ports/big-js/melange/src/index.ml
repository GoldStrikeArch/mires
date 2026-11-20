(* The boundary (Melange): what libraries/big-js/reference.mjs does. dp, rm and repeat are
   nullable: undefined and null select the defaults, as the adapter's ?? does. n and mode are
   optional (undefined) and reach the methods as upstream's optional arguments. *)

type operand

external js_string : operand -> string = "String"

type operation =
  < op : string
  ; x : operand
  ; y : operand
  ; n : float Js.undefined
  ; mode : float Js.undefined
  ; dp : float Js.nullable
  ; rm : float Js.nullable >
  Js.t

type input = < cases : operation array ; repeat : int Js.nullable > Js.t

(* The Big constructor's conversion: a number goes through String(), except that -0 becomes
   "-0". *)
let text (v : operand) =
  match Js.Types.classify v with
  | JSString s -> s
  | JSNumber n when n = 0. && 1. /. n < 0. -> "-0"
  | _ -> js_string v

let option_or v default = Option.value (Js.Nullable.toOption v) ~default

let run (input : input) =
  let outputs = ref [] in
  for _ = 1 to option_or input##repeat 1 do
    input##cases
    |> Array.iter (fun c ->
           let result =
             try
               Big.operate ~op:c##op ~x:(text c##x) ~y:(text c##y)
                 ~n:(Js.Undefined.toOption c##n) ~mode:(Js.Undefined.toOption c##mode)
                 ~dp:(option_or c##dp 20.) ~rm:(option_or c##rm 1.)
             with Big.Big_error message -> message
           in
           outputs := result :: !outputs)
  done;
  String.concat "\n" (List.rev !outputs)
