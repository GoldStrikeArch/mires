// Decimal coefficients contain normalized base-ten digits, most significant first.
import 'dart:math' as math;

class Decimal {
  final int s, e;
  final List<int> c;
  Decimal(this.s, this.e, this.c);
}

Decimal dec(int s, int e, List<int> c) => Decimal(s, e, c);
int size(List<int> a) => a.length;
int slen(String s) => s.length;
List<int> filled(int n, int d) => List.filled(n, d);
List<int> cut(List<int> a, int start, int stop) => a.sublist(start, stop);
List<int> extend(List<int> a, int d) => [...a, d];
int code(String s, int i) => s.codeUnitAt(i);
String letter(int n) => String.fromCharCode(n);
String strint(int n) => n.toString();
String cat(String a, String b) => a + b;
Never error(String msg) => throw FormatException(msg);
double machineRoot(String s) => math.sqrt(double.parse(s));
bool finite(double f) => f.isFinite;
String floatText(double f) => f.toString();
int max(int a, int b) => math.max(a, b);
int min(int a, int b) => math.min(a, b);
int halvedExponent(int e) => (e / 2).floor();

Decimal normalize(int sign, int exponent, List<int> digits) {
  int start = 0;
  while (((start < size(digits)) && (digits[start] == 0))) {
    start = start + 1;
  }
  int stop = size(digits);
  while (((stop > start) && (digits[(stop - 1)] == 0))) {
    stop = stop - 1;
  }
  Decimal out = dec(sign, 0, filled(1, 0));
  if ((stop > start)) {
    out = dec(sign, (exponent - start), cut(digits, start, stop));
  }
  return out;
}

Decimal parse(String text) {
  int length = slen(text);
  int pos = 0;
  int sign = 1;
  if (((pos < length) && (code(text, pos) == 45))) {
    sign = (-1);
    pos = pos + 1;
  }
  List<int> digits = filled(length, 0);
  int count = 0;
  int places = 0;
  while (((pos < length) &&
      (code(text, pos) >= 48) &&
      (code(text, pos) <= 57))) {
    digits[count] = (code(text, pos) - 48);
    count = count + 1;
    pos = pos + 1;
    places = places + 1;
  }
  if (((pos < length) && (code(text, pos) == 46))) {
    pos = pos + 1;
    while (((pos < length) &&
        (code(text, pos) >= 48) &&
        (code(text, pos) <= 57))) {
      digits[count] = (code(text, pos) - 48);
      count = count + 1;
      pos = pos + 1;
    }
  }
  if ((count == 0)) {
    error("[big.js] Invalid number");
  }
  if (((pos < length) &&
      ((code(text, pos) == 69) || (code(text, pos) == 101)))) {
    pos = pos + 1;
    int factor = 1;
    if (((pos < length) &&
        ((code(text, pos) == 45) || (code(text, pos) == 43)))) {
      factor = ((code(text, pos) == 45) ? (-1) : 1);
      pos = pos + 1;
    }
    int start = pos;
    int exponent = 0;
    while (((pos < length) &&
        (code(text, pos) >= 48) &&
        (code(text, pos) <= 57))) {
      exponent = (((exponent * 10) + code(text, pos)) - 48);
      pos = pos + 1;
    }
    if ((pos == start)) {
      error("[big.js] Invalid number");
    }
    places = places + (factor * exponent);
  }
  if ((pos != length)) {
    error("[big.js] Invalid number");
  }
  return normalize(sign, (places - 1), cut(digits, 0, count));
}

int digit(Decimal x, int exponential) {
  int idx = (x.e - exponential);
  return (((idx >= 0) && (idx < size(x.c))) ? x.c[idx] : 0);
}

int cmp(Decimal x, Decimal y) {
  int result = 0;
  if ((x.c[0] == 0)) {
    result = ((y.c[0] == 0) ? 0 : (-y.s));
  } else {
    if (((y.c[0] == 0) || (x.s != y.s))) {
      result = x.s;
    } else {
      if ((x.e != y.e)) {
        result = ((x.e > y.e) ? x.s : (-x.s));
      } else {
        int i = 0;
        int limit = min(size(x.c), size(y.c));
        while (((i < limit) && (x.c[i] == y.c[i]))) {
          i = i + 1;
        }
        if ((i < limit)) {
          result = ((x.c[i] > y.c[i]) ? x.s : (-x.s));
        } else {
          if ((size(x.c) != size(y.c))) {
            result = ((size(x.c) > size(y.c)) ? x.s : (-x.s));
          }
        }
      }
    }
  }
  return result;
}

