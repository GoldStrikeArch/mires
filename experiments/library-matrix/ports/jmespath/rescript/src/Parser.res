// jmespath's Pratt parser, method by method. Nodes are plain JS objects with jmespath's property
// order, built as object literals, since `compile` returns them and `&expr` passes them around as
// values. A parse result is `option<node>`: `None` is jmespath's `undefined` node.

open Host
open Lexer

let node = (nodeType): option<node> => Some(nodeOf({"type": nodeType}))
let identity = () => node("Identity")
let branch = (nodeType, children: array<option<node>>) =>
  Some(nodeOf({"type": nodeType, "children": children}))

let power = tokenType => bindingPower(tokenType)->Option.getOrThrow

// `rbp < bindingPower[type]`, false where jmespath's table has no entry.
let bindsTighterThan = (tokenType, rbp) =>
  switch bindingPower(tokenType) {
  | Some(power) => rbp < power
  | None => false
  }

let comparatorName = tokenType =>
  switch tokenType {
  | EQ => "EQ"
  | NE => "NE"
  | GT => "GT"
  | GTE => "GTE"
  | LT => "LT"
  | _ => "LTE"
  }

type parser = {tokens: array<token>, mutable index: int}

let lookaheadToken = (p, number) => p.tokens->Array.getUnsafe(p.index + number)
let lookahead = (p, number) => lookaheadToken(p, number).tokenType
let advance = p => p.index = p.index + 1

let match_ = (p, tokenType) =>
  if lookahead(p, 0) == tokenType {
    advance(p)
  } else {
    throw(Errors.syntaxError("Unexpected token"))
  }

let errorToken = () => Errors.syntaxError("Invalid token")

let rec expression = (p, rbp) => {
  let leftToken = lookaheadToken(p, 0)
  advance(p)
  let left = ref(nud(p, leftToken))
  let currentToken = ref(lookahead(p, 0))
  while bindsTighterThan(currentToken.contents, rbp) {
    advance(p)
    left := led(p, currentToken.contents, left.contents)
    currentToken := lookahead(p, 0)
  }
  left.contents
}

and nud = (p, token) =>
  switch token.tokenType {
  | Literal => Some(nodeOf({"type": "Literal", "value": token.value}))
  | UnquotedIdentifier => Some(nodeOf({"type": "Field", "name": token.value}))
  | QuotedIdentifier =>
    let field = Some(nodeOf({"type": "Field", "name": token.value}))
    if lookahead(p, 0) == Lparen {
      throw(Errors.syntaxError("Quoted identifier not allowed for function names."))
    }
    field
  | Not => branch("NotExpression", [expression(p, power(Not))])
  | Star =>
    let right = lookahead(p, 0) == Rbracket ? identity() : parseProjectionRHS(p, power(Star))
    branch("ValueProjection", [identity(), right])
  | Filter => led(p, token.tokenType, identity())
  | Lbrace => parseMultiselectHash(p)
  | Flatten =>
    let left = branch("Flatten", [identity()])
    branch("Projection", [left, parseProjectionRHS(p, power(Flatten))])
  | Lbracket =>
    if lookahead(p, 0) == Number || lookahead(p, 0) == Colon {
      projectIfSlice(p, identity(), parseIndexExpression(p))
    } else if lookahead(p, 0) == Star && lookahead(p, 1) == Rbracket {
      advance(p)
      advance(p)
      branch("Projection", [identity(), parseProjectionRHS(p, power(Star))])
    } else {
      parseMultiselectList(p)
    }
  | Current => node("Current")
  | Expref => branch("ExpressionReference", [expression(p, power(Expref))])
  // `args[0]`: undefined for `()`.
  | Lparen =>
    let args = parseArguments(p, ~commas=false)
    Array.length(args) == 0 ? None : args->Array.getUnsafe(0)
  | _ => throw(errorToken())
  }

and led = (p, tokenName, left) =>
  switch tokenName {
  | Dot =>
    let rbp = power(Dot)
    if lookahead(p, 0) != Star {
      branch("Subexpression", [left, parseDotRHS(p, rbp)])
    } else {
      advance(p)
      branch("ValueProjection", [left, parseProjectionRHS(p, rbp)])
    }
  | Pipe => branch("Pipe", [left, expression(p, power(Pipe))])
  | Or => branch("OrExpression", [left, expression(p, power(Or))])
  | And => branch("AndExpression", [left, expression(p, power(And))])
  | Lparen =>
    switch left {
    // `left.name`: a TypeError (syntax) when there is no left node, as after `()`.
    | None => throw(Errors.syntaxError("Function call without a name"))
    | Some(left) =>
      let name = left.name
      Some(nodeOf({"type": "Function", "name": name, "children": parseArguments(p, ~commas=true)}))
    }
  | Filter =>
    let condition = expression(p, 0)
    match_(p, Rbracket)
    let right = lookahead(p, 0) == Flatten ? identity() : parseProjectionRHS(p, power(Filter))
    branch("FilterProjection", [left, right, condition])
  | Flatten =>
    branch("Projection", [branch("Flatten", [left]), parseProjectionRHS(p, power(Flatten))])
  | EQ | NE | GT | GTE | LT | LTE => parseComparator(p, left, tokenName)
  | Lbracket =>
    let token = lookaheadToken(p, 0)
    if token.tokenType == Number || token.tokenType == Colon {
      projectIfSlice(p, left, parseIndexExpression(p))
    } else {
      match_(p, Star)
      match_(p, Rbracket)
      branch("Projection", [left, parseProjectionRHS(p, power(Star))])
    }
  | _ => throw(errorToken())
  }

