/// The error categories of `libraries/jmespath/reference.mjs`, which reads them off jmespath's
/// exceptions: a message starting with "TypeError", "ArgumentError" or "Unknown function", the
/// name "RuntimeError", and anything else (lexer and parser errors, `JSON.parse` failures, engine
/// errors) as "syntax".
enum ErrorCategory {
  syntax('syntax'),
  invalidType('invalid-type'),
  invalidArity('invalid-arity'),
  unknownFunction('unknown-function'),
  invalidValue('invalid-value');

  const ErrorCategory(this.label);
  final String label;
}

class JmesPathException implements Exception {
  JmesPathException(this.category, this.message);
  final ErrorCategory category;
  final String message;

  @override
  String toString() => message;
}

JmesPathException syntaxError(String message) => JmesPathException(ErrorCategory.syntax, message);
JmesPathException typeError(String message) => JmesPathException(ErrorCategory.invalidType, 'TypeError: $message');
JmesPathException argumentError(String message) =>
    JmesPathException(ErrorCategory.invalidArity, 'ArgumentError: $message');
JmesPathException unknownFunction(Object? name) =>
    JmesPathException(ErrorCategory.unknownFunction, 'Unknown function: $name()');
JmesPathException runtimeError(String message) => JmesPathException(ErrorCategory.invalidValue, message);
