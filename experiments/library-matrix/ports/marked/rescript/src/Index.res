// Entry module of the ReScript port of marked@18.1.1: mirrors `libraries/marked/reference.mjs`.

type optionsJson = {
  gfm?: bool,
  breaks?: bool,
  pedantic?: bool,
}

type input = {
  op: string,
  markdown: string,
  options?: optionsJson,
}

let resolveOptions = (json: option<optionsJson>): Tokens.options =>
  switch json {
  | None => {gfm: true, breaks: false, pedantic: false}
  | Some(o) => {
      gfm: switch o.gfm {
      | Some(v) => v
      | None => true
      },
      breaks: switch o.breaks {
      | Some(v) => v
      | None => false
      },
      pedantic: switch o.pedantic {
      | Some(v) => v
      | None => false
      },
    }
  }

let run = (input: input): string => {
  let opts = resolveOptions(input.options)
  switch input.op {
  | "parse" => Parser.parse(Lexer.lex(input.markdown, opts))
  | "parseInline" => Parser.parseInline(Lexer.lexInline(input.markdown, opts))
  | op => JsError.TypeError.make(`unknown operation ${op}`)->JsError.throw
  }
}
