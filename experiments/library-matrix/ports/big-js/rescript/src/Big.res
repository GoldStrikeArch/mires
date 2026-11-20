// big.js 7.0.1 (big.mjs): the decimal type, its parser, `round`, `stringify` and the methods the
// contract uses, function by function and loop by loop. A coefficient is a JS array of digits,
// most significant first, mutated where upstream mutates its array (push, pop, shift, unshift,
// length). The exponent is a float because upstream's is a JS number: past int32 and 2^53 it
// rounds as a double does, and Infinity and NaN exponents keep upstream's comparison chains.
// ReScript has no `break` or early `return`, so upstream's loops that leave early run on flags.

exception BigError(string)

let name = "[big.js] "
let invalid = name ++ "Invalid "
let invalidDp = invalid ++ "decimal places"
let invalidRm = invalid ++ "rounding mode"
let divByZero = name ++ "Division by zero"
let maxDp = 1e6
let maxPower = 1e6
let ne = -7.
let pe = 21.
let numeric = %re("/^-?(\d+(\.\d*)?|\.\d+)(e[+-]?\d+)?$/i")

type t = {mutable s: int, mutable e: float, mutable c: array<int>}

// JS `Number(s)`: upstream's `+s`, and Math.sqrt's own coercion.
@val external number: string => float = "Number"
@set external setLength: (array<int>, int) => unit = "length"

let copy = x => {...x, c: x.c->Array.copy}

// Upstream's `n !== ~~n || n < lo || n > hi`, negated: an integer in the range (all lie in int32).
let isIntIn = (n, lo, hi) => n >= lo && n <= hi && n == Math.floor(n)

// JS truthiness of a number, for upstream's `if (a = xe - ye)`.
let truthy = n => n != 0. && !Float.isNaN(n)

// ECMAScript ToInt32, for upstream's `| 0` and `&` on exponents.
let toInt32 = v =>
  if !Float.isFinite(v) {
    0
  } else {
    let m = Float.mod(Math.trunc(v), 4294967296.)
    Float.toInt(m >= 2147483648. ? m -. 4294967296. : m < -2147483648. ? m +. 4294967296. : m)
  }

// `Array.prototype.slice(0, end)`, where a negative end counts from the back.
let head = (c, end) => {
  let length = Int.toFloat(c->Array.length)
  c->Array.slice(~start=0, ~end=Float.toInt(end < 0. ? Math.max(length +. end, 0.) : Math.min(end, length)))
}

let digits = c => c->Array.map(d => Int.toString(d))->Array.join("")

// Upstream `parse`: the decimal string n, validated against NUMERIC.
let parse = source => {
  if !(numeric->RegExp.test(source)) {
    throw(BigError(invalid ++ "number"))
  }
  // Determine sign.
  let s = source->String.charAt(0) == "-" ? -1 : 1
  let n = s < 0 ? source->String.slice(~start=1) : source
  // Decimal point?
  let dot = n->String.indexOf(".")
  let n = dot > -1 ? n->String.replace(".", "") : n
  // Exponential form?
  let i = n->String.search(%re("/e/i"))
  let (e, n) = if i > 0 {
    // Determine exponent.
    let exponent = number(n->String.slice(~start=i + 1))
    (Int.toFloat(dot < 0 ? i : dot) +. exponent, n->String.slice(~start=0, ~end=i))
  } else {
    // Integer.
    (Int.toFloat(dot < 0 ? n->String.length : dot), n)
  }
  let nl = n->String.length
  // Determine leading zeros.
  let lead = ref(0)
  while lead.contents < nl && n->String.charAt(lead.contents) == "0" {
    lead := lead.contents + 1
  }
  if lead.contents == nl {
    // Zero.
    {s, e: 0., c: [0]}
  } else {
    // Determine trailing zeros.
    let last = ref(nl - 1)
    while n->String.charAt(last.contents) == "0" {
      last := last.contents - 1
    }
    // Convert string to array of digits without leading/trailing zeros.
    let c = Array.fromInitializer(~length=last.contents - lead.contents + 1, k =>
      n->String.charCodeAtUnsafe(lead.contents + k) - 48
    )
    {s, e: e -. Int.toFloat(lead.contents) -. 1., c}
  }
}

