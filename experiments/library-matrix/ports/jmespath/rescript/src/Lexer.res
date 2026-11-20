// jmespath's Lexer, method by method.

open Host

type tokenType =
  | EOF
  | UnquotedIdentifier
  | QuotedIdentifier
  | Rbracket
  | Rparen
  | Comma
  | Colon
  | Rbrace
  | Number
  | Current
  | Expref
  | Pipe
  | Or
  | And
  | EQ
  | GT
  | LT
  | GTE
  | LTE
  | NE
  | Flatten
  | Star
  | Filter
  | Dot
  | Not
  | Lbrace
  | Lbracket
  | Lparen
  | Literal

// jmespath's `bindingPower` table. Colon and Literal have no entry there, so their power is None:
// `rbp < bindingPower[Colon]` is false, and so is `bindingPower[Literal] < 10` in
// `_parseProjectionRHS`, which therefore rejects a literal after a projection.
let bindingPower = tokenType =>
  switch tokenType {
  | EOF
  | UnquotedIdentifier
  | QuotedIdentifier
  | Rbracket
  | Rparen
  | Comma
  | Rbrace
  | Number
  | Current
  | Expref =>
    Some(0)
  | Pipe => Some(1)
  | Or => Some(2)
  | And => Some(3)
  | EQ | GT | LT | GTE | LTE | NE => Some(5)
  | Flatten => Some(9)
  | Star => Some(20)
  | Filter => Some(21)
  | Dot => Some(40)
  | Not => Some(45)
  | Lbrace => Some(50)
  | Lbracket => Some(55)
  | Lparen => Some(60)
  | Colon | Literal => None
  }

type token = {tokenType: tokenType, value: value, start: int}

let isAlpha = ch => (ch >= "a" && ch <= "z") || (ch >= "A" && ch <= "Z") || ch == "_"
let isNum = ch => (ch >= "0" && ch <= "9") || ch == "-"
let isAlphaNum = ch => isAlpha(ch) || (ch >= "0" && ch <= "9")

let basicToken = ch =>
  switch ch {
  | "." => Some(Dot)
  | "*" => Some(Star)
  | "," => Some(Comma)
  | ":" => Some(Colon)
  | "{" => Some(Lbrace)
  | "}" => Some(Rbrace)
  | "]" => Some(Rbracket)
  | "(" => Some(Lparen)
  | ")" => Some(Rparen)
  | "@" => Some(Current)
  | _ => None
  }

// `String.prototype.trimLeft`'s set: ECMAScript WhiteSpace and LineTerminator.
let isJsWhiteSpace = unit =>
  switch unit {
  | 0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0x20 | 0xa0 | 0x1680 | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff => true
  | _ => unit >= 0x2000 && unit <= 0x200a
  }

let trimLeft = text => {
  let start = ref(0)
  while start.contents < String.length(text) && isJsWhiteSpace(text->String.charCodeAtUnsafe(start.contents)) {
    start := start.contents + 1
  }
  text->String.slice(~start=start.contents)
}

let looksLikeJson = literalString =>
  if literalString == "" {
    false
  } else if String.includes("[{\"", literalString->String.getUnsafe(0)) {
    true
  } else if literalString == "true" || literalString == "false" || literalString == "null" {
    true
  } else if String.includes("-0123456789", literalString->String.getUnsafe(0)) {
    try {
      parseJson(literalString)->ignore
      true
    } catch {
    | _ => false
    }
  } else {
    false
  }