Decimal add(Decimal x, Decimal y, bool minus) {
  int sign = (minus ? (-y.s) : y.s);
  Decimal result = dec(sign, y.e, y.c);
  if ((x.c[0] == 0)) {
    if (((y.c[0] == 0) && (sign != x.s))) {
      result = dec(1, 0, filled(1, 0));
    }
  } else {
    if ((y.c[0] == 0)) {
      result = x;
    } else {
      bool same = (x.s == sign);
      bool reverse = ((!same) && ((cmp(x, dec(x.s, y.e, y.c)) * x.s) < 0));
      int top = max(x.e, y.e);
      int low = min(((x.e - size(x.c)) + 1), ((y.e - size(y.c)) + 1));
      List<int> digits = filled(((top - low) + 2), 0);
      int carry = 0;
      int p = low;
      while ((p <= top)) {
        int a = digit(x, p);
        int b = digit(y, p);
        int total = (carry + (same ? (a + b) : (reverse ? (b - a) : (a - b))));
        carry = ((total < 0) ? (-1) : (total ~/ 10));
        digits[((top - p) + 1)] = (total - (carry * 10));
        p = p + 1;
      }
      digits[0] = carry;
      result = normalize((reverse ? (-x.s) : x.s), (top + 1), digits);
      if ((result.c[0] == 0)) {
        result = dec(1, 0, result.c);
      }
    }
  }
  return result;
}

Decimal times(Decimal x, Decimal y) {
  List<int> digits = filled((size(x.c) + size(y.c)), 0);
  int i = (size(x.c) - 1);
  while ((i >= 0)) {
    int carry = 0;
    int j = (size(y.c) - 1);
    while ((j >= 0)) {
      int k = ((i + j) + 1);
      int v = (((x.c[i] * y.c[j]) + digits[k]) + carry);
      digits[k] = (v % 10);
      carry = (v ~/ 10);
      j = j - 1;
    }
    digits[i] = carry;
    i = i - 1;
  }
  return normalize((x.s * y.s), ((x.e + y.e) + 1), digits);
}

Decimal rounded(Decimal x, int sd, int mode, bool more) {
  if (((mode < 0) || (mode > 3))) {
    error("[big.js] Invalid rounding mode");
  }
  Decimal result = x;
  if ((sd < size(x.c))) {
    int guardDigit = ((sd >= 0) ? x.c[sd] : 0);
    bool odd = ((sd > 0) && ((x.c[(sd - 1)] % 2) == 1));
    bool sticky = (((sd + 1) < size(x.c)) || more);
    bool up =
        (((mode == 1) && (guardDigit >= 5)) ||
        ((mode == 2) &&
            ((guardDigit > 5) || ((guardDigit == 5) && (odd || sticky)))) ||
        ((mode == 3) && ((x.c[0] != 0) || more)));
    if ((sd <= 0)) {
      result = (up
          ? dec(x.s, ((x.e - sd) + 1), filled(1, 1))
          : dec(x.s, 0, filled(1, 0)));
    } else {
      List<int> digits = cut(x.c, 0, sd);
      int pos = (sd - 1);
      if (up) {
        while (((pos >= 0) && (digits[pos] == 9))) {
          digits[pos] = 0;
          pos = pos - 1;
        }
        if ((pos >= 0)) {
          digits[pos] = (digits[pos] + 1);
        }
      }
      result = ((pos < 0)
          ? dec(x.s, (x.e + 1), filled(1, 1))
          : normalize(x.s, x.e, digits));
    }
  }
  return result;
}

int natural(List<int> a, List<int> b) {
  int result = 0;
  if ((size(a) != size(b))) {
    result = ((size(a) > size(b)) ? 1 : (-1));
  } else {
    int i = 0;
    while (((i < size(a)) && (a[i] == b[i]))) {
      i = i + 1;
    }
    if ((i < size(a))) {
      result = ((a[i] > b[i]) ? 1 : (-1));
    }
  }
  return result;
}

