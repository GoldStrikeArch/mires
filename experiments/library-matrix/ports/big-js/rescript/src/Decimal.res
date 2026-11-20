/* Decimal coefficients contain normalized base-ten digits, most significant first. */
type decimal = {s:int, e:float, c:array<int>}
let dec = (s,e,c) => {s,e,c}
let filled = (n,d) => Array.fromInitializer(~length=n,_=>d)
let size = Array.length
let slen = String.length
@send external cut:(array<int>,int,int)=>array<int>="slice"
let extend = (a,d) => Array.concat(a,[d])
@send external code:(string,int)=>int="charCodeAt"
@scope("String") @val external letter:int=>string="fromCharCode"
@val external floatText:float=>string="String"
@val external strint:int=>string="String"
@val external decimalFloat:string=>float="Number"
let cat=(a,b)=>a++b
let machineRoot=s=>Math.sqrt(decimalFloat(s))
@scope("Number") @val external finite:float=>bool="isFinite"
let asFloat = Int.toFloat
let asIndex = Int.fromFloat
let maxExp = Math.max
let minExp = Math.min
let even = e => mod_float(e,2.0) == 0.0
let halvedExponent = e => Math.floor(e /. 2.0)
@send external piece: (string,int,int)=>string = "slice"
let parseExponent = (s,start,stop) => decimalFloat(piece(s,start,stop))
let grow = a => Array.concat(a,filled(Array.length(a),0))
let min=Math.Int.min
let max=Math.Int.max
exception Invalid(string)
let error=msg=>raise(Invalid(msg))


let normalizeFrom = (sign: int, exponent: float, digits: array<int>, offset: int): decimal => {
    let start = ref(offset)
    while ((start.contents < size(digits)) && (Array.getUnsafe(digits, start.contents) == 0)) {
        start := start.contents + 1
    }
    let stop = ref(size(digits))
    while ((stop.contents > start.contents) && (Array.getUnsafe(digits, (stop.contents - 1)) == 0)) {
        stop := stop.contents - 1
    }
    let out = ref(dec(sign, 0.0, filled(1, 0)))
    if (stop.contents > start.contents) {
        out := dec(sign, (exponent -. asFloat((start.contents - offset))), cut(digits, start.contents, stop.contents))
    }
    out.contents
}

let normalize = (sign: int, exponent: float, digits: array<int>): decimal => {
    normalizeFrom(sign, exponent, digits, 0)
}

let parse = (text: string): decimal => {
    let length = slen(text)
    let pos = ref(0)
    let sign = ref(1)
    if ((pos.contents < length) && (code(text, pos.contents) == 45)) {
        sign := (-1)
        pos := pos.contents + 1
    }
    let digits = filled(length, 0)
    let count = ref(0)
    let places = ref(0)
    while ((pos.contents < length) && (code(text, pos.contents) >= 48) && (code(text, pos.contents) <= 57)) {
        digits[count.contents] = (code(text, pos.contents) - 48)
        count := count.contents + 1
        pos := pos.contents + 1
        places := places.contents + 1
    }
    if ((pos.contents < length) && (code(text, pos.contents) == 46)) {
        pos := pos.contents + 1
        while ((pos.contents < length) && (code(text, pos.contents) >= 48) && (code(text, pos.contents) <= 57)) {
            digits[count.contents] = (code(text, pos.contents) - 48)
            count := count.contents + 1
            pos := pos.contents + 1
        }
    }
    if (count.contents == 0) {
        error("[big.js] Invalid number")
    }
    let scale = ref(0.0)
    if ((pos.contents < length) && ((code(text, pos.contents) == 69) || (code(text, pos.contents) == 101))) {
        pos := pos.contents + 1
        let exponentStart = pos.contents
        if ((pos.contents < length) && ((code(text, pos.contents) == 45) || (code(text, pos.contents) == 43))) {
            pos := pos.contents + 1
        }
        let start = pos.contents
        while ((pos.contents < length) && (code(text, pos.contents) >= 48) && (code(text, pos.contents) <= 57)) {
            pos := pos.contents + 1
        }
        if (pos.contents == start) {
            error("[big.js] Invalid number")
        }
        scale := parseExponent(text, exponentStart, pos.contents)
    }
    if (pos.contents != length) {
        error("[big.js] Invalid number")
    }
    let leading = ref(0)
    while ((leading.contents < count.contents) && (Array.getUnsafe(digits, leading.contents) == 0)) {
        leading := leading.contents + 1
    }
    normalizeFrom(sign.contents, (((asFloat(places.contents) +. scale.contents) -. asFloat(leading.contents)) -. 1.0), cut(digits, 0, count.contents), leading.contents)
}