// The argument loops of `(…)` and of a function call. An argument that starts with `@` is the bare
// Current node, so `abs(@.a)` fails on the `.` as in jmespath. Calls may separate arguments with
// commas; parentheses may not.
and parseArguments = (p, ~commas) => {
  let args = []
  while lookahead(p, 0) != Rparen {
    let argument = if lookahead(p, 0) == Current {
      advance(p)
      node("Current")
    } else {
      expression(p, 0)
    }
    if commas && lookahead(p, 0) == Comma {
      match_(p, Comma)
    }
    args->Array.push(argument)
  }
  match_(p, Rparen)
  args
}

and parseIndexExpression = p =>
  if lookahead(p, 0) == Colon || lookahead(p, 1) == Colon {
    parseSliceExpression(p)
  } else {
    let index = nodeOf({"type": "Index", "value": lookaheadToken(p, 0).value})
    advance(p)
    match_(p, Rbracket)
    index
  }

and projectIfSlice = (p, left, right) => {
  let indexExpr = branch("IndexExpression", [left, Some(right)])
  stringValue(right.nodeType) == Some("Slice")
    ? branch("Projection", [indexExpr, parseProjectionRHS(p, power(Star))])
    : indexExpr
}

// `[start:stop:step]`, each part optional; a later number in one part replaces an earlier one.
and parseSliceExpression = p => {
  let parts = [jsNull, jsNull, jsNull]
  let index = ref(0)
  let currentToken = ref(lookahead(p, 0))
  while currentToken.contents != Rbracket && index.contents < 3 {
    switch currentToken.contents {
    | Colon =>
      index := index.contents + 1
      advance(p)
    | Number =>
      parts->Array.setUnsafe(index.contents, lookaheadToken(p, 0).value)
      advance(p)
    | _ => throw(Errors.syntaxError("Syntax error, unexpected token"))
    }
    currentToken := lookahead(p, 0)
  }
  match_(p, Rbracket)
  nodeOf({"type": "Slice", "children": parts})
}

and parseComparator = (p, left, comparator) => {
  let right = expression(p, power(comparator))
  Some(nodeOf({"type": "Comparator", "name": comparatorName(comparator), "children": [left, right]}))
}

// Undefined (no error) when nothing that can follow a dot follows it, as in jmespath.
and parseDotRHS = (p, rbp) =>
  switch lookahead(p, 0) {
  | UnquotedIdentifier | QuotedIdentifier | Star => expression(p, rbp)
  | Lbracket =>
    match_(p, Lbracket)
    parseMultiselectList(p)
  | Lbrace =>
    match_(p, Lbrace)
    parseMultiselectHash(p)
  | _ => None
  }

and parseProjectionRHS = (p, rbp) => {
  let next = lookahead(p, 0)
  switch bindingPower(next) {
  | Some(power) if power < 10 => identity()
  | _ if next == Lbracket || next == Filter => expression(p, rbp)
  | _ if next == Dot =>
    match_(p, Dot)
    parseDotRHS(p, rbp)
  | _ => throw(Errors.syntaxError("Sytanx error, unexpected token"))
  }
}

// `[a, b]`; the comma between elements is optional, a trailing one is an error.
and parseMultiselectList = p => {
  let expressions = []
  while lookahead(p, 0) != Rbracket {
    expressions->Array.push(expression(p, 0))
    if lookahead(p, 0) == Comma {
      match_(p, Comma)
      if lookahead(p, 0) == Rbracket {
        throw(Errors.syntaxError("Unexpected token Rbracket"))
      }
    }
  }
  match_(p, Rbracket)
  branch("MultiSelectList", expressions)
}

// `{k: v, …}`; as in jmespath, the comma between pairs is optional too.
and parseMultiselectHash = p => {
  let pairs = []
  let finished = ref(false)
  while !finished.contents {
    let keyToken = lookaheadToken(p, 0)
    if keyToken.tokenType != UnquotedIdentifier && keyToken.tokenType != QuotedIdentifier {
      throw(Errors.syntaxError("Expecting an identifier token"))
    }
    advance(p)
    match_(p, Colon)
    let value = expression(p, 0)
    pairs->Array.push(Some(nodeOf({"type": "KeyValuePair", "name": keyToken.value, "value": value})))
    if lookahead(p, 0) == Comma {
      match_(p, Comma)
    } else if lookahead(p, 0) == Rbrace {
      match_(p, Rbrace)
      finished := true
    }
  }
  branch("MultiSelectHash", pairs)
}

let parse = expression_ => {
  let tokens = tokenize(expression_)
  tokens->Array.push({tokenType: EOF, value: fromString(""), start: String.length(expression_)})
  let p = {tokens, index: 0}
  let ast = expression(p, 0)
  if lookahead(p, 0) != EOF {
    throw(Errors.syntaxError("Unexpected token type"))
  }
  ast
}
