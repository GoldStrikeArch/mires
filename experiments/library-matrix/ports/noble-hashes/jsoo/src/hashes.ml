(* libraries/noble-hashes/reference.mjs on OCaml values; the boundary modules read the JS input
   into them. *)

(* What the input supplies: a one-shot message, or chunks fed [repeat] times to one state. *)
type message = Bytes of int array | Chunks of { chunks : int array array; repeat : int }

(* reference.mjs's [hashes] table plus HMAC-SHA-256: the constructor for [algorithm]. *)
let hasher algorithm : Blake2.opts -> Hash.t =
  match algorithm with
  | "sha256" -> fun _ -> Sha2.sha256 ()
  | "sha512" -> fun _ -> Sha2.sha512 ()
  | "sha3_256" -> fun _ -> Sha3.make ~block_len:136 ~suffix:0x06 ~output_len:32
  | "keccak_256" -> fun _ -> Sha3.make ~block_len:136 ~suffix:0x01 ~output_len:32
  | "blake2b" -> Blake2.blake2b
  | "blake2s" -> Blake2.blake2s
  | "hmac_sha256" -> (
      fun { Blake2.key; _ } ->
        match key with
        | Some key -> Hmac.make Sha2.sha256 key
        | None -> invalid_arg "\"key\" expected Uint8Array")
  | _ -> invalid_arg "unknown hash"

let run algorithm opts message =
  let create = hasher algorithm in
  let { Hash.update; digest; _ } = create opts in
  (match message with
   | Bytes bytes -> update bytes
   | Chunks { chunks; repeat } ->
     for _ = 1 to repeat do
       Array.iter update chunks
     done);
  Hash.bytes_to_hex (digest ())