let coefficient = (x: decimal, idx: int): int => {
    (((idx >= 0) && (idx < size(x.c))) ? Array.getUnsafe(x.c, idx) : 0)
}

let cmp = (x: decimal, y: decimal): int => {
    let result = ref(0)
    if (Array.getUnsafe(x.c, 0) == 0) {
        result := ((Array.getUnsafe(y.c, 0) == 0) ? 0 : (-y.s))
    } else {
        if ((Array.getUnsafe(y.c, 0) == 0) || (x.s != y.s)) {
            result := x.s
        } else {
            if (x.e != y.e) {
                result := ((x.e > y.e) ? x.s : (-x.s))
            } else {
                let i = ref(0)
                let limit = min(size(x.c), size(y.c))
                while ((i.contents < limit) && (Array.getUnsafe(x.c, i.contents) == Array.getUnsafe(y.c, i.contents))) {
                    i := i.contents + 1
                }
                if (i.contents < limit) {
                    result := ((Array.getUnsafe(x.c, i.contents) > Array.getUnsafe(y.c, i.contents)) ? x.s : (-x.s))
                } else {
                    if (size(x.c) != size(y.c)) {
                        result := ((size(x.c) > size(y.c)) ? x.s : (-x.s))
                    }
                }
            }
        }
    }
    result.contents
}

let add = (x: decimal, y: decimal, minus: bool): decimal => {
    let sign = (minus ? (-y.s) : y.s)
    let result = ref(dec(sign, y.e, y.c))
    if (Array.getUnsafe(x.c, 0) == 0) {
        if ((Array.getUnsafe(y.c, 0) == 0) && (sign != x.s)) {
            result := dec(1, 0.0, filled(1, 0))
        }
    } else {
        if (Array.getUnsafe(y.c, 0) == 0) {
            result := x
        } else {
            let same = (x.s == sign)
            let reverse = ((!same) && ((cmp(x, dec(x.s, y.e, y.c)) * x.s) < 0))
            let top = maxExp(x.e, y.e)
            let offsetX = asIndex((top -. x.e))
            let offsetY = asIndex((top -. y.e))
            let width = max((offsetX + size(x.c)), (offsetY + size(y.c)))
            let digits = filled((width + 1), 0)
            let carry = ref(0)
            let p = ref((width - 1))
            while (p.contents >= 0) {
                let a = coefficient(x, (p.contents - offsetX))
                let b = coefficient(y, (p.contents - offsetY))
                let total = (carry.contents + (same ? (a + b) : (reverse ? (b - a) : (a - b))))
                carry := ((total < 0) ? (-1) : (total / 10))
                digits[(p.contents + 1)] = (total - (carry.contents * 10))
                p := p.contents - 1
            }
            digits[0] = carry.contents
            result := normalizeFrom((reverse ? (-x.s) : x.s), ((carry.contents != 0) ? (top +. 1.0) : top), digits, ((carry.contents != 0) ? 0 : 1))
            if (Array.getUnsafe(result.contents.c, 0) == 0) {
                result := dec(1, 0.0, result.contents.c)
            }
        }
    }
    result.contents
}

let times = (x: decimal, y: decimal): decimal => {
    let digits = filled((size(x.c) + size(y.c)), 0)
    let i = ref((size(x.c) - 1))
    while (i.contents >= 0) {
        let carry = ref(0)
        let j = ref((size(y.c) - 1))
        while (j.contents >= 0) {
            let k = ((i.contents + j.contents) + 1)
            let v = (((Array.getUnsafe(x.c, i.contents) * Array.getUnsafe(y.c, j.contents)) + Array.getUnsafe(digits, k)) + carry.contents)
            digits[k] = mod(v, 10)
            carry := (v / 10)
            j := j.contents - 1
        }
        digits[i.contents] = carry.contents
        i := i.contents - 1
    }
    let exponent = (x.e +. y.e)
    normalizeFrom((x.s * y.s), ((Array.getUnsafe(digits, 0) == 0) ? exponent : (exponent +. 1.0)), digits, ((Array.getUnsafe(digits, 0) == 0) ? 1 : 0))
}

