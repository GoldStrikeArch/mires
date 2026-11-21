// Entry module of the ReScript port: what libraries/jmespath/reference.mjs does, on the port's
// jmespath. The boundary: the input record is the corpus JS object itself, so reading it converts
// nothing; the document stays the JS value it is (jmespath returns parts of it), and each result is
// a plain object `{ value }` or `{ error }` with the reference's error category.

type query = {op: string, expression: string}
type input = {data: Host.value, queries: array<query>}

// jmespath's `compile`.
let compile = expression => Host.fromNode(Parser.parse(expression))

// jmespath's `search`: a fresh parse and interpretation per call.
let search = (data, expression) => Interpreter.visit(Parser.parse(expression), data)

let run = (input: input): array<dict<Host.value>> =>
  input.queries->Array.map(query =>
    try {
      dict{"value": query.op == "compile" ? compile(query.expression) : search(input.data, query.expression)}
    } catch {
    | Errors.JmesPathError(category, _) => dict{"error": Host.fromString(Errors.label(category))}
    // An exhausted stack propagates, as it does from the reference adapter.
    | JsExn(error) if Host.stackExhausted(error) => JsExn.throw(error)
    // The reference's category() default: JSON.parse's SyntaxError and engine errors.
    | _ => dict{"error": Host.fromString(Errors.label(Syntax))}
    }
  )
