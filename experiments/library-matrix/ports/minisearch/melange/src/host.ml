(* The host for Melange: Melange strings are JavaScript strings and Melange arrays JavaScript
   arrays, so nothing is converted; the string primitives are [@@mel.send] externals, the input
   is read as [Js.Json.t] and the result built with [Js.Json] and [Js.Dict]. *)

module Text = struct
  type t = string

  let of_string (text : string) : t = text
  let length = String.length

  external unit_at : t -> int -> int = "charCodeAt" [@@mel.send]
  external slice : t -> int -> int -> t = "slice" [@@mel.send]

  let append (a : t) (b : t) = a ^ b
  let equal = String.equal

  external lower : t -> t = "toLowerCase" [@@mel.send]
  external split_by : t -> Js.Re.t -> t array = "split" [@@mel.send]

  (* MiniSearch's SPACE_OR_PUNCTUATION, a constant of its module as here. [split] does not use
     or change its [lastIndex] (the [u] flag alone makes it neither global nor sticky). *)
  let space_or_punctuation = Js.Re.fromStringWithFlags "[\\n\\r\\p{Z}\\p{P}]+" ~flags:"u"
  let split text = split_by text space_or_punctuation

  external of_number : float -> t = "String"
end

module Value = struct
  type t = Js.Json.t
  type view = Null | Bool of bool | Number of float | String of Text.t | Array of t array | Object

  let view value =
    match Js.Json.classify value with
    | JSONNull -> Null
    | JSONFalse -> Bool false
    | JSONTrue -> Bool true
    | JSONNumber number -> Number number
    | JSONString text -> String text
    | JSONArray items -> Array items
    | JSONObject _ -> Object

  external get : t -> string -> t Js.undefined = "" [@@mel.get_index]

  let field value name = Js.Undefined.toOption (get value name)

  external keys : t -> string array = "keys" [@@mel.scope "Object"]

  let name value =
    match Js.Json.decodeString value with Some name -> name | None -> invalid_arg "MiniSearch port: a name was expected"
end

module Json = struct
  type t = Js.Json.t

  let null = Js.Json.null
  let bool = Js.Json.boolean
  let int number = Js.Json.number (float_of_int number)
  let number = Js.Json.number
  let text = Js.Json.string
  let array items = Js.Json.array (Array.of_list items)
  let obj fields = Js.Json.object_ (Js.Dict.fromList fields)
end
