package jmespath

/** The error categories of `libraries/jmespath/reference.mjs`, which reads them off jmespath's
  * exceptions: a message starting with "TypeError", "ArgumentError" or "Unknown function", the name
  * "RuntimeError", and anything else (lexer and parser errors, `JSON.parse` failures, engine errors)
  * as "syntax".
  */
enum ErrorCategory(val label: String):
  case Syntax extends ErrorCategory("syntax")
  case InvalidType extends ErrorCategory("invalid-type")
  case InvalidArity extends ErrorCategory("invalid-arity")
  case UnknownFunction extends ErrorCategory("unknown-function")
  case InvalidValue extends ErrorCategory("invalid-value")

final class JmesPathException(val category: ErrorCategory, message: String) extends Exception(message)

private[jmespath] object Errors:
  def syntaxError(message: String) = JmesPathException(ErrorCategory.Syntax, message)
  def typeError(message: String) = JmesPathException(ErrorCategory.InvalidType, s"TypeError: $message")
  def argumentError(message: String) = JmesPathException(ErrorCategory.InvalidArity, s"ArgumentError: $message")
  def unknownFunction(name: Any) = JmesPathException(ErrorCategory.UnknownFunction, s"Unknown function: $name()")
  def runtimeError(message: String) = JmesPathException(ErrorCategory.InvalidValue, message)
