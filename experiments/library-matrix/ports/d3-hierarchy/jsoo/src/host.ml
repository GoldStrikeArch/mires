(* The js_of_ocaml side of the boundary. The input is read in place through class types; strings
   cross as OCaml UTF-8 byte strings (Js.to_string and Js.string), numbers are floats. d3 reads
   every field of a datum with [!= null] or [??], so null and undefined both mean absent. The
   result is a JS array of rows built from [value]s. *)

open Js_of_ocaml

class type datum_js = object
  method id : Js.js_string Js.t Js.Opt.t Js.Optdef.t Js.readonly_prop
  method parentId : Js.js_string Js.t Js.Opt.t Js.Optdef.t Js.readonly_prop
  method value : Js.number Js.t Js.Opt.t Js.Optdef.t Js.readonly_prop
  method children : datum_js Js.t Js.js_array Js.t Js.Opt.t Js.Optdef.t Js.readonly_prop
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

(* A field that is neither undefined nor null. *)
let present field = Js.Optdef.case field (fun () -> None) Js.Opt.to_option

(* An absent string field reads as "", which names nothing in stratify. *)
let text field = match present field with Some s -> Js.to_string s | None -> ""

let id (d : datum) = text d##.id
let parent_id (d : datum) = text d##.parentId

(* The datum's [value ?? 0]. *)
let value (d : datum) = match present d##.value with Some x -> Js.float_of_number x | None -> 0.

let children (d : datum) = match present d##.children with Some a -> Js.to_array a | None -> [||]
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

(* The datum's id as the input has it, a string, null or undefined: the reference's [n.data.id]. *)
let id_value (d : datum) : value = Js.Unsafe.inject d##.id

let string s : value = Js.Unsafe.inject (Js.string s)
let number x : value = Js.Unsafe.inject (Js.number_of_float x)
let null : value = Js.Unsafe.inject Js.null
let array (values : value array) : value = Js.Unsafe.inject (Js.array values)
