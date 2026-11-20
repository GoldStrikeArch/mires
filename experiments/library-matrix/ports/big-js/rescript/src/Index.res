// The boundary: what libraries/big-js/reference.mjs does. `dp`, `rm` and `repeat` are nullable:
// undefined and null select the defaults, as the adapter's ?? does. `n` and `mode` are optional
// fields and reach the methods as upstream's optional arguments.
@unboxed type operand = String(string) | Number(float)

type operation = {
  op: string,
  x: operand,
  y?: operand,
  n?: float,
  mode?: float,
  dp: Nullable.t<float>,
  rm: Nullable.t<float>,
}

type input = {cases: array<operation>, repeat: Nullable.t<int>}

// The Big constructor's conversion: a number goes through String(), except that -0 becomes "-0".
let text = operand =>
  switch operand {
  | Some(String(s)) => s
  | Some(Number(n)) => n == 0. && 1. /. n < 0. ? "-0" : Float.toString(n)
  | None => "undefined"
  }

let operate = c => {
  let dp = c.dp->Nullable.toOption->Option.getOr(20.)
  let rm = c.rm->Nullable.toOption->Option.getOr(1.)
  let x = Big.parse(text(Some(c.x)))
  let mode = c.mode->Option.getOr(rm)
  let y = () => Big.parse(text(c.y))
  switch c.op {
  | "toString" => x->Big.toString
  | "sqrt" => Big.sqrt(x, dp, rm)->Big.toString
  | "plus" => Big.plus(x, y())->Big.toString
  | "minus" => Big.minus(x, y())->Big.toString
  | "times" => Big.times(x, y())->Big.toString
  | "div" => Big.div(x, y(), dp, rm)->Big.toString
  | "cmp" => Big.cmp(x, y())->Int.toString
  | "pow" => Big.pow(x, c.n, dp, rm)->Big.toString
  | "round" => Big.round(x, c.n, mode)->Big.toString
  | "toFixed" => Big.toFixed(x, c.n, mode)
  | "toExponential" => Big.toExponential(x, c.n, mode)
  | "toPrecision" => Big.toPrecision(x, c.n, mode)
  | _ => throw(Big.BigError("unknown decimal operation"))
  }
}

let run = (input: input) => {
  let outputs = []
  for _ in 1 to input.repeat->Nullable.toOption->Option.getOr(1) {
    input.cases->Array.forEach(c =>
      outputs->Array.push(
        switch operate(c) {
        | result => result
        | exception Big.BigError(message) => message
        },
      )
    )
  }
  outputs->Array.join("\n")
}
