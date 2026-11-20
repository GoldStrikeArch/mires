// The boundary: libraries/noble-hashes/reference.mjs in ReScript. The input record is the corpus
// object itself, so reading it converts nothing; the byte arrays are read in place.
type input = {
  algorithm: string,
  bytes?: array<int>,
  chunks?: array<array<int>>,
  repeat?: int,
  dkLen?: float,
  key?: array<int>,
  salt?: array<int>,
  personalization?: array<int>,
}

// reference.mjs's `hashes` table plus HMAC-SHA-256: the constructor for `algorithm`.
let hasher = (algorithm): (Blake2.opts => Hash.t) =>
  switch algorithm {
  | "sha256" => _ => Sha2.sha256()
  | "sha512" => _ => Sha2.sha512()
  | "sha3_256" => _ => Sha3.make(~blockLen=136, ~suffix=0x06, ~outputLen=32)
  | "keccak_256" => _ => Sha3.make(~blockLen=136, ~suffix=0x01, ~outputLen=32)
  | "blake2b" => Blake2.blake2b
  | "blake2s" => Blake2.blake2s
  | "hmac_sha256" =>
    opts =>
      switch opts.key {
      | Some(key) => Hmac.make(Sha2.sha256, key)
      | None => JsError.TypeError.throwWithMessage(`"key" expected Uint8Array`)
      }
  | _ => JsError.TypeError.throwWithMessage("unknown hash")
  }

let run = (input: input) => {
  let create = hasher(input.algorithm)
  let opts: Blake2.opts = {
    dkLen: input.dkLen,
    key: input.key,
    salt: input.salt,
    personalization: input.personalization,
  }
  let hash = create(opts)
  switch input.bytes {
  | Some(message) => hash.update(message)
  | None =>
    let blocks = input.chunks->Option.getOrThrow(~message="expected bytes or chunks")
    for _ in 1 to input.repeat->Option.getOr(1) {
      blocks->Array.forEach(block => hash.update(block))
    }
  }
  Utils.bytesToHex(hash.digest())
}
