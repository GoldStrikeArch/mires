package main

// The error categories of libraries/jmespath/reference.mjs, which reads them off jmespath's
// exceptions: a message starting with "TypeError", "ArgumentError" or "Unknown function", the name
// "RuntimeError", and anything else (lexer and parser errors, JSON.parse failures, engine errors)
// as "syntax".
const (
	categorySyntax          = "syntax"
	categoryInvalidType     = "invalid-type"
	categoryInvalidArity    = "invalid-arity"
	categoryUnknownFunction = "unknown-function"
	categoryInvalidValue    = "invalid-value"
)

// Error is a jmespath error with its category.
type Error struct {
	Category string
	Message  string
}

func (e *Error) Error() string { return e.Message }

func syntaxError(message string) error { return &Error{categorySyntax, message} }
func typeError(message string) error   { return &Error{categoryInvalidType, "TypeError: " + message} }
func argumentError(message string) error {
	return &Error{categoryInvalidArity, "ArgumentError: " + message}
}
func runtimeError(message string) error { return &Error{categoryInvalidValue, message} }
func unknownFunction(name string) error {
	return &Error{categoryUnknownFunction, "Unknown function: " + name + "()"}
}