Decimal divide(Decimal x, Decimal y, int dp, int mode) {
  if (((dp < 0) || (dp > 1000000))) {
    error("[big.js] Invalid decimal places");
  }
  if ((y.c[0] == 0)) {
    error("[big.js] Division by zero");
  }
  int sign = (x.s * y.s);
  Decimal result = dec(sign, 0, filled(1, 0));
  if ((x.c[0] != 0)) {
    List<int> rest = filled(size(y.c), 0);
    int pos = 0;
    while ((pos < size(y.c))) {
      rest[pos] = ((pos < size(x.c)) ? x.c[pos] : 0);
      pos = pos + 1;
    }
    int exponent = (x.e - y.e);
    final digits = <int>[];
    int count = 0;
    bool complete = false;
    while ((!complete)) {
      int quotient = 0;
      while ((natural(rest, y.c) >= 0)) {
        int shift = (size(rest) - size(y.c));
        int borrow = 0;
        int k = (size(rest) - 1);
        while ((k >= 0)) {
          int dv = ((k >= shift) ? y.c[(k - shift)] : 0);
          int v = ((rest[k] - dv) - borrow);
          borrow = ((v < 0) ? 1 : 0);
          rest[k] = (v + (borrow * 10));
          k = k - 1;
        }
        int start = 0;
        while (((start < size(rest)) && (rest[start] == 0))) {
          start = start + 1;
        }
        if ((start > 0)) {
          rest = cut(rest, start, size(rest));
        }
        quotient = quotient + 1;
      }
      if (((quotient == 0) && (count == 0))) {
        exponent = exponent - 1;
      } else {
        digits.add(quotient);
        count = count + 1;
      }
      bool more = ((size(rest) > 0) || (pos < size(x.c)));
      complete = ((count > 0) && ((!more) || (count > ((dp + exponent) + 1))));
      if (complete) {
        result = dec(sign, exponent, cut(digits, 0, count));
        if ((count > ((dp + exponent) + 1))) {
          result = rounded(result, ((dp + exponent) + 1), mode, more);
        }
      } else {
        int nxt = ((pos < size(x.c)) ? x.c[pos] : 0);
        if (((size(rest) > 0) || (nxt != 0))) {
          rest = extend(rest, nxt);
        }
        pos = pos + 1;
      }
    }
  }
  return result;
}

Decimal power(Decimal x, int n, int dp, int mode) {
  if (((n < (-1000000)) || (n > 1000000))) {
    error("[big.js] Invalid exponent");
  }
  Decimal one = dec(1, 0, filled(1, 1));
  Decimal result = one;
  Decimal base = x;
  int i = ((n < 0) ? (-n) : n);
  while ((i > 0)) {
    if (((i % 2) == 1)) {
      result = times(result, base);
    }
    i = (i ~/ 2);
    if ((i > 0)) {
      base = times(base, base);
    }
  }
  if ((n < 0)) {
    result = divide(one, result, dp, mode);
  }
  return result;
}

String render(Decimal x, String op, int n, int mode) {
  Decimal v = x;
  if ((n != (-2000002))) {
    int sd = n;
    if ((op == "toPrecision")) {
      if (((n < 1) || (n > 1000000))) {
        error("[big.js] Invalid precision");
      }
    } else {
      if (((n < 0) || (n > 1000000))) {
        error("[big.js] Invalid decimal places");
      }
      sd = ((op == "toFixed") ? ((n + v.e) + 1) : (n + 1));
    }
    v = rounded(v, sd, mode, false);
    int width = ((op == "toFixed") ? ((n + v.e) + 1) : sd);
    if ((width > size(v.c))) {
      List<int> padded = filled(width, 0);
      int i = 0;
      while ((i < size(v.c))) {
        padded[i] = v.c[i];
        i = i + 1;
      }
      v = dec(v.s, v.e, padded);
    }
  }
  bool exponential =
      ((op == "toExponential") ||
      ((op != "toFixed") &&
          ((v.e <= (-7)) ||
              (v.e >= 21) ||
              ((op == "toPrecision") && (n != (-2000002)) && (n <= v.e)))));
  String text = (((x.s < 0) && (x.c[0] != 0)) ? "-" : "");
  if (exponential) {
    text = cat(text, letter((48 + v.c[0])));
    if ((size(v.c) > 1)) {
      text = cat(text, ".");
      int j = 1;
      while ((j < size(v.c))) {
        text = cat(text, letter((48 + v.c[j])));
        j = j + 1;
      }
    }
    text = cat(text, ((v.e >= 0) ? "e+" : "e"));
    text = cat(text, strint(v.e));
  } else {
    int p = max(0, v.e);
    while ((p >= 0)) {
      text = cat(text, letter((48 + digit(v, p))));
      p = p - 1;
    }
    int last = ((v.e - size(v.c)) + 1);
    if ((last < 0)) {
      text = cat(text, ".");
      p = (-1);
      while ((p >= last)) {
        text = cat(text, letter((48 + digit(v, p))));
        p = p - 1;
      }
    }
  }
  return text;
}

