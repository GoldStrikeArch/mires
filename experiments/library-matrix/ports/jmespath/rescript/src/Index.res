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

// The reference adapter's `run`. Only jmespath's errors become categories: anything else thrown
// (stack exhaustion, a fault in the port) propagates, as it does out of the reference adapter. A JS
// exception is thrown on as itself, not as ReScript's `JsExn` wrapper.
let run = (input: input): array<dict<Host.value>> =>
  input.queries->Array.map(query =>
    try {
      dict{"value": query.op == "compile" ? compile(query.expression) : search(input.data, query.expression)}
    } catch {
    | Errors.JmesPathError(category, _) => dict{"error": Host.fromString(Errors.label(category))}
    | JsExn(error) => JsExn.throw(error)
    }
  )