let rounded = (x: decimal, precision: float, mode: int, more: bool): decimal => {
    if ((mode < 0) || (mode > 3)) {
        error("[big.js] Invalid rounding mode")
    }
    let result = ref(x)
    if (precision < asFloat(size(x.c))) {
        let sd = asIndex(maxExp(precision, (-1.0)))
        let guardDigit = ((sd >= 0) ? Array.getUnsafe(x.c, sd) : 0)
        let odd = ((sd > 0) && (mod(Array.getUnsafe(x.c, (sd - 1)), 2) == 1))
        let sticky = (((sd + 1) < size(x.c)) || more)
        let up = (((mode == 1) && (guardDigit >= 5)) || ((mode == 2) && ((guardDigit > 5) || ((guardDigit == 5) && (odd || sticky)))) || ((mode == 3) && ((Array.getUnsafe(x.c, 0) != 0) || more)))
        if (sd <= 0) {
            result := (up ? dec(x.s, ((x.e -. precision) +. 1.0), filled(1, 1)) : dec(x.s, 0.0, filled(1, 0)))
        } else {
            let digits = cut(x.c, 0, sd)
            let pos = ref((sd - 1))
            if up {
                while ((pos.contents >= 0) && (Array.getUnsafe(digits, pos.contents) == 9)) {
                    digits[pos.contents] = 0
                    pos := pos.contents - 1
                }
                if (pos.contents >= 0) {
                    digits[pos.contents] = (Array.getUnsafe(digits, pos.contents) + 1)
                }
            }
            result := ((pos.contents < 0) ? dec(x.s, (x.e +. 1.0), filled(1, 1)) : normalize(x.s, x.e, digits))
        }
    }
    result.contents
}

let natural = (a: array<int>, b: array<int>): int => {
    let result = ref(0)
    if (size(a) != size(b)) {
        result := ((size(a) > size(b)) ? 1 : (-1))
    } else {
        let i = ref(0)
        while ((i.contents < size(a)) && (Array.getUnsafe(a, i.contents) == Array.getUnsafe(b, i.contents))) {
            i := i.contents + 1
        }
        if (i.contents < size(a)) {
            result := ((Array.getUnsafe(a, i.contents) > Array.getUnsafe(b, i.contents)) ? 1 : (-1))
        }
    }
    result.contents
}

let divide = (x: decimal, y: decimal, dp: int, mode: int): decimal => {
    if ((dp < 0) || (dp > 1000000)) {
        error("[big.js] Invalid decimal places")
    }
    if (Array.getUnsafe(y.c, 0) == 0) {
        error("[big.js] Division by zero")
    }
    let sign = (x.s * y.s)
    let result = ref(dec(sign, 0.0, filled(1, 0)))
    if (Array.getUnsafe(x.c, 0) != 0) {
        let rest = ref(filled(size(y.c), 0))
        let pos = ref(0)
        while (pos.contents < size(y.c)) {
            rest.contents[pos.contents] = ((pos.contents < size(x.c)) ? Array.getUnsafe(x.c, pos.contents) : 0)
            pos := pos.contents + 1
        }
        let exponent = ref((x.e -. y.e))
        let digits = ref(filled(16, 0))
        let count = ref(0)
        let complete = ref(false)
        while (!complete.contents) {
            let quotient = ref(0)
            while (natural(rest.contents, y.c) >= 0) {
                let shift = (size(rest.contents) - size(y.c))
                let borrow = ref(0)
                let k = ref((size(rest.contents) - 1))
                while (k.contents >= 0) {
                    let dv = ((k.contents >= shift) ? Array.getUnsafe(y.c, (k.contents - shift)) : 0)
                    let v = ((Array.getUnsafe(rest.contents, k.contents) - dv) - borrow.contents)
                    borrow := ((v < 0) ? 1 : 0)
                    rest.contents[k.contents] = (v + (borrow.contents * 10))
                    k := k.contents - 1
                }
                let start = ref(0)
                while ((start.contents < size(rest.contents)) && (Array.getUnsafe(rest.contents, start.contents) == 0)) {
                    start := start.contents + 1
                }
                if (start.contents > 0) {
                    rest := cut(rest.contents, start.contents, size(rest.contents))
                }
                quotient := quotient.contents + 1
            }
            if ((quotient.contents == 0) && (count.contents == 0)) {
                exponent := (exponent.contents -. 1.0)
            } else {
                if (count.contents == size(digits.contents)) {
                    digits := grow(digits.contents)
                }
                digits.contents[count.contents] = quotient.contents
                count := count.contents + 1
            }
            let more = ((size(rest.contents) > 0) || (pos.contents < size(x.c)))
            complete := ((count.contents > 0) && ((!more) || (asFloat(count.contents) > ((asFloat(dp) +. exponent.contents) +. 1.0))))
            if complete.contents {
                result := dec(sign, exponent.contents, cut(digits.contents, 0, count.contents))
                if (asFloat(count.contents) > ((asFloat(dp) +. exponent.contents) +. 1.0)) {
                    result := rounded(result.contents, ((asFloat(dp) +. exponent.contents) +. 1.0), mode, more)
                }
            } else {
                let nxt = ((pos.contents < size(x.c)) ? Array.getUnsafe(x.c, pos.contents) : 0)
                if ((size(rest.contents) > 0) || (nxt != 0)) {
                    rest := extend(rest.contents, nxt)
                }
                pos := pos.contents + 1
            }
        }
    }
    result.contents
}