Decimal root(Decimal x, int dp, int mode) {
  // The root is below a tenth of the last requested decimal place.
  if (x.s > 0 && dp >= 0 && dp <= 999996 && mode >= 0 && mode <= 3 && x.e < -2 * dp - 2) {
    return mode == 3 ? dec(1, -dp, [1]) : dec(1, 0, [0]);
  }
  Decimal result = x;
  if ((x.c[0] != 0)) {
    if ((x.s < 0)) {
      error("[big.js] No square root");
    }
    double estimate = machineRoot(render(x, "toExponential", (-2000002), mode));
    if (((estimate > 0.0) && finite(estimate))) {
      result = parse(floatText(estimate));
    } else {
      String coefficient = "";
      int i = 0;
      while ((i < size(x.c))) {
        coefficient = cat(coefficient, letter((48 + x.c[i])));
        i = i + 1;
      }
      if ((((size(x.c) + x.e) % 2) == 0)) {
        coefficient = cat(coefficient, "0");
      }
      double fallback = machineRoot(coefficient);
      Decimal leading = (finite(fallback)
          ? parse(floatText(fallback))
          : dec(1, 0, filled(1, 5)));
      result = dec(1, halvedExponent(x.e), leading.c);
    }
    Decimal half = dec(1, (-1), filled(1, 5));
    int work = max((dp + 4), (5 - result.e));
    int count = max(1, (result.e + work));
    bool matched = false;
    while ((!matched)) {
      Decimal nxt = times(
        half,
        add(result, divide(x, result, work, mode), false),
      );
      matched = (result.e == nxt.e);
      int length = min(count, max(size(result.c), size(nxt.c)));
      int j = 0;
      while ((matched && (j < length))) {
        matched =
            ((j < size(result.c)) &&
            (j < size(nxt.c)) &&
            (result.c[j] == nxt.c[j]));
        j = j + 1;
      }
      result = nxt;
    }
    result = rounded(result, ((dp + result.e) + 1), mode, false);
  }
  return result;
}

String operate(
  String op,
  String xText,
  String yText,
  int n,
  int dp,
  int rm,
  int mode,
) {
  Decimal x = parse(xText);
  Decimal result = x;
  String out = "";
  if (((op == "plus") ||
      (op == "minus") ||
      (op == "times") ||
      (op == "div") ||
      (op == "cmp"))) {
    Decimal y = parse(yText);
    if ((op == "cmp")) {
      out = strint(cmp(x, y));
    } else {
      if ((op == "times")) {
        result = times(x, y);
      } else {
        if ((op == "div")) {
          result = divide(x, y, dp, rm);
        } else {
          result = add(x, y, (op == "minus"));
        }
      }
    }
  } else {
    if ((op == "round")) {
      int places = ((n == (-2000002)) ? 0 : n);
      if (((places < (-1000000)) || (places > 1000000))) {
        error("[big.js] Invalid decimal places");
      }
      result = rounded(x, ((places + x.e) + 1), mode, false);
    } else {
      if ((op == "pow")) {
        result = power(x, n, dp, rm);
      } else {
        if ((op == "sqrt")) {
          result = root(x, dp, rm);
        } else {
          if (((op == "toString") ||
              (op == "toExponential") ||
              (op == "toFixed") ||
              (op == "toPrecision"))) {
            out = render(x, op, ((op == "toString") ? (-2000002) : n), mode);
          } else {
            error("unknown decimal operation");
          }
        }
      }
    }
  }
  if ((out == "")) {
    out = render(result, "toString", (-2000002), mode);
  }
  return out;
}
