type command = {op: string, s: string, n: int, r: int, a: array<int>, b: array<int>, u: bool, v: bool}
type input = {cases: array<command>}
module B = BigInt
let nat = B.fromInt
let zero = nat(0)
let one = nat(1)
let unsigned = a => B.asUintN(~width=64,a)
let signed = a => B.asIntN(~width=64,a)
let asValue = (a,u) => if u { unsigned(a) } else { signed(a) }
let words = p => B.bitwiseOr(B.shiftLeft(B.asUintN(~width=32,nat(Array.getUnsafe(p,1))),nat(32)),B.asUintN(~width=32,nat(Array.getUnsafe(p,0))))
external band: (int,int) => int = "%andint"
let radix = r => if r == 0 {10} else if r >= 2 && r <= 36 {r} else {raise(Invalid_argument("radix"))}
let format = (a,u,r) => B.toString(asValue(a,u),~radix=radix(r))
let rec parse = (s,base) => {
  if s == "" {raise(Invalid_argument("empty string"))}
  if s == "NaN" || s == "Infinity" || s == "+Infinity" || s == "-Infinity" { zero } else {
    let r = radix(base)
    if String.startsWith(s,"-") { B.sub(zero,parse(String.slice(s,~start=1),r)) } else {
      if String.includes(s,"-") {raise(Invalid_argument("hyphen"))}
      let result = ref(zero)
      let start = ref(0)
      let b = nat(r)
      while start.contents < String.length(s) {
        let stop = min(start.contents+8,String.length(s))
        let i = ref(start.contents)
        let code = j => String.charCodeAtUnsafe(s,j)
        while i.contents < stop && (code(i.contents) == 32 || (code(i.contents) >= 9 && code(i.contents) <= 13)) {
          i := i.contents+1
        }
        if i.contents < stop && code(i.contents) == 43 {i := i.contents+1}
        if r == 16 && i.contents+1 < stop && code(i.contents) == 48 && (code(i.contents+1) == 120 || code(i.contents+1) == 88) {i := i.contents+2}
        let part = ref(zero)
        while i.contents < stop {
          let c = code(i.contents)
          let d = if c >= 48 && c <= 57 {c-48} else if c >= 65 && c <= 90 {c-55} else if c >= 97 && c <= 122 {c-87} else {36}
          if d >= r {i := stop} else {
            part := B.add(B.mul(part.contents,b),nat(d))
            i := i.contents+1
          }
        }
        for _j in start.contents to stop-1 {result := B.mul(result.contents,b)}
        result := unsigned(B.add(result.contents,part.contents))
        start := stop
      }
      result.contents
    }
  }
}
let divide = (a,b,u) => {
  if b == zero {raise(Invalid_argument("division by zero"))}
  if !u && B.shiftRight(a,nat(32)) == nat(-2147483648) && signed(b) == nat(-1) {a}
  else {B.div(asValue(a,u),asValue(b,u))}
}
let rotl = (a,n) => {
  let k = band(n,63)
  unsigned(B.bitwiseOr(B.shiftLeft(a,nat(k)),B.shiftRight(unsigned(a),nat(64-k))))
}
let compare = (a,b,u,v) => {
  if a == b && (u == v || signed(a) >= zero) {0} else {
    let an = !u && signed(a) < zero
    let bn = !v && signed(b) < zero
    let less = if an != bn {an} else if u {a < b} else {signed(B.sub(a,b)) < zero}
    if less {-1} else {1}
  }
}
let countZero = (a,leading) => {
  let bits = ref(a)
  let count = ref(0)
  while count.contents < 64 && (if leading {signed(bits.contents) >= zero} else {B.bitwiseAnd(bits.contents,one) == zero}) {
    bits := if leading {unsigned(B.shiftLeft(bits.contents,one))} else {B.shiftRight(bits.contents,one)}
    count := count.contents+1
  }
  count.contents
}
let evaluate = c => {
  let a = words(c.a)
  let b = words(c.b)
  let n = c.n
  switch c.op {
  | "clz" => Int.toString(countZero(a,true))
  | "ctz" => Int.toString(countZero(a,false))
  | "compare" => Int.toString(compare(a,b,c.u,c.v))
  | "mix" =>
    let acc = ref(a)
    let check = ref(0)
    let k = B.bitwiseOr(b,one)
    for i in 0 to n-1 {
      acc := B.bitwiseXor(rotl(B.add(B.mul(acc.contents,k),nat(i+1)),11),b)
      acc := unsigned(B.add(acc.contents,divide(acc.contents,nat(3+band(i,255)),c.u)))
      if band(i,7) == 0 {
        let s = format(acc.contents,c.u,c.r)
        check := check.contents*33+String.length(s)+String.charCodeAtUnsafe(s,0)
      }
    }
    format(acc.contents,c.u,c.r) ++ ":" ++ Int.toString(check.contents)
  | _ =>
    let shift = nat(band(n,63))
    let value = switch c.op {
    | "fromBits" | "toString" => a
    | "fromString" => parse(c.s,n)
    | "add" => B.add(a,b)
    | "subtract" => B.sub(a,b)
    | "multiply" => B.mul(a,b)
    | "divide" => divide(signed(a),b,c.u)
    | "modulo" => if b == zero {raise(Invalid_argument("division by zero"))} else {B.mod(asValue(a,c.u),asValue(b,c.u))}
    | "and" => B.bitwiseAnd(a,b) | "or" => B.bitwiseOr(a,b) | "xor" => B.bitwiseXor(a,b)
    | "shiftLeft" => B.shiftLeft(a,shift)
    | "shiftRight" => B.shiftRight(signed(a),shift)
    | "shiftRightUnsigned" => B.shiftRight(a,shift)
    | "rotl" => rotl(a,n)
    | _ => raise(Invalid_argument("operation"))
    }
    format(value,c.u,c.r)
  }
}
let run = (input:input) => input.cases->Array.map(c => {
  try {evaluate(c)} catch {| Invalid_argument(_) => "!"}
})->Array.join("\n")