// Upstream `round`: x rounded in place to sd significant digits using rounding mode rm. `more`
// says whether a division was truncated.
let roundTo = (x, sd, rm, ~more=false) => {
  let xc = x.c
  if rm != 0. && rm != 1. && rm != 2. && rm != 3. {
    throw(BigError(invalidRm))
  }
  if sd < 1. {
    let first = xc->Array.getUnsafe(0)
    let up =
      (rm == 3. && (more || first != 0)) ||
        (sd == 0. &&
          ((rm == 1. && first >= 5) ||
            (rm == 2. && (first > 5 || (first == 5 && (more || xc[1]->Option.isSome))))))
    xc->setLength(1)
    if up {
      // 1, 0.1, 0.01, 0.001, 0.0001 etc.
      x.e = x.e -. sd +. 1.
      xc[0] = 1
    } else {
      // Zero.
      xc[0] = 0
      x.e = 0.
    }
  } else if sd < Int.toFloat(xc->Array.length) {
    let sd = Float.toInt(sd)
    // xc[sd] is the digit after the digit that may be rounded up.
    let next = xc->Array.getUnsafe(sd)
    let up =
      (rm == 1. && next >= 5) ||
      (rm == 2. &&
        (next > 5 ||
          (next == 5 && (more || xc[sd + 1]->Option.isSome || mod(xc->Array.getUnsafe(sd - 1), 2) == 1)))) ||
      (rm == 3. && (more || xc->Array.getUnsafe(0) != 0))
    // Remove any digits after the required precision.
    xc->setLength(sd)
    // Round up? Rounding up may mean the previous digit has to be rounded up.
    let i = ref(sd)
    let carry = ref(up)
    while carry.contents {
      i := i.contents - 1
      let digit = xc->Array.getUnsafe(i.contents) + 1
      if digit <= 9 {
        xc[i.contents] = digit
        carry := false
      } else {
        xc[i.contents] = 0
        if i.contents == 0 {
          x.e = x.e +. 1.
          xc->Array.unshift(1)
          carry := false
        }
      }
    }
    // Remove trailing zeros.
    while xc->Array.getUnsafe(xc->Array.length - 1) == 0 {
      xc->Array.pop->ignore
    }
  }
  x
}

// Upstream `stringify`: x in normal or exponential notation.
let stringify = (x, doExponential, isNonzero) => {
  let e = x.e
  let s = digits(x.c)
  let n = s->String.length
  let s = if doExponential {
    // Exponential notation.
    s->String.charAt(0) ++
    (n > 1 ? "." ++ s->String.slice(~start=1) : "") ++
    (e < 0. ? "e" : "e+") ++
    Float.toString(e)
  } else if e < 0. {
    // Normal notation.
    "0." ++ "0"->String.repeat(Float.toInt(-.e -. 1.)) ++ s
  } else if e > 0. {
    let point = Float.toInt(e) + 1
    if point > n {
      s ++ "0"->String.repeat(point - n)
    } else if point < n {
      s->String.slice(~start=0, ~end=point) ++ "." ++ s->String.slice(~start=point)
    } else {
      s
    }
  } else if n > 1 {
    s->String.charAt(0) ++ "." ++ s->String.slice(~start=1)
  } else {
    s
  }
  x.s < 0 && isNonzero ? "-" ++ s : s
}

// The first index below n at which a and b differ, or -1.
let firstDifference = (a, b, n) => {
  let i = ref(0)
  while i.contents < n && a->Array.getUnsafe(i.contents) == b->Array.getUnsafe(i.contents) {
    i := i.contents + 1
  }
  i.contents < n ? i.contents : -1
}

// Return 1, -1 or 0 as x is greater than, less than or equal to y.
let cmp = (x, y) => {
  let xc = x.c
  let yc = y.c
  let i = x.s
  let j = y.s
  let x0 = xc->Array.getUnsafe(0)
  let y0 = yc->Array.getUnsafe(0)
  let isneg = i < 0
  let sign = greater => greater != isneg ? 1 : -1
  if x0 == 0 || y0 == 0 {
    // Either zero?
    x0 == 0 ? (y0 == 0 ? 0 : -j) : i
  } else if i != j {
    // Signs differ?
    i
  } else if x.e != y.e {
    // Compare exponents.
    sign(x.e > y.e)
  } else {
    // Compare digit by digit, then lengths.
    let k = firstDifference(xc, yc, Math.Int.min(xc->Array.length, yc->Array.length))
    if k >= 0 {
      sign(xc->Array.getUnsafe(k) > yc->Array.getUnsafe(k))
    } else if xc->Array.length == yc->Array.length {
      0
    } else {
      sign(xc->Array.length > yc->Array.length)
    }
  }
}

