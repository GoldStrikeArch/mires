// noble's Hash: an incremental hash with its block and digest lengths. Bytes are arrays of
// ints in 0..255.
type t = {
  blockLen: int,
  outputLen: int,
  update: array<int> => unit,
  digest: unit => array<int>,
}
