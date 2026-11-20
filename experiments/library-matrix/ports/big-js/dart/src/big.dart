// big.js 7.0.1 (big.mjs): the decimal type, its parser, `round`, `stringify` and the methods the
// contract uses, function by function and loop by loop. A coefficient is a growable List<int> of
// digits, most significant first, mutated where upstream mutates its array. The exponent is a
// double because upstream's is a JS number: past int32 and 2^53 it rounds as a double does, and
// Infinity and NaN exponents keep upstream's comparison chains.
import 'dart:math' as math;

const _name = '[big.js] ';
const _invalid = '${_name}Invalid ';
const _invalidDp = '${_invalid}decimal places';
const _invalidRm = '${_invalid}rounding mode';
const _divByZero = '${_name}Division by zero';
const _maxDp = 1000000;
const _maxPower = 1000000;
const _ne = -7;
const _pe = 21;
final _numeric = RegExp(r'^-?(\d+(\.\d*)?|\.\d+)(e[+-]?\d+)?$', caseSensitive: false);

class BigError implements Exception {
  final String message;
  const BigError(this.message);
}

// Upstream's `n !== ~~n || n < lo || n > hi`: an integer within the range (all ranges lie in int32).
bool _isIntIn(double n, int lo, int hi) => n >= lo && n <= hi && n == n.truncateToDouble();

// JS truthiness of a number, for upstream's `if (a = xe - ye)`.
bool _truthy(double n) => n != 0 && !n.isNaN;

// ECMAScript ToInt32, for upstream's `| 0` and `&` on exponents. Dart's % is the spec's modulo.
int _toInt32(double v) {
  if (!v.isFinite) return 0;
  final m = v.truncateToDouble() % 4294967296;
  return (m >= 2147483648 ? m - 4294967296 : m).toInt();
}

// JS `Number(s)` for the strings upstream passes it: NaN where it is not a number.
double _number(String s) => double.tryParse(s) ?? double.nan;

// `Array.prototype.slice(0, end)`, where a negative end counts from the back.
List<int> _head(List<int> c, double end) =>
    c.sublist(0, (end < 0 ? math.max(c.length + end, 0) : math.min(end, c.length)).toInt());

