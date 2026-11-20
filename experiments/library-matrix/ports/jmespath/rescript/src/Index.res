open Host
let run = input => {
  let data = get(input, "data")
  let qs = get(input, "queries")
  let result = array()
  for i in 0 to length(qs) - 1 {
    let q = at(qs, i)
    let out = obj()
    try {
      let ast = Parser.compile(get(q, "expression"))
      let v = if strict(get(q, "op"), str("compile")) {
        ast
      } else {
        Eval.eval(ast, data)
      }
      set(out, "value", v)
    } catch {
    | Parser.Error(category) => set(out, "error", str(category))
    | _ => set(out, "error", str("syntax"))
    }
    push(result, out)
  }
  result
}
