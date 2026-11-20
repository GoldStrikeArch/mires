type command = < op : string; a : int array; b : int array; u : bool; v : bool; n : int; r : int; s : string > Js.t
type input = < cases : command array > Js.t
let run (input : input) =
  Array.map (fun (c : command) -> Long_ops.command c##op c##a c##b c##u c##v c##n c##r c##s) input##cases
  |> Array.to_list |> String.concat "\n"