// x divided by y, rounded if necessary to dp decimal places using rounding mode rm.
let div = (x, y, dp, rm) => {
  let a = x.c // dividend
  let b = y.c // divisor
  let sign = x.s == y.s ? 1 : -1
  if !isIntIn(dp, 0., maxDp) {
    throw(BigError(invalidDp))
  }
  // Divisor is zero?
  if b->Array.getUnsafe(0) == 0 {
    throw(BigError(divByZero))
  }
  if a->Array.getUnsafe(0) == 0 {
    // Dividend is 0? Return +-0.
    {s: sign, e: 0., c: [0]}
  } else {
    let bl = b->Array.length
    let al = a->Array.length
    let ai = ref(bl)
    let bz = [0]->Array.concat(b) // divisor with a leading zero
    let r = ref(a->Array.slice(~start=0, ~end=bl)) // remainder
    let qc = [] // quotient
    let qe = ref(x.e -. y.e)
    let p = ref(dp +. qe.contents +. 1.) // precision of the result
    let k = ref(p.contents < 0. ? 0. : p.contents)

    // Compare divisor and remainder.
    let compare = () => {
      let rl = r.contents->Array.length
      if bl != rl {
        bl > rl ? 1 : -1
      } else {
        let i = firstDifference(b, r.contents, bl)
        i < 0 ? 0 : b->Array.getUnsafe(i) > r.contents->Array.getUnsafe(i) ? 1 : -1
      }
    }
    // Subtract the divisor from the remainder. The remainder can't be more than 1 digit longer
    // than the divisor: equalise lengths using the divisor with an extra leading zero.
    let subtract = () => {
      let r = r.contents
      let rl = ref(r->Array.length)
      let bt = rl.contents == bl ? b : bz
      while rl.contents > 0 {
        rl := rl.contents - 1
        let at = rl.contents
        if r->Array.getUnsafe(at) < bt->Array.getUnsafe(at) {
          let ri = ref(at - 1)
          while ri.contents > 0 && r->Array.getUnsafe(ri.contents) == 0 {
            r[ri.contents] = 9
            ri := ri.contents - 1
          }
          r[ri.contents] = r->Array.getUnsafe(ri.contents) - 1
          r[at] = r->Array.getUnsafe(at) + 10
        }
        r[at] = r->Array.getUnsafe(at) - bt->Array.getUnsafe(at)
      }
      while r->Array.getUnsafe(0) == 0 {
        r->Array.shift->ignore
      }
    }

    // Add zeros to make remainder as long as divisor.
    while r.contents->Array.length < bl {
      r.contents->Array.push(0)
    }
    let going = ref(true)
    while going.contents {
      // n is how many times the divisor goes into current remainder (at most nine).
      let n = ref(0)
      let order = ref(compare())
      // If divisor < remainder, subtract divisor from remainder.
      while order.contents < 0 {
        subtract()
        n := n.contents + 1
        order := compare()
      }
      // Add the digit n to the result array.
      qc->Array.push(order.contents != 0 ? n.contents : n.contents + 1)
      // Update the remainder. Upstream's [undefined], the dividend's end, is the empty array.
      let rest = r.contents
      if rest->Array.length > 0 && rest->Array.getUnsafe(0) != 0 && order.contents != 0 {
        rest->Array.push(a[ai.contents]->Option.getOr(0))
      } else {
        r :=
          switch a[ai.contents] {
          | Some(digit) => [digit]
          | None => []
          }
      }
      let more = ai.contents < al || r.contents->Array.length > 0
      ai := ai.contents + 1
      // `k--` is falsy at 0 and at NaN, as `k > 0` is.
      going := more && k.contents > 0.
      k := k.contents -. 1.
    }

    // Leading zero? Do not remove if result is simply zero (qi == 1).
    let qi = qc->Array.length
    if qc->Array.getUnsafe(0) == 0 && qi != 1 {
      // There can't be more than one zero.
      qc->Array.shift->ignore
      qe := qe.contents -. 1.
      p := p.contents -. 1.
    }
    let q = {s: sign, e: qe.contents, c: qc}
    // Round?
    if Int.toFloat(qi) > p.contents {
      roundTo(q, p.contents, rm, ~more=r.contents->Array.length > 0)->ignore
    }
    q
  }
}