bool _same(List<int> a, List<int> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

class Big {
  int s;
  double e;
  List<int> c;

  Big(this.s, this.e, this.c);

  Big copy() => Big(s, e, List.of(c));

  // Upstream `parse`: the decimal string n, validated against NUMERIC.
  factory Big.parse(String source) {
    if (!_numeric.hasMatch(source)) throw const BigError('${_invalid}number');
    // Determine sign.
    final s = source.startsWith('-') ? -1 : 1;
    var n = s < 0 ? source.substring(1) : source;
    // Decimal point?
    final dot = n.indexOf('.');
    if (dot > -1) n = n.replaceFirst('.', '');
    // Exponential form?
    final i = n.toLowerCase().indexOf('e');
    final double e;
    if (i > 0) {
      // Determine exponent.
      e = (dot < 0 ? i : dot) + double.parse(n.substring(i + 1));
      n = n.substring(0, i);
    } else {
      // Integer.
      e = (dot < 0 ? n.length : dot).toDouble();
    }
    // Determine leading zeros.
    var lead = 0;
    while (lead < n.length && n.codeUnitAt(lead) == 0x30) {
      lead++;
    }
    // Zero.
    if (lead == n.length) return Big(s, 0, [0]);
    // Determine trailing zeros.
    var last = n.length - 1;
    while (n.codeUnitAt(last) == 0x30) {
      last--;
    }
    // Convert string to array of digits without leading/trailing zeros.
    return Big(s, e - lead - 1, [for (var k = lead; k <= last; k++) n.codeUnitAt(k) - 0x30]);
  }

  // Return 1, -1 or 0 as this is greater than, less than or equal to y.
  int cmp(Big y) {
    final xc = c, yc = y.c;
    final i = s, j = y.s;
    // Either zero?
    if (xc[0] == 0 || yc[0] == 0) return xc[0] == 0 ? (yc[0] == 0 ? 0 : -j) : i;
    // Signs differ?
    if (i != j) return i;
    final isneg = i < 0;
    // Compare exponents.
    if (e != y.e) return (e > y.e) != isneg ? 1 : -1;
    // Compare digit by digit.
    final n = math.min(xc.length, yc.length);
    for (var k = 0; k < n; k++) {
      if (xc[k] != yc[k]) return (xc[k] > yc[k]) != isneg ? 1 : -1;
    }
    // Compare lengths.
    return xc.length == yc.length ? 0 : ((xc.length > yc.length) != isneg ? 1 : -1);
  }

  // This divided by y, rounded if necessary to dp decimal places using rounding mode rm.
  Big div(Big y, double dp, double rm) {
    final a = c; // dividend
    final b = y.c; // divisor
    final sign = s == y.s ? 1 : -1;
    if (!_isIntIn(dp, 0, _maxDp)) throw const BigError(_invalidDp);
    // Divisor is zero?
    if (b[0] == 0) throw const BigError(_divByZero);
    // Dividend is 0? Return +-0.
    if (a[0] == 0) return Big(sign, 0, [0]);

    final bl = b.length, al = a.length;
    var ai = bl;
    final bz = [0, ...b]; // divisor with a leading zero
    var r = a.sublist(0, math.min(bl, al)); // remainder
    final qc = <int>[]; // quotient
    var qe = e - y.e;
    var p = dp + qe + 1; // precision of the result
    var k = p < 0 ? 0.0 : p;

    // Add zeros to make remainder as long as divisor.
    while (r.length < bl) {
      r.add(0);
    }

    var rl = 0, cmp = 0;
    do {
      // n is how many times the divisor goes into current remainder.
      var n = 0;
      for (; n < 10; n++) {
        // Compare divisor and remainder.
        rl = r.length;
        if (bl != rl) {
          cmp = bl > rl ? 1 : -1;
        } else {
          cmp = 0;
          for (var ri = 0; ri < bl; ri++) {
            if (b[ri] != r[ri]) {
              cmp = b[ri] > r[ri] ? 1 : -1;
              break;
            }
          }
        }
        // If divisor < remainder, subtract divisor from remainder.
        if (cmp >= 0) break;
        // Remainder can't be more than 1 digit longer than divisor.
        // Equalise lengths using divisor with extra leading zero?
        final bt = rl == bl ? b : bz;
        while (rl > 0) {
          if (r[--rl] < bt[rl]) {
            var ri = rl;
            while (ri > 0 && r[--ri] == 0) {
              r[ri] = 9;
            }
            r[ri]--;
            r[rl] += 10;
          }
          r[rl] -= bt[rl];
        }
        while (r[0] == 0) {
          r.removeAt(0);
        }
      }
      // Add the digit n to the result array.
      qc.add(cmp != 0 ? n : n + 1);
      // Update the remainder. Upstream's [undefined], the dividend's end, is the empty list.
      if (r.isNotEmpty && r[0] != 0 && cmp != 0) {
        r.add(ai < al ? a[ai] : 0);
      } else {
        r = ai < al ? [a[ai]] : [];
      }
      // `k--` is falsy at 0 and at NaN, as `k-- > 0` is.
    } while ((ai++ < al || r.isNotEmpty) && k-- > 0);

    // Leading zero? Do not remove if result is simply zero (qi == 1).
    final qi = qc.length;
    if (qc[0] == 0 && qi != 1) {
      // There can't be more than one zero.
      qc.removeAt(0);
      qe--;
      p--;
    }
    final q = Big(sign, qe, qc);
    // Round?
    if (qi > p) _round(q, p, rm, r.isNotEmpty);
    return q;
  }

  // This minus y.
  Big minus(Big that) {
    var y = that.copy();
    final b = y.s;
    // Signs differ?
    if (s != b) {
      y.s = -b;
      return plus(y);
    }
    var xc = List.of(c), yc = y.c;
    final xe = e;
    var ye = y.e;
    // Either zero?
    if (xc[0] == 0 || yc[0] == 0) {
      if (yc[0] != 0) {
        y.s = -b;
      } else if (xc[0] != 0) {
        y = copy();
      } else {
        y.s = 1;
      }
      return y;
    }
    // Determine which is the bigger number. Prepend zeros to equalise exponents.
    var a = xe - ye;
    bool xlty;
    if (_truthy(a)) {
      xlty = a < 0;
      final List<int> t;
      if (xlty) {
        a = -a;
        t = xc;
      } else {
        ye = xe;
        t = yc;
      }
      t.insertAll(0, List.filled(a.toInt(), 0));
    } else {
      // Exponents equal. Check digit by digit.
      xlty = xc.length < yc.length;
      final j = math.min(xc.length, yc.length);
      for (var i = 0; i < j; i++) {
        if (xc[i] != yc[i]) {
          xlty = xc[i] < yc[i];
          break;
        }
      }
      a = 0;
    }
    // x < y? Point xc to the array of the bigger number.
    if (xlty) {
      final t = xc;
      xc = yc;
      yc = t;
      y.s = -y.s;
    }
    // Append zeros to xc if shorter. No need to add zeros to yc if shorter as subtraction only
    // needs to start at yc.length.
    while (xc.length < yc.length) {
      xc.add(0);
    }
    // Subtract yc from xc.
    for (var j = yc.length; j > a;) {
      if (xc[--j] < yc[j]) {
        var i = j;
        while (i > 0 && xc[--i] == 0) {
          xc[i] = 9;
        }
        xc[i]--;
        xc[j] += 10;
      }
      xc[j] -= yc[j];
    }
    // Remove trailing zeros.
    while (xc.isNotEmpty && xc.last == 0) {
      xc.removeLast();
    }
    // Remove leading zeros and adjust exponent accordingly.
    while (xc.isNotEmpty && xc.first == 0) {
      xc.removeAt(0);
      --ye;
    }
    if (xc.isEmpty) {
      // n - n = +0
      y.s = 1;
      // Result must be zero.
      xc = [0];
      ye = 0;
    }
    y.c = xc;
    y.e = ye;
    return y;
  }

  // This plus y.
  Big plus(Big that) {
    final y = that.copy();
    // Signs differ?
    if (s != y.s) {
      y.s = -y.s;
      return minus(y);
    }
    var xc = c, yc = y.c;
    var ye = y.e;
    // Either zero?
    if (xc[0] == 0 || yc[0] == 0) {
      if (yc[0] == 0) {
        if (xc[0] != 0) return copy();
        y.s = s;
      }
      return y;
    }
    xc = List.of(xc);
    // Prepend zeros to equalise exponents.
    var d = e - ye;
    if (_truthy(d)) {
      final List<int> t;
      if (d > 0) {
        ye = e;
        t = yc;
      } else {
        d = -d;
        t = xc;
      }
      t.insertAll(0, List.filled(d.toInt(), 0));
    }
    // Point xc to the longer array.
    if (xc.length - yc.length < 0) {
      final t = yc;
      yc = xc;
      xc = t;
    }
    // Only start adding at yc.length - 1 as the further digits of xc can be left as they are.
    var k = 0;
    for (var i = yc.length - 1; i >= 0; i--) {
      xc[i] += yc[i] + k;
      k = xc[i] ~/ 10;
      xc[i] %= 10;
    }
    // No need to check for zero, as +x + +y != 0 && -x + -y != 0
    if (k != 0) {
      xc.insert(0, k);
      ++ye;
    }
    // Remove trailing zeros.
    while (xc.last == 0) {
      xc.removeLast();
    }
    y.c = xc;
    y.e = ye;
    return y;
  }

  // This raised to the integer power n; a negative power divides with dp and rm.
  Big pow(double? n, double dp, double rm) {
    var x = this;
    final one = Big(1, 0, [1]);
    var y = one;
    if (n == null || !_isIntIn(n, -_maxPower, _maxPower)) throw const BigError('${_invalid}exponent');
    final isneg = n < 0;
    var k = n.abs().toInt();
    for (;;) {
      if (k & 1 != 0) y = y.times(x);
      k >>= 1;
      if (k == 0) break;
      x = x.times(x);
    }
    return isneg ? one.div(y, dp, rm) : y;
  }

  // This rounded to dp decimal places (0 when absent) using rounding mode rm.
  Big round(double? dp, double rm) {
    if (dp != null && !_isIntIn(dp, -_maxDp, _maxDp)) throw const BigError(_invalidDp);
    return _round(copy(), (dp ?? 0) + e + 1, rm);
  }

  // The square root of this, rounded to dp decimal places using rounding mode rm.
  Big sqrt(double dp, double rm) {
    final half = Big(1, -1, [5]);
    // Zero?
    if (c[0] == 0) return copy();
    // Negative?
    if (s < 0) throw const BigError('${_name}No square root');
    // Estimate.
    var estimate = math.sqrt(_number(_stringify(this, true, true)));
    Big r;
    // Math.sqrt underflow/overflow?
    // Re-estimate: pass x coefficient to Math.sqrt as integer, then adjust the result exponent.
    if (estimate == 0 || estimate == double.infinity) {
      var digits = c.join();
      if (_toInt32(digits.length + e) & 1 == 0) digits += '0';
      estimate = math.sqrt(_number(digits));
      final exponent = _toInt32((e + 1) / 2) - (e < 0 || _toInt32(e) & 1 != 0 ? 1 : 0);
      final mantissa = estimate == double.infinity ? '5e' : _mantissa(estimate);
      r = Big.parse('$mantissa$exponent');
    } else {
      r = Big.parse(estimate.toString());
    }
    // Big.DP += 4 for the iteration; div validates it.
    final places = dp + 4;
    final compared = r.e + places;
    // Newton-Raphson iteration.
    Big t;
    do {
      t = r;
      r = half.times(t.plus(div(t, places, rm)));
    } while (!_same(_head(t.c, compared), _head(r.c, compared)));
    return _round(r, dp + r.e + 1, rm);
  }

  // This times y.
  Big times(Big that) {
    var xc = this.c, yc = that.c;
    var a = xc.length, b = yc.length;
    // Determine sign of result.
    final sign = s == that.s ? 1 : -1;
    // Return signed 0 if either 0.
    if (xc[0] == 0 || yc[0] == 0) return Big(sign, 0, [0]);
    // Initialise exponent of result as x.e + y.e.
    var e = this.e + that.e;
    // If array xc has fewer digits than yc, swap xc and yc, and lengths.
    if (a < b) {
      (xc, yc) = (yc, xc);
      (a, b) = (b, a);
    }
    // Initialise coefficient array of result with zeros.
    final c = List.filled(a + b, 0, growable: true);
    // Multiply.
    var carry = 0;
    for (var i = b - 1; i >= 0; i--) {
      carry = 0;
      var j = a + i;
      while (j > i) {
        // Current sum of products at this digit position, plus carry.
        carry += c[j] + yc[i] * xc[j - i - 1];
        c[j--] = carry % 10;
        carry ~/= 10;
      }
      c[j] = carry;
    }
    // Increment result exponent if there is a final carry, otherwise remove leading zero.
    if (carry != 0) {
      ++e;
    } else {
      c.removeAt(0);
    }
    // Remove trailing zeros.
    while (c.last == 0) {
      c.removeLast();
    }
    return Big(sign, e, c);
  }

  // Exponential notation with dp fixed decimal places, rounded using rounding mode rm.
  String toExponential(double? dp, double rm) {
    var x = this;
    final n = c[0];
    if (dp != null) {
      if (!_isIntIn(dp, 0, _maxDp)) throw const BigError(_invalidDp);
      final sd = dp + 1;
      x = _round(copy(), sd, rm);
      while (x.c.length < sd) {
        x.c.add(0);
      }
    }
    return _stringify(x, true, n != 0);
  }

  // Normal notation with dp fixed decimal places, rounded using rounding mode rm.
  String toFixed(double? dp, double rm) {
    var x = this;
    final n = c[0];
    if (dp != null) {
      if (!_isIntIn(dp, 0, _maxDp)) throw const BigError(_invalidDp);
      x = _round(copy(), dp + e + 1, rm);
      // x.e may have changed if the value is rounded up.
      final width = dp + x.e + 1;
      while (x.c.length < width) {
        x.c.add(0);
      }
    }
    return _stringify(x, false, n != 0);
  }

  // sd significant digits, rounded using rounding mode rm, in exponential notation if sd is less
  // than the number of digits of the integer part.
  String toPrecision(double? sd, double rm) {
    var x = this;
    final n = c[0];
    if (sd != null) {
      if (!_isIntIn(sd, 1, _maxDp)) throw const BigError('${_invalid}precision');
      x = _round(copy(), sd, rm);
      while (x.c.length < sd) {
        x.c.add(0);
      }
    }
    return _stringify(x, sd != null && sd <= x.e || x.e <= _ne || x.e >= _pe, n != 0);
  }

  // Exponential notation at exponents of NE and below or PE and above; no sign for zero.
  @override
  String toString() => _stringify(this, e <= _ne || e >= _pe, c[0] != 0);
}

// JS `toExponential()` up to and including its `e`: the shortest digits of s in d.ddd form.
String _mantissa(double s) {
  final text = s.toStringAsExponential();
  return text.substring(0, text.indexOf('e') + 1);
}

// Upstream `round`: x rounded in place to sd significant digits using rounding mode rm. `more`
// says whether a division was truncated.
Big _round(Big x, double sd, double rm, [bool more = false]) {
  final xc = x.c;
  if (rm != 0 && rm != 1 && rm != 2 && rm != 3) throw const BigError(_invalidRm);
  if (sd < 1) {
    final up = rm == 3 && (more || xc[0] != 0) ||
        sd == 0 && (rm == 1 && xc[0] >= 5 || rm == 2 && (xc[0] > 5 || xc[0] == 5 && (more || xc.length > 1)));
    xc.length = 1;
    if (up) {
      // 1, 0.1, 0.01, 0.001, 0.0001 etc.
      x.e = x.e - sd + 1;
      xc[0] = 1;
    } else {
      // Zero.
      xc[0] = 0;
      x.e = 0;
    }
  } else if (sd < xc.length) {
    var i = sd.toInt();
    // xc[i] is the digit after the digit that may be rounded up.
    final up = rm == 1 && xc[i] >= 5 ||
        rm == 2 && (xc[i] > 5 || xc[i] == 5 && (more || i + 1 < xc.length || xc[i - 1].isOdd)) ||
        rm == 3 && (more || xc[0] != 0);
    // Remove any digits after the required precision.
    xc.length = i;
    // Round up?
    if (up) {
      // Rounding up may mean the previous digit has to be rounded up.
      while (++xc[--i] > 9) {
        xc[i] = 0;
        if (i == 0) {
          ++x.e;
          xc.insert(0, 1);
          break;
        }
      }
    }
    // Remove trailing zeros.
    while (xc.last == 0) {
      xc.removeLast();
    }
  }
  return x;
}

// Upstream `stringify`: x in normal or exponential notation.
String _stringify(Big x, bool doExponential, bool isNonzero) {
  final e = x.e;
  var s = x.c.join();
  final n = s.length;
  // Exponential notation?
  if (doExponential) {
    s = '${s[0]}${n > 1 ? '.${s.substring(1)}' : ''}${e < 0 ? 'e' : 'e+'}$e';
  } else if (e < 0) {
    // Normal notation.
    s = '0.${'0' * (-e - 1).toInt()}$s';
  } else if (e > 0) {
    final point = e.toInt() + 1;
    if (point > n) {
      s += '0' * (point - n);
    } else if (point < n) {
      s = '${s.substring(0, point)}.${s.substring(point)}';
    }
  } else if (n > 1) {
    s = '${s[0]}.${s.substring(1)}';
  }
  return x.s < 0 && isNonzero ? '-$s' : s;
}