let power = (x: decimal, n: int, dp: int, mode: int): decimal => {
    if ((n < (-1000000)) || (n > 1000000)) {
        error("[big.js] Invalid exponent")
    }
    let one = dec(1, 0.0, filled(1, 1))
    let result = ref(one)
    let base = ref(x)
    let i = ref(((n < 0) ? (-n) : n))
    while (i.contents > 0) {
        if (mod(i.contents, 2) == 1) {
            result := times(result.contents, base.contents)
        }
        i := (i.contents / 2)
        if (i.contents > 0) {
            base := times(base.contents, base.contents)
        }
    }
    if (n < 0) {
        result := divide(one, result.contents, dp, mode)
    }
    result.contents
}

let render = (x: decimal, op: string, n: int, mode: int): string => {
    let v = ref(x)
    if (n != (-2000002)) {
        let sd = ref(asFloat(n))
        if (op == "toPrecision") {
            if ((n < 1) || (n > 1000000)) {
                error("[big.js] Invalid precision")
            }
        } else {
            if ((n < 0) || (n > 1000000)) {
                error("[big.js] Invalid decimal places")
            }
            sd := ((op == "toFixed") ? ((asFloat(n) +. v.contents.e) +. 1.0) : asFloat((n + 1)))
        }
        v := rounded(v.contents, sd.contents, mode, false)
        let width = ((op == "toFixed") ? ((asFloat(n) +. v.contents.e) +. 1.0) : sd.contents)
        if (width > asFloat(size(v.contents.c))) {
            let padded = filled(asIndex(width), 0)
            let i = ref(0)
            while (i.contents < size(v.contents.c)) {
                padded[i.contents] = Array.getUnsafe(v.contents.c, i.contents)
                i := i.contents + 1
            }
            v := dec(v.contents.s, v.contents.e, padded)
        }
    }
    let exponential = ((op == "toExponential") || ((op != "toFixed") && ((v.contents.e <= (-7.0)) || (v.contents.e >= 21.0) || ((op == "toPrecision") && (n != (-2000002)) && (asFloat(n) <= v.contents.e)))))
    let text = ref((((x.s < 0) && (Array.getUnsafe(x.c, 0) != 0)) ? "-" : ""))
    if exponential {
        text := cat(text.contents, letter((48 + Array.getUnsafe(v.contents.c, 0))))
        if (size(v.contents.c) > 1) {
            text := cat(text.contents, ".")
            let j = ref(1)
            while (j.contents < size(v.contents.c)) {
                text := cat(text.contents, letter((48 + Array.getUnsafe(v.contents.c, j.contents))))
                j := j.contents + 1
            }
        }
        text := cat(text.contents, ((v.contents.e >= 0.0) ? "e+" : "e"))
        text := cat(text.contents, floatText(v.contents.e))
    } else {
        let point = asIndex(v.contents.e)
        let p = ref(max(0, point))
        while (p.contents >= 0) {
            text := cat(text.contents, letter((48 + coefficient(v.contents, (point - p.contents)))))
            p := p.contents - 1
        }
        let last = ((point - size(v.contents.c)) + 1)
        if (last < 0) {
            text := cat(text.contents, ".")
            p := (-1)
            while (p.contents >= last) {
                text := cat(text.contents, letter((48 + coefficient(v.contents, (point - p.contents)))))
                p := p.contents - 1
            }
        }
    }
    text.contents
}

