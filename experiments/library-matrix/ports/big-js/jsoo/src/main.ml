open Js_of_ocaml
let get v k = Js.Unsafe.get v k
let integer v k fallback = match Js.Optdef.to_option (get v k) with
  | None -> fallback
  | Some raw -> let n=Js.float_of_number raw in let i=int_of_float n in if float_of_int i=n && i <> -2000002 then i else -2000001
let run x =
  let lines=ref [] in
  for _i=1 to integer x "repeat" 1 do
    Array.iter (fun c ->
      let rm=integer c "rm" 1 in
      let result=try Decimal.operate (Js.to_string (get c "op")) (Float_host.text (get c "x")) (Float_host.text (get c "y"))
        (integer c "n" (-2000002)) (integer c "dp" 20) rm (integer c "mode" rm)
        with Failure msg -> msg in
      lines:=result :: !lines) (Js.to_array (get x "cases"))
  done;
  Js.string (String.concat "\n" (List.rev !lines))
let ()=Js.export "run" (Js.wrap_callback run)
