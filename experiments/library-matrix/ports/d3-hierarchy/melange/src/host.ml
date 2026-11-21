(* The Melange side of the boundary. The input is read in place through [Js.t] object types;
   strings are JS strings and numbers are floats, with no conversion. The result is a JS array of
   rows built from [value]s. *)

(* A nested datum or a stratify row. *)
type datum =
  < id : string Js.undefined
  ; parentId : string Js.undefined
  ; value : float Js.undefined
  ; children : datum array Js.undefined >
  Js.t

type input =
  < op : string
  ; source : string
  ; data : datum
  ; rows : datum array
  ; width : float
  ; height : float
  ; padding : float
  ; sort : bool
  ; round : bool
  ; nodeSize : bool
  ; tile : string >
  Js.t

(* An undefined string field reads as "", which names nothing in stratify. *)
let text field = Option.value (Js.Undefined.toOption field) ~default:""

let id (d : datum) = text d##id
let parent_id (d : datum) = text d##parentId

(* The datum's [value ?? 0]. *)
let value (d : datum) = Option.value (Js.Undefined.toOption d##value) ~default:0.

let children (d : datum) = Option.value (Js.Undefined.toOption d##children) ~default:[||]
let op (input : input) = input##op
let source (input : input) = input##source
let data (input : input) = input##data
let rows (input : input) = input##rows
let width (input : input) = input##width
let height (input : input) = input##height
let padding (input : input) = input##padding
let sort (input : input) = input##sort
let round (input : input) = input##round
let node_size (input : input) = input##nodeSize
let tile (input : input) = input##tile

(* Math.round, which rounds halves up, keeps -0 and returns -0 for [-0.5, 0). *)
let math_round = Js.Math.round

type value

external box : 'a -> value = "%identity"

let string (s : string) = box s
let number (x : float) = box x
let null = box Js.null
let array (values : value array) = box values