// x minus y, and x plus y; each turns into the other when the signs differ.
let rec minus = (x, that) => {
  let y = copy(that)
  let b = y.s
  if x.s != b {
    // Signs differ?
    y.s = -b
    plus(x, y)
  } else {
    let xc = ref(x.c->Array.copy)
    let yc = ref(y.c)
    let xe = x.e
    let ye = ref(y.e)
    if xc.contents->Array.getUnsafe(0) == 0 || yc.contents->Array.getUnsafe(0) == 0 {
      // Either zero?
      if yc.contents->Array.getUnsafe(0) != 0 {
        y.s = -b
        y
      } else if xc.contents->Array.getUnsafe(0) != 0 {
        copy(x)
      } else {
        y.s = 1
        y
      }
    } else {
      // Determine which is the bigger number. Prepend zeros to equalise exponents.
      let a = xe -. ye.contents
      let (xlty, a) = if truthy(a) {
        let zeros = Array.make(~length=Float.toInt(Math.abs(a)), 0)
        if a < 0. {
          xc := zeros->Array.concat(xc.contents)
        } else {
          ye := xe
          yc := zeros->Array.concat(yc.contents)
        }
        (a < 0., Math.abs(a))
      } else {
        // Exponents equal. Check digit by digit.
        let (xs, ys) = (xc.contents, yc.contents)
        let i = firstDifference(xs, ys, Math.Int.min(xs->Array.length, ys->Array.length))
        let xlty =
          i >= 0
            ? xs->Array.getUnsafe(i) < ys->Array.getUnsafe(i)
            : xs->Array.length < ys->Array.length
        (xlty, 0.)
      }
      // x < y? Point xc to the array of the bigger number.
      if xlty {
        let t = xc.contents
        xc := yc.contents
        yc := t
        y.s = -y.s
      }
      let xc = xc.contents
      let yc = yc.contents
      // Append zeros to xc if shorter. No need to add zeros to yc if shorter as subtraction only
      // needs to start at yc.length.
      while xc->Array.length < yc->Array.length {
        xc->Array.push(0)
      }
      // Subtract yc from xc.
      let j = ref(yc->Array.length)
      while Int.toFloat(j.contents) > a {
        j := j.contents - 1
        let at = j.contents
        if xc->Array.getUnsafe(at) < yc->Array.getUnsafe(at) {
          let i = ref(at - 1)
          while i.contents > 0 && xc->Array.getUnsafe(i.contents) == 0 {
            xc[i.contents] = 9
            i := i.contents - 1
          }
          xc[i.contents] = xc->Array.getUnsafe(i.contents) - 1
          xc[at] = xc->Array.getUnsafe(at) + 10
        }
        xc[at] = xc->Array.getUnsafe(at) - yc->Array.getUnsafe(at)
      }
      // Remove trailing zeros.
      while xc->Array.length > 0 && xc->Array.getUnsafe(xc->Array.length - 1) == 0 {
        xc->Array.pop->ignore
      }
      // Remove leading zeros and adjust exponent accordingly.
      while xc->Array.length > 0 && xc->Array.getUnsafe(0) == 0 {
        xc->Array.shift->ignore
        ye := ye.contents -. 1.
      }
      if xc->Array.length == 0 {
        // n - n = +0
        y.s = 1
        // Result must be zero.
        y.c = [0]
        y.e = 0.
      } else {
        y.c = xc
        y.e = ye.contents
      }
      y
    }
  }
}
and plus = (x, that) => {
  let y = copy(that)
  if x.s != y.s {
    // Signs differ?
    y.s = -y.s
    minus(x, y)
  } else if x.c->Array.getUnsafe(0) == 0 || y.c->Array.getUnsafe(0) == 0 {
    // Either zero?
    if y.c->Array.getUnsafe(0) != 0 {
      y
    } else if x.c->Array.getUnsafe(0) != 0 {
      copy(x)
    } else {
      y.s = x.s
      y
    }
  } else {
    let xc = ref(x.c->Array.copy)
    let yc = ref(y.c)
    let ye = ref(y.e)
    // Prepend zeros to equalise exponents.
    let e = x.e -. ye.contents
    if truthy(e) {
      let zeros = Array.make(~length=Float.toInt(Math.abs(e)), 0)
      if e > 0. {
        ye := x.e
        yc := zeros->Array.concat(yc.contents)
      } else {
        xc := zeros->Array.concat(xc.contents)
      }
    }
    // Point xc to the longer array.
    let (xc, yc) =
      xc.contents->Array.length - yc.contents->Array.length < 0
        ? (yc.contents, xc.contents)
        : (xc.contents, yc.contents)
    // Only start adding at yc.length - 1 as the further digits of xc can be left as they are.
    let k = ref(0)
    for i in yc->Array.length - 1 downto 0 {
      let sum = xc->Array.getUnsafe(i) + yc->Array.getUnsafe(i) + k.contents
      k := sum / 10
      xc[i] = mod(sum, 10)
    }
    // No need to check for zero, as +x + +y != 0 && -x + -y != 0
    if k.contents != 0 {
      xc->Array.unshift(k.contents)
      ye := ye.contents +. 1.
    }
    // Remove trailing zeros.
    while xc->Array.getUnsafe(xc->Array.length - 1) == 0 {
      xc->Array.pop->ignore
    }
    y.c = xc
    y.e = ye.contents
    y
  }
}