let root = (x: decimal, dp: int, mode: int): decimal => {
    let result = ref(x)
    if ((x.s > 0) && (dp >= 0) && (dp <= 999996) && (mode >= 0) && (mode <= 3) && (x.e < asFloat((((-2) * dp) - 2)))) {
        result := ((mode == 3) ? dec(1, asFloat((-dp)), filled(1, 1)) : dec(1, 0.0, filled(1, 0)))
    } else {
        if (Array.getUnsafe(x.c, 0) != 0) {
            if (x.s < 0) {
                error("[big.js] No square root")
            }
            let estimate = machineRoot(render(x, "toExponential", (-2000002), mode))
            if ((estimate > 0.0) && finite(estimate)) {
                result := parse(floatText(estimate))
            } else {
                let coefficient = ref("")
                let i = ref(0)
                while (i.contents < size(x.c)) {
                    coefficient := cat(coefficient.contents, letter((48 + Array.getUnsafe(x.c, i.contents))))
                    i := i.contents + 1
                }
                if even((asFloat(size(x.c)) +. x.e)) {
                    coefficient := cat(coefficient.contents, "0")
                }
                let fallback = machineRoot(coefficient.contents)
                let leading = (finite(fallback) ? parse(floatText(fallback)) : dec(1, 0.0, filled(1, 5)))
                result := dec(1, halvedExponent(x.e), leading.c)
            }
            let half = dec(1, (-1.0), filled(1, 5))
            let work = asIndex(minExp(1000001.0, maxExp(asFloat((dp + 4)), (5.0 -. result.contents.e))))
            let count = maxExp(1.0, (result.contents.e +. asFloat(work)))
            let matched = ref(false)
            while (!matched.contents) {
                let nxt = times(half, add(result.contents, divide(x, result.contents, work, mode), false))
                matched := (result.contents.e == nxt.e)
                let length = asIndex(minExp(count, asFloat(max(size(result.contents.c), size(nxt.c)))))
                let j = ref(0)
                while (matched.contents && (j.contents < length)) {
                    matched := ((j.contents < size(result.contents.c)) && (j.contents < size(nxt.c)) && (Array.getUnsafe(result.contents.c, j.contents) == Array.getUnsafe(nxt.c, j.contents)))
                    j := j.contents + 1
                }
                result := nxt
            }
            result := rounded(result.contents, ((asFloat(dp) +. result.contents.e) +. 1.0), mode, false)
        }
    }
    result.contents
}

let operate = (op: string, xText: string, yText: string, n: int, dp: int, rm: int, mode: int): string => {
    let x = parse(xText)
    let result = ref(x)
    let out = ref("")
    if ((op == "plus") || (op == "minus") || (op == "times") || (op == "div") || (op == "cmp")) {
        let y = parse(yText)
        if (op == "cmp") {
            out := strint(cmp(x, y))
        } else {
            if (op == "times") {
                result := times(x, y)
            } else {
                if (op == "div") {
                    result := divide(x, y, dp, rm)
                } else {
                    result := add(x, y, (op == "minus"))
                }
            }
        }
    } else {
        if (op == "round") {
            let places = ((n == (-2000002)) ? 0 : n)
            if ((places < (-1000000)) || (places > 1000000)) {
                error("[big.js] Invalid decimal places")
            }
            result := rounded(x, ((asFloat(places) +. x.e) +. 1.0), mode, false)
        } else {
            if (op == "pow") {
                result := power(x, n, dp, rm)
            } else {
                if (op == "sqrt") {
                    result := root(x, dp, rm)
                } else {
                    if ((op == "toString") || (op == "toExponential") || (op == "toFixed") || (op == "toPrecision")) {
                        out := render(x, op, ((op == "toString") ? (-2000002) : n), mode)
                    } else {
                        error("unknown decimal operation")
                    }
                }
            }
        }
    }
    if (out.contents == "") {
        out := render(result.contents, "toString", (-2000002), mode)
    }
    out.contents
}
