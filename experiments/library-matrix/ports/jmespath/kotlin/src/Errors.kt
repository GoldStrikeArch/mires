package jmespath

/**
 * The error categories of `libraries/jmespath/reference.mjs`, which reads them off jmespath's
 * exceptions: a message starting with "TypeError", "ArgumentError" or "Unknown function", the
 * name "RuntimeError", and anything else (lexer and parser errors, `JSON.parse` failures, engine
 * errors) as "syntax".
 */
internal enum class ErrorCategory(val label: String) {
    Syntax("syntax"),
    InvalidType("invalid-type"),
    InvalidArity("invalid-arity"),
    UnknownFunction("unknown-function"),
    InvalidValue("invalid-value"),
}

internal class JmesPathException(val category: ErrorCategory, message: String) : Exception(message)

internal fun syntaxError(message: String) = JmesPathException(ErrorCategory.Syntax, message)
internal fun typeError(message: String) = JmesPathException(ErrorCategory.InvalidType, "TypeError: $message")
internal fun argumentError(message: String) = JmesPathException(ErrorCategory.InvalidArity, "ArgumentError: $message")
internal fun unknownFunction(name: Any?) = JmesPathException(ErrorCategory.UnknownFunction, "Unknown function: $name()")
internal fun runtimeError(message: String) = JmesPathException(ErrorCategory.InvalidValue, message)
