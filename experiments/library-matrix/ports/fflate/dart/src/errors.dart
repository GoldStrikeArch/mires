// Error codes and exception type matching fflate 0.8.3 (FlateErrorCode 0..6).

abstract final class FlateErrorCode {
  static const int unexpectedEOF = 0;
  static const int invalidBlockType = 1;
  static const int invalidLengthLiteral = 2;
  static const int invalidDistance = 3;
  static const int streamFinished = 4;
  static const int noStreamHandler = 5;
  static const int invalidHeader = 6;
}

const List<String> _errorMessages = [
  'unexpected EOF',
  'invalid block type',
  'invalid length/literal',
  'invalid distance',
  'stream finished',
  'no stream handler',
  'invalid header',
];

final class FlateException implements Exception {
  FlateException(this.code, [String? message])
      : message = message ?? (code >= 0 && code < _errorMessages.length ? _errorMessages[code] : 'flate error');

  final int code;
  final String message;

  @override
  String toString() => 'FlateException($code): $message';
}
