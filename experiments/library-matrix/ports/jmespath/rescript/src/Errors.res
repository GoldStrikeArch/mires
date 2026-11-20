// The error categories of `libraries/jmespath/reference.mjs`, which reads them off jmespath's
// exceptions: a message starting with "TypeError", "ArgumentError" or "Unknown function", the name
// "RuntimeError", and anything else (lexer and parser errors, `JSON.parse` failures, engine errors)
// as "syntax".
type category = Syntax | InvalidType | InvalidArity | UnknownFunction | InvalidValue

let label = category =>
  switch category {
  | Syntax => "syntax"
  | InvalidType => "invalid-type"
  | InvalidArity => "invalid-arity"
  | UnknownFunction => "unknown-function"
  | InvalidValue => "invalid-value"
  }

exception JmesPathError(category, string)

let syntaxError = message => JmesPathError(Syntax, message)
let typeError = message => JmesPathError(InvalidType, "TypeError: " ++ message)
let argumentError = message => JmesPathError(InvalidArity, "ArgumentError: " ++ message)
let unknownFunction = name => JmesPathError(UnknownFunction, "Unknown function: " ++ name ++ "()")
let runtimeError = message => JmesPathError(InvalidValue, message)
