// noble's _u64.ts: a 64-bit word is a high and a low 32-bit half, each held in an int.
//
// Shifts and rotations return one half, as in noble. Sums return both halves as a tuple; the
// carry out of the low halves comes from an unsigned comparison of the wrapped sum with an addend
// (flipping the sign bit turns signed order into unsigned order), where noble divides its
// unmasked JS-number sum by 2^32. The 3- to 5-term sums are noble's add3L/add3H ...
// add5L/add5H, each computed in one function.

// High half of a 64-bit logical right shift by s in 0..31.
let shrSH = (h, _l, s) => h >>> s

// Low half of a 64-bit logical right shift by s in 1..31.
let shrSL = (h, l, s) => h << 32 - s ||| l >>> s

// High half of a 64-bit right rotation by s in 1..31.
let rotrSH = (h, l, s) => h >>> s ||| l << 32 - s

// Low half of a 64-bit right rotation by s in 1..31.
let rotrSL = (h, l, s) => h << 32 - s ||| l >>> s

// High half of a 64-bit right rotation by s in 33..63.
let rotrBH = (h, l, s) => h << 64 - s ||| l >>> s - 32

// Low half of a 64-bit right rotation by s in 33..63.
let rotrBL = (h, l, s) => h >>> s - 32 ||| l << 64 - s

// High half of a 64-bit right rotation by 32: the low half.
let rotr32H = (_h, l) => l

// Low half of a 64-bit right rotation by 32: the high half.
let rotr32L = (h, _l) => h

// noble's fromNumH: the high half of an integer in 0 .. 2^53.
let fromNumH = n => Float.toInt(n /. 4294967296.)

// noble's fromNumL (`n >>> 0`): the low half of an integer in 0 .. 2^53, as a signed int.
let fromNumL = n => {
  let low = Float.mod(n, 4294967296.)
  Float.toInt(low >= 2147483648. ? low -. 4294967296. : low)
}

// 1 when adding `addend` wrapped the 32-bit `sum`, else 0.
let carryOut = (sum, addend) => (sum ^^^ -2147483648) < (addend ^^^ -2147483648) ? 1 : 0

// The sum of two 64-bit words, as (high, low).
let add = (ah, al, bh, bl) => {
  let l = al + bl
  (ah + bh + carryOut(l, bl), l)
}

// The sum of three 64-bit words, as (high, low).
let add3 = (ah, al, bh, bl, ch, cl) => {
  let ab = al + bl
  let l = ab + cl
  (ah + bh + ch + carryOut(ab, bl) + carryOut(l, cl), l)
}

// The sum of four 64-bit words, as (high, low).
let add4 = (ah, al, bh, bl, ch, cl, dh, dl) => {
  let ab = al + bl
  let abc = ab + cl
  let l = abc + dl
  (ah + bh + ch + dh + carryOut(ab, bl) + carryOut(abc, cl) + carryOut(l, dl), l)
}

// The sum of five 64-bit words, as (high, low).
let add5 = (ah, al, bh, bl, ch, cl, dh, dl, eh, el) => {
  let ab = al + bl
  let abc = ab + cl
  let abcd = abc + dl
  let l = abcd + el
  let carries = carryOut(ab, bl) + carryOut(abc, cl) + carryOut(abcd, dl) + carryOut(l, el)
  (ah + bh + ch + dh + eh + carries, l)
}
