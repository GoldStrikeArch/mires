(* The js_of_ocaml side of the boundary. The input is read in place through class types; strings
   cross as OCaml UTF-8 byte strings (Js.to_string and Js.string), numbers are floats. The result
   is a JS array of rows built from [value]s. *)

open Js_of_ocaml

class type datum_js = object
  method id : Js.js_string Js.t Js.Optdef.t Js.readonly_prop
  method parentId : Js.js_string Js.t Js.Optdef.t Js.readonly_prop
  method value : Js.number Js.t Js.Optdef.t Js.readonly_prop
  method children : datum_js Js.t Js.js_array Js.t Js.Optdef.t Js.readonly_prop
end

class type input_js = object
  method op : Js.js_string Js.t Js.readonly_prop
  method source : Js.js_string Js.t Js.readonly_prop
  method data : datum_js Js.t Js.readonly_prop
  method rows : datum_js Js.t Js.js_array Js.t Js.readonly_prop
  method width : Js.number Js.t Js.readonly_prop
  method height : Js.number Js.t Js.readonly_prop
  method padding : Js.number Js.t Js.readonly_prop
  method sort : bool Js.t Js.readonly_prop
  method round : bool Js.t Js.readonly_prop
  method nodeSize : bool Js.t Js.readonly_prop
  method tile : Js.js_string Js.t Js.readonly_prop
end

(* A nested datum or a stratify row. *)
type datum = datum_js Js.t

type input = input_js Js.t

(* An undefined string field reads as "", which names nothing in stratify. *)
let text field = Js.Optdef.case field (fun () -> "") Js.to_string

let id (d : datum) = text d##.id
let parent_id (d : datum) = text d##.parentId

(* The datum's [value ?? 0]. *)
let value (d : datum) = Js.Optdef.case d##.value (fun () -> 0.) Js.float_of_number

let children (d : datum) = Js.Optdef.case d##.children (fun () -> [||]) Js.to_array
let op (input : input) = Js.to_string input##.op
let source (input : input) = Js.to_string input##.source
let data (input : input) = input##.data
let rows (input : input) = Js.to_array input##.rows
let width (input : input) = Js.float_of_number input##.width
let height (input : input) = Js.float_of_number input##.height
let padding (input : input) = Js.float_of_number input##.padding
let sort (input : input) = Js.to_bool input##.sort
let round (input : input) = Js.to_bool input##.round
let node_size (input : input) = Js.to_bool input##.nodeSize
let tile (input : input) = Js.to_string input##.tile

(* Math.round, which rounds halves up, keeps -0 and returns -0 for [-0.5, 0). *)
let math_round x = Js.float_of_number (Js.math##round (Js.number_of_float x))

type value = Js.Unsafe.any

let string s : value = Js.Unsafe.inject (Js.string s)
let number x : value = Js.Unsafe.inject (Js.number_of_float x)
let null : value = Js.Unsafe.inject Js.null
let array (values : value array) : value = Js.Unsafe.inject (Js.array values)
