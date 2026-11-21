// Ky 2.2.0 core/retry-timing.js: which header gives a server-provided retry delay, and that delay.
// The patterns are Ky's own; on the web, Dart's RegExp is the JS one.

import 'dart:math' as math;

import 'host.dart';

class TimingHeader {
  const TimingHeader(this.value, {required this.allowTimestamp});
  final String value;
  final bool allowTimestamp;
}

/// A reset value is only read as an epoch from this second onwards (2001-09-09).
const _timestampThresholdSeconds = 1000000000.0;
const _msPerDay = 86400000.0;
final _delayPattern = RegExp(r'^\d+$');
const _months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

// HTTP-date parsing is limited to IMF-fixdate and the two obsolete HTTP-date formats.
final _imfDatePattern = RegExp(
  r'^(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun), (\d{2}) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{4}) (\d{2}):(\d{2}):(\d{2}) GMT$',
);
final _rfc850DatePattern = RegExp(
  r'^(?:Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday), (\d{2})-(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)-(\d{2}) (\d{2}):(\d{2}):(\d{2}) GMT$',
);
final _asctimeDatePattern = RegExp(
  r'^(Mon|Tue|Wed|Thu|Fri|Sat|Sun) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{2}| \d) (\d{2}:\d{2}:\d{2}) (\d{4})$',
);

/// Retry-After is authoritative when present. Only missing Retry-After falls through to the
/// rate-limit headers: RateLimit-Reset, Symfony's X-RateLimit-Retry-After, then GitHub's and
/// Twitter's reset headers.
const _timingHeaders = [
  ('Retry-After', false),
  ('RateLimit-Reset', true),
  ('X-RateLimit-Retry-After', false),
  ('X-RateLimit-Reset', true),
  ('X-Rate-Limit-Reset', true),
];

TimingHeader? getRetryTimingHeader(Headers headers) {
  for (final (name, allowTimestamp) in _timingHeaders) {
    final value = headers.get(name);
    if (value != null) return TimingHeader(value, allowTimestamp: allowTimestamp);
  }
  return null;
}

bool _isLeapYear(int year) => (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;

int _daysInMonth(int year, int month) => switch (month) {
  1 => _isLeapYear(year) ? 29 : 28,
  3 || 5 || 8 || 10 => 30,
  _ => 31,
};

/// Days from 1970-01-01 to a proleptic Gregorian date, as ECMAScript's MakeDay counts them. A
/// day past the month's end carries into the next month, as MakeDay does.
int _daysFromCivil(int year, int monthIndex, int day) {
  final month = monthIndex + 1;
  final y = month <= 2 ? year - 1 : year;
  final era = (y >= 0 ? y : y - 399) ~/ 400;
  final yearOfEra = y - era * 400;
  final dayOfYear = (153 * ((month + 9) % 12) + 2) ~/ 5 + day - 1;
  final dayOfEra = yearOfEra * 365 + yearOfEra ~/ 4 - yearOfEra ~/ 100 + dayOfYear;
  return era * 146097 + dayOfEra - 719468;
}

/// The year, month index and day of a day count, as getUTCFullYear and its siblings give them.
(int, int, int) _civilFromDays(int dayCount) {
  final days = dayCount + 719468;
  final era = (days >= 0 ? days : days - 146096) ~/ 146097;
  final dayOfEra = days - era * 146097;
  final yearOfEra = (dayOfEra - dayOfEra ~/ 1460 + dayOfEra ~/ 36524 - dayOfEra ~/ 146096) ~/ 365;
  final dayOfYear = dayOfEra - (365 * yearOfEra + yearOfEra ~/ 4 - yearOfEra ~/ 100);
  final shiftedMonth = (5 * dayOfYear + 2) ~/ 153;
  final day = dayOfYear - (153 * shiftedMonth + 2) ~/ 5 + 1;
  final month = shiftedMonth < 10 ? shiftedMonth + 2 : shiftedMonth - 10;
  return (yearOfEra + era * 400 + (month <= 1 ? 1 : 0), month, day);
}

/// Ky's createTimestamp: Date.UTC of the parts, with the year set again by setUTCFullYear, and
/// undefined unless every part round-trips. Date.UTC reads a year from 0 to 99 as 1900 + year,
/// so the day has to exist in that year too. Dart's DateTime does neither, so the dates are
/// counted here.
double? _createTimestamp(int year, String month, String day, String hours, String minutes, String seconds) {
  final monthIndex = _months.indexOf(month);
  final (dayNumber, hoursNumber) = (int.parse(day), int.parse(hours));
  final (minutesNumber, secondsNumber) = (int.parse(minutes), int.parse(seconds));
  if (monthIndex == -1 || hoursNumber > 23 || minutesNumber > 59 || secondsNumber > 60) return null;
  final normalizedSeconds = math.min(secondsNumber, 59);
  final utcYear = year >= 0 && year <= 99 ? year + 1900 : year;
  if (dayNumber < 1 ||
      dayNumber > _daysInMonth(utcYear, monthIndex) ||
      dayNumber > _daysInMonth(year, monthIndex)) {
    return null;
  }
  final timestamp = _daysFromCivil(year, monthIndex, dayNumber) * _msPerDay +
      (hoursNumber * 3600000 + minutesNumber * 60000 + normalizedSeconds * 1000);
  // A leap second is represented by parsing 60 as 59 and adding one second.
  return secondsNumber == 60 ? timestamp + 1000 : timestamp;
}

double? _parseDate(String value) {
  if (_imfDatePattern.firstMatch(value) case final date?) {
    return _createTimestamp(int.parse(date[3]!), date[2]!, date[1]!, date[4]!, date[5]!, date[6]!);
  }
  if (_rfc850DatePattern.firstMatch(value) case final date?) {
    final now = _now();
    final today = (now / _msPerDay).floor();
    final (currentYear, currentMonth, currentDay) = _civilFromDays(today);
    final currentCenturyYear = currentYear ~/ 100 * 100 + int.parse(date[3]!);
    final fiftyYearsFromNow =
        _daysFromCivil(currentYear + 50, currentMonth, currentDay) * _msPerDay + (now - today * _msPerDay);
    // RFC 9110 maps two-digit years to the latest matching century that is not more than 50
    // years in the future.
    double? timestamp;
    for (final year in [currentCenturyYear - 100, currentCenturyYear, currentCenturyYear + 100]) {
      final candidate = _createTimestamp(year, date[2]!, date[1]!, date[4]!, date[5]!, date[6]!);
      if (candidate != null && candidate <= fiftyYearsFromNow) timestamp = candidate;
    }
    return timestamp;
  }
  if (_asctimeDatePattern.firstMatch(value) case final date?) {
    final [hours, minutes, seconds] = date[4]!.split(':');
    return _createTimestamp(int.parse(date[5]!), date[2]!, date[3]!.trim(), hours, minutes, seconds);
  }
  return null;
}

double _now() => DateTime.now().millisecondsSinceEpoch.toDouble();

double? calculateRetryTimingDelay(TimingHeader header) {
  if (_delayPattern.hasMatch(header.value)) {
    final seconds = double.parse(header.value);
    // Standard Retry-After numbers are delay seconds. Timestamp compatibility only applies to
    // reset headers.
    final delay = header.allowTimestamp && seconds >= _timestampThresholdSeconds ? seconds * 1000 - _now() : seconds * 1000;
    return math.max(0.0, delay);
  }
  // createTimestamp only returns valid dates, so the delay is always finite.
  final timestamp = _parseDate(header.value);
  return timestamp == null ? null : math.max(0.0, timestamp - _now());
}
