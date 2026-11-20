(* The Melange boundary: the corpus input is a JS object read in place with [##]. Its arrays of
   numbers are OCaml int arrays and its numbers OCaml ints or floats, so nothing is converted. *)

type input =
  < algorithm : string
  ; bytes : int array Js.undefined
  ; chunks : int array array Js.undefined
  ; repeat : int Js.undefined
  ; dkLen : float Js.undefined
  ; key : int array Js.undefined
  ; salt : int array Js.undefined
  ; personalization : int array Js.undefined >
  Js.t

let run (input : input) =
  let opt = Js.Undefined.toOption in
  let opts : Blake2.opts =
    {
      dk_len = opt input##dkLen;
      key = opt input##key;
      salt = opt input##salt;
      personalization = opt input##personalization;
    }
  in
  let message : Hashes.message =
    match opt input##bytes with
    | Some bytes -> Bytes bytes
    | None ->
      let chunks = Option.get (opt input##chunks) in
      Chunks { chunks; repeat = Option.value (opt input##repeat) ~default:1 }
  in
  Hashes.run input##algorithm opts message
