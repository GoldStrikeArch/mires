let run name chunks key salt personal length =
  let or_empty = function None -> [||] | Some a -> a in
  let key_bytes = or_empty key in
  let digest = match name with
    | "sha256" -> Sha2.digest 0 chunks key_bytes
    | "sha512" -> Sha2.digest 1 chunks key_bytes
    | "hmac_sha256" ->
      if key = None then invalid_arg "missing key";
      Sha2.digest 2 chunks key_bytes
    | "sha3_256" | "keccak_256" -> Sha3.digest (name = "keccak_256") chunks
    | "blake2b" | "blake2s" ->
      let wide = name = "blake2b" in
      let limit = if wide then 64 else 32 in
      let size = match length with None -> float_of_int limit | Some x -> x in
      let dk = int_of_float size in
      if size <> float_of_int dk || dk < 1 || dk > limit then invalid_arg "invalid digest size";
      (match key with Some a when Array.length a = 0 || Array.length a > limit -> invalid_arg "invalid key" | _ -> ());
      let check_fixed = function Some a when Array.length a <> limit / 4 -> invalid_arg "invalid parameter" | _ -> () in
      check_fixed salt; check_fixed personal;
      Blake2.digest wide chunks dk key_bytes (or_empty salt) (or_empty personal)
    | _ -> invalid_arg "unknown hash"
  in
  let digits = "0123456789abcdef" in
  String.init (Array.length digest * 2) (fun i ->
    let b = digest.(i / 2) in digits.[if i mod 2 = 0 then b lsr 4 else b land 15])

let repeat n blocks =
  if n = 1 then blocks else Array.init (n * Array.length blocks) (fun i -> blocks.(i mod Array.length blocks))