// x times y.
let times = (x, y) => {
  // Determine sign of result.
  let sign = x.s == y.s ? 1 : -1
  if x.c->Array.getUnsafe(0) == 0 || y.c->Array.getUnsafe(0) == 0 {
    // Return signed 0 if either 0.
    {s: sign, e: 0., c: [0]}
  } else {
    // If array xc has fewer digits than yc, swap xc and yc, and lengths.
    let (xc, yc) = x.c->Array.length < y.c->Array.length ? (y.c, x.c) : (x.c, y.c)
    let a = xc->Array.length
    let b = yc->Array.length
    // Initialise coefficient array of result with zeros.
    let c = Array.make(~length=a + b, 0)
    // Multiply.
    let carry = ref(0)
    for i in b - 1 downto 0 {
      carry := 0
      let j = ref(a + i)
      while j.contents > i {
        // Current sum of products at this digit position, plus carry.
        carry :=
          carry.contents +
          c->Array.getUnsafe(j.contents) +
          yc->Array.getUnsafe(i) * xc->Array.getUnsafe(j.contents - i - 1)
        c[j.contents] = mod(carry.contents, 10)
        j := j.contents - 1
        carry := carry.contents / 10
      }
      c[j.contents] = carry.contents
    }
    // Initialise exponent of result as x.e + y.e; increment it if there is a final carry,
    // otherwise remove leading zero.
    let e = if carry.contents != 0 {
      x.e +. y.e +. 1.
    } else {
      c->Array.shift->ignore
      x.e +. y.e
    }
    // Remove trailing zeros.
    while c->Array.getUnsafe(c->Array.length - 1) == 0 {
      c->Array.pop->ignore
    }
    {s: sign, e, c}
  }
}

// x raised to the integer power n; a negative power divides with dp and rm.
let pow = (x, n, dp, rm) => {
  let one = {s: 1, e: 0., c: [1]}
  switch n {
  | Some(n) if isIntIn(n, -.maxPower, maxPower) =>
    let isneg = n < 0.
    let k = ref(Float.toInt(Math.abs(n)))
    let base = ref(x)
    let y = ref(one)
    let going = ref(true)
    while going.contents {
      if Int.bitwiseAnd(k.contents, 1) != 0 {
        y := times(y.contents, base.contents)
      }
      k := Int.shiftRight(k.contents, 1)
      if k.contents == 0 {
        going := false
      } else {
        base := times(base.contents, base.contents)
      }
    }
    isneg ? div(one, y.contents, dp, rm) : y.contents
  | _ => throw(BigError(invalid ++ "exponent"))
  }
}

// x rounded to dp decimal places (0 when absent) using rounding mode rm.
let round = (x, dp, rm) => {
  switch dp {
  | Some(dp) if !isIntIn(dp, -.maxDp, maxDp) => throw(BigError(invalidDp))
  | _ => ()
  }
  roundTo(copy(x), dp->Option.getOr(0.) +. x.e +. 1., rm)
}

