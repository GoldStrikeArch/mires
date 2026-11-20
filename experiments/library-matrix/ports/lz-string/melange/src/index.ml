type input
external get : input -> string -> 'a = "" [@@mel.get_index]
let run input =
  let data = Js.Nullable.toOption (get input "data") in
  let repeat = Option.value ~default:1 (Js.Nullable.toOption (get input "repeat")) in
  match Codec.run (get input "op") (get input "format") data repeat with
  | None -> Js.null
  | Some output -> Js.Null.return output