let tokenize = (stream: string): array<token> => {
  let tokens = []
  let length = String.length(stream)
  let current = ref(0)
  let at = i => i < length ? stream->String.getUnsafe(i) : ""
  let add = (tokenType, value, start) => tokens->Array.push({tokenType, value, start})

  let consumeUnquotedIdentifier = () => {
    let start = current.contents
    current := current.contents + 1
    while current.contents < length && isAlphaNum(at(current.contents)) {
      current := current.contents + 1
    }
    stream->String.slice(~start, ~end=current.contents)
  }

  // Skips a quoted body, where a backslash escapes `quote` or another backslash.
  let skipQuoted = quote =>
    while current.contents < length && at(current.contents) != quote {
      let next = at(current.contents + 1)
      let escaped = at(current.contents) == "\\" && (next == "\\" || next == quote)
      current := current.contents + (escaped ? 2 : 1)
    }

  let consumeQuotedIdentifier = () => {
    let start = current.contents
    current := current.contents + 1
    skipQuoted("\"")
    current := current.contents + 1
    parseJson(stream->String.slice(~start, ~end=current.contents))
  }

  // An unterminated raw string runs to the end of the expression, as in jmespath.
  let consumeRawStringLiteral = () => {
    let start = current.contents
    current := current.contents + 1
    skipQuoted("'")
    current := current.contents + 1
    stream->String.slice(~start=start + 1, ~end=current.contents - 1)->String.replace("\\'", "'")
  }

  let consumeNumber = () => {
    let start = current.contents
    current := current.contents + 1
    while current.contents < length && isNum(at(current.contents)) {
      current := current.contents + 1
    }
    add(Number, fromFloat(parseInt(stream->String.slice(~start, ~end=current.contents))), start)
  }

  let consumeLBracket = () => {
    let start = current.contents
    current := current.contents + 1
    switch at(current.contents) {
    | "?" =>
      current := current.contents + 1
      add(Filter, fromString("[?"), start)
    | "]" =>
      current := current.contents + 1
      add(Flatten, fromString("[]"), start)
    | _ => add(Lbracket, fromString("["), start)
    }
  }

  // A lone `=` is the one character jmespath accepts here without making a token: it pushes
  // `undefined`, and parsing fails on it with a TypeError, which is a syntax error. That error is
  // raised here.
  let consumeOperator = () => {
    let start = current.contents
    let startingChar = at(start)
    current := current.contents + 1
    let equalsFollows = at(current.contents) == "="
    if equalsFollows {
      current := current.contents + 1
    }
    switch (startingChar, equalsFollows) {
    | ("!", true) => add(NE, fromString("!="), start)
    | ("!", false) => add(Not, fromString("!"), start)
    | ("<", true) => add(LTE, fromString("<="), start)
    | ("<", false) => add(LT, fromString("<"), start)
    | (">", true) => add(GTE, fromString(">="), start)
    | (">", false) => add(GT, fromString(">"), start)
    | (_, true) => add(EQ, fromString("=="), start)
    | (_, false) => throw(Errors.syntaxError("Unknown operator: ="))
    }
  }

  let consumeLiteral = () => {
    current := current.contents + 1
    let start = current.contents
    skipQuoted("`")
    let literalString =
      stream->String.slice(~start, ~end=current.contents)->trimLeft->String.replace("\\`", "`")
    let literal = looksLikeJson(literalString)
      ? parseJson(literalString)
      : parseJson("\"" ++ literalString ++ "\"")
    current := current.contents + 1
    literal
  }

  while current.contents < length {
    let start = current.contents
    let ch = at(start)
    switch basicToken(ch) {
    | _ if isAlpha(ch) => add(UnquotedIdentifier, fromString(consumeUnquotedIdentifier()), start)
    | Some(tokenType) =>
      add(tokenType, fromString(ch), start)
      current := current.contents + 1
    | None if isNum(ch) => consumeNumber()
    | None =>
      switch ch {
      | "[" => consumeLBracket()
      | "\"" => add(QuotedIdentifier, consumeQuotedIdentifier(), start)
      | "'" => add(Literal, fromString(consumeRawStringLiteral()), start)
      | "`" => add(Literal, consumeLiteral(), start)
      | "<" | ">" | "=" | "!" => consumeOperator()
      | " " | "\t" | "\n" => current := current.contents + 1
      | "&" | "|" =>
        current := current.contents + 1
        let doubled = at(current.contents) == ch
        if doubled {
          current := current.contents + 1
        }
        switch (ch, doubled) {
        | ("&", true) => add(And, fromString("&&"), start)
        | ("&", false) => add(Expref, fromString("&"), start)
        | (_, true) => add(Or, fromString("||"), start)
        | (_, false) => add(Pipe, fromString("|"), start)
        }
      | _ => throw(Errors.syntaxError("Unknown character:" ++ ch))
      }
    }
  }
  tokens
}