// JS `toExponential()` up to and including its `e`: the shortest digits of s in d.ddd form.
let mantissa = s => {
  let text = s->Float.toExponential
  text->String.slice(~start=0, ~end=text->String.indexOf("e") + 1)
}

let sameDigits = (a, b) => Array.equal(a, b, (u: int, v) => u == v)

// The square root of x, rounded to dp decimal places using rounding mode rm.
let sqrt = (x, dp, rm) => {
  let half = {s: 1, e: -1., c: [5]}
  if x.c->Array.getUnsafe(0) == 0 {
    // Zero?
    copy(x)
  } else if x.s < 0 {
    // Negative?
    throw(BigError(name ++ "No square root"))
  } else {
    // Estimate.
    let estimate = Math.sqrt(number(stringify(x, true, true)))
    let e = x.e
    // Math.sqrt underflow/overflow?
    // Re-estimate: pass x coefficient to Math.sqrt as integer, then adjust the result exponent.
    let r = ref(
      if estimate == 0. || estimate == Float.Constants.positiveInfinity {
        let c = digits(x.c)
        let c = Int.bitwiseAnd(toInt32(Int.toFloat(c->String.length) +. e), 1) == 0 ? c ++ "0" : c
        let estimate = Math.sqrt(number(c))
        let exponent =
          toInt32((e +. 1.) /. 2.) - (e < 0. || Int.bitwiseAnd(toInt32(e), 1) != 0 ? 1 : 0)
        let prefix = estimate == Float.Constants.positiveInfinity ? "5e" : mantissa(estimate)
        parse(prefix ++ Int.toString(exponent))
      } else {
        parse(Float.toString(estimate))
      },
    )
    // Big.DP += 4 for the iteration; div validates it.
    let places = dp +. 4.
    let compared = r.contents.e +. places
    // Newton-Raphson iteration.
    let going = ref(true)
    while going.contents {
      let t = r.contents
      r := times(half, plus(t, div(x, t, places, rm)))
      going := !sameDigits(head(t.c, compared), head(r.contents.c, compared))
    }
    roundTo(r.contents, dp +. r.contents.e +. 1., rm)
  }
}

// Exponential notation with dp fixed decimal places, rounded using rounding mode rm.
let toExponential = (x, dp, rm) => {
  let n = x.c->Array.getUnsafe(0)
  let x = switch dp {
  | None => x
  | Some(dp) =>
    if !isIntIn(dp, 0., maxDp) {
      throw(BigError(invalidDp))
    }
    let x = roundTo(copy(x), dp +. 1., rm)
    while Int.toFloat(x.c->Array.length) < dp +. 1. {
      x.c->Array.push(0)
    }
    x
  }
  stringify(x, true, n != 0)
}

// Normal notation with dp fixed decimal places, rounded using rounding mode rm.
let toFixed = (x, dp, rm) => {
  let n = x.c->Array.getUnsafe(0)
  let x = switch dp {
  | None => x
  | Some(dp) =>
    if !isIntIn(dp, 0., maxDp) {
      throw(BigError(invalidDp))
    }
    let x = roundTo(copy(x), dp +. x.e +. 1., rm)
    // x.e may have changed if the value is rounded up.
    while Int.toFloat(x.c->Array.length) < dp +. x.e +. 1. {
      x.c->Array.push(0)
    }
    x
  }
  stringify(x, false, n != 0)
}

// sd significant digits, rounded using rounding mode rm, in exponential notation if sd is less
// than the number of digits of the integer part.
let toPrecision = (x, sd, rm) => {
  let n = x.c->Array.getUnsafe(0)
  let x = switch sd {
  | None => x
  | Some(sd) =>
    if !isIntIn(sd, 1., maxDp) {
      throw(BigError(invalid ++ "precision"))
    }
    let x = roundTo(copy(x), sd, rm)
    while Int.toFloat(x.c->Array.length) < sd {
      x.c->Array.push(0)
    }
    x
  }
  let exponential = switch sd {
  | Some(sd) => sd <= x.e
  | None => false
  }
  stringify(x, exponential || x.e <= ne || x.e >= pe, n != 0)
}

// Exponential notation at exponents of NE and below or PE and above; no sign for zero.
let toString = x => stringify(x, x.e <= ne || x.e >= pe, x.c->Array.getUnsafe(0) != 0)
