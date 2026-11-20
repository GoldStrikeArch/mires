type input = {
  algorithm: string,
  bytes?: array<int>,
  chunks?: array<array<int>>,
  repeat?: int,
  key?: array<int>,
  salt?: array<int>,
  personalization?: array<int>,
  dkLen?: float,
}
let invalid = msg => JsError.TypeError.make(msg)->JsError.throw
let run = (input: input) => {
  let chunks = switch input.bytes {
  | Some(one) => [one]
  | None =>
    let parts = input.chunks->Option.getOr([])
    let n = input.repeat->Option.getOr(1)
    n == 1 ? parts : Array.fromInitializer(~length=n * Array.length(parts), i =>
      Array.getUnsafe(parts, mod(i, Array.length(parts)))
    )
  }
  let key = input.key->Option.getOr([])
  let bytes = switch input.algorithm {
  | "sha256" => Sha2.digest(0, chunks, key)
  | "sha512" => Sha2.digest(1, chunks, key)
  | "hmac_sha256" =>
    if input.key->Option.isNone {invalid("missing key")}
    Sha2.digest(2, chunks, key)
  | "sha3_256" | "keccak_256" => Sha3.digest(input.algorithm == "keccak_256", chunks)
  | "blake2b" | "blake2s" =>
    let wide = input.algorithm == "blake2b"
    let limit = wide ? 64 : 32
    let size = input.dkLen->Option.getOr(Int.toFloat(limit))
    let dk = Float.toInt(size)
    if dk < 1 || dk > limit || Int.toFloat(dk) != size {invalid("invalid digest size")}
    if input.key->Option.isSome && (Array.length(key) == 0 || Array.length(key) > limit) {invalid("invalid key")}
    let fixed = v => switch v {
    | None => []
    | Some(a) => if Array.length(a) != limit / 4 {invalid("invalid parameter")}; a
    }
    Blake2.digest(wide, chunks, dk, key, fixed(input.salt), fixed(input.personalization))
  | _ => invalid("unknown hash")
  }
  let digits = "0123456789abcdef"
  bytes->Array.map(b => digits->String.charAt(Int.shiftRightUnsigned(b,4)) ++ digits->String.charAt(Int.bitwiseAnd(b,15)))->Array.join("")
}
