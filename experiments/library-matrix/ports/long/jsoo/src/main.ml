open Js_of_ocaml
let field v key = Js.Unsafe.get v key
let run input =
  let results = Js.to_array (field input "cases") |> Array.map (fun c ->
    Long_ops.command (Js.to_string (field c "op")) (Js.to_array (field c "a")) (Js.to_array (field c "b"))
      (Js.to_bool (field c "u")) (Js.to_bool (field c "v")) (field c "n") (field c "r") (Js.to_string (field c "s"))) in
  Js.string (String.concat "\n" (Array.to_list results))
let () = Js.export "run" (Js.wrap_callback run)
