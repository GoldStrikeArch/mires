type input
external get : input -> string -> 'a = "" [@@mel.get_index]
let optional x k = Js.Nullable.toOption (get x k)
let run x =
  let chunks = match optional x "bytes" with
    | Some b -> [| b |]
    | None -> Dispatch.repeat (Option.value ~default:1 (optional x "repeat")) (get x "chunks")
  in
  Dispatch.run (get x "algorithm") chunks (optional x "key") (optional x "salt")
    (optional x "personalization") (optional x "dkLen")
