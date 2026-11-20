import 'dart:js_interop';
import 'dart:js_interop_unsafe';

extension type Input._(JSObject _) implements JSObject {
  external JSArray<Command> get cases;
}

extension type Command._(JSObject _) implements JSObject {
  external JSArray<JSNumber> get a;
  external JSArray<JSNumber> get b;
  external String get op;
  external String get s;
  external int get n;
  external int get r;
  external bool get u;
  external bool get v;
}

const mask = 0xffffffff;
const limb = 65536;

// Both words are unsigned JS-safe integers. Products accumulate in 16-bit limbs.
class Word {
  final int lo, hi;
  const Word(this.lo, this.hi);
  static const zero = Word(0, 0);
  static const one = Word(1, 0);
  factory Word.input(JSArray<JSNumber> p) =>
      Word(p[0].toDartInt & mask, p[1].toDartInt & mask);
  bool get isZero => lo == 0 && hi == 0;
  bool get topBit => hi >= 0x80000000;
  bool same(Word b) => lo == b.lo && hi == b.hi;
  bool less(Word b) => hi < b.hi || (hi == b.hi && lo < b.lo);
  Word add(Word b) {
    final low = lo + b.lo;
    return Word(low & mask, (hi + b.hi + (low > mask ? 1 : 0)) & mask);
  }

  Word neg() => Word((-lo) & mask, (-hi - (lo == 0 ? 0 : 1)) & mask);
  Word sub(Word b) => add(b.neg());
  Word and(Word b) => Word(lo & b.lo, hi & b.hi);
  Word or(Word b) => Word(lo | b.lo, hi | b.hi);
  Word xor(Word b) => Word(lo ^ b.lo, hi ^ b.hi);
  Word mul(Word b) {
    final a0 = lo & 65535, a1 = lo >>> 16, a2 = hi & 65535, a3 = hi >>> 16;
    final b0 = b.lo & 65535,
        b1 = b.lo >>> 16,
        b2 = b.hi & 65535,
        b3 = b.hi >>> 16;
    final c0 = a0 * b0;
    final c1 = c0 ~/ limb + a1 * b0 + a0 * b1;
    final c2 = c1 ~/ limb + a2 * b0 + a1 * b1 + a0 * b2;
    final c3 = c2 ~/ limb + a3 * b0 + a2 * b1 + a1 * b2 + a0 * b3;
    return Word(
      (c0 & 65535) | ((c1 & 65535) << 16),
      (c2 & 65535) | ((c3 & 65535) << 16),
    );
  }

  Word shl(int count) {
    final k = count & 63;
    if (k == 0) return this;
    if (k >= 32) return Word(0, (lo << (k - 32)) & mask);
    return Word((lo << k) & mask, ((hi << k) | (lo >>> (32 - k))) & mask);
  }

  Word shr(int count, [bool arithmetic = false]) {
    final k = count & 63;
    if (k == 0) return this;
    final fill = arithmetic && topBit ? mask : 0;
    int highShift(int n) =>
        n == 0 ? hi : (hi >>> n) | ((fill << (32 - n)) & mask);
    if (k >= 32) return Word(highShift(k - 32), fill);
    return Word((lo >>> k) | ((hi << (32 - k)) & mask), highShift(k));
  }

  Word rotl(int k) => shl(k).or(shr(-k));
  int get clz => hi == 0 ? 64 - lo.bitLength : 32 - hi.bitLength;
  int get ctz {
    if (isZero) return 64;
    final w = lo != 0 ? lo : hi;
    return (w & -w).bitLength - 1 + (lo == 0 ? 32 : 0);
  }

  // Base-65536 short division keeps every intermediate below 2^48.
  (Word, int) smallDiv(int d) {
    final a = hi >>> 16;
    final b = (a % d) * limb + (hi & 65535);
    final c = (b % d) * limb + (lo >>> 16);
    final e = (c % d) * limb + (lo & 65535);
    return (
      Word(((c ~/ d) << 16) | (e ~/ d), ((a ~/ d) << 16) | (b ~/ d)),
      e % d,
    );
  }

  (Word, Word) divmod(Word b, bool unsigned) {
    if (b.isZero) throw const FormatException();
    final an = !unsigned && topBit, bn = !unsigned && b.topBit;
    var rem = an ? neg() : this;
    final divisor = bn ? b.neg() : b;
    var q = zero;
    if (divisor.hi == 0) {
      final result = rem.smallDiv(divisor.lo);
      q = result.$1;
      rem = Word(result.$2, 0);
    } else {
      for (var bit = divisor.clz - rem.clz; bit >= 0; bit--) {
        final shift = divisor.shl(bit);
        if (!rem.less(shift)) {
          rem = rem.sub(shift);
          q = q.or(one.shl(bit));
        }
      }
    }
    return (an != bn ? q.neg() : q, an ? rem.neg() : rem);
  }

  Word divide(Word b, bool u) {
    if (!u && hi == 0x80000000 && b.lo == mask && b.hi == mask) return this;
    return divmod(b, u).$1;
  }

  String format(bool unsigned, int r) {
    r = radix(r);
    final negative = !unsigned && topBit;
    var mag = negative ? neg() : this;
    var power = 1;
    for (var i = 0; i < 6; i++) {
      power *= r;
    }
    var result = '';
    do {
      final step = mag.smallDiv(power);
      mag = step.$1;
      final part = step.$2.toRadixString(r);
      result = (mag.isZero ? part : part.padLeft(6, '0')) + result;
    } while (!mag.isZero);
    return negative ? '-' + result : result;
  }
}

int radix(int r) {
  if (r == 0) return 10;
  if (r < 2 || r > 36) throw const FormatException();
  return r;
}

Word parse(String text, int base) {
  if (text.isEmpty) throw const FormatException();
  if (const ['NaN', 'Infinity', '+Infinity', '-Infinity'].contains(text))
    return Word.zero;
  final r = radix(base);
  if (text.startsWith('-')) return parse(text.substring(1), r).neg();
  if (text.contains('-')) throw const FormatException();
  var value = Word.zero;
  final b = Word(r, 0);
  for (var start = 0; start < text.length; start += 8) {
    final end = start + 8 > text.length ? text.length : start + 8;
    var group = text.substring(start, end).trimLeft();
    if (group.startsWith('+')) group = group.substring(1);
    if (r == 16 && group.toLowerCase().startsWith('0x'))
      group = group.substring(2);
    var part = Word.zero;
    for (final code in group.toLowerCase().codeUnits) {
      final d = code >= 48 && code <= 57
          ? code - 48
          : code >= 97 && code <= 122
          ? code - 87
          : 36;
      if (d >= r) break;
      part = part.mul(b).add(Word(d, 0));
    }
    for (var i = start; i < end; i++) {
      value = value.mul(b);
    }
    value = value.add(part);
  }
  return value;
}

int compare(Word a, Word b, bool u, bool v) {
  if (a.same(b) && (u == v || !a.topBit)) return 0;
  final an = !u && a.topBit, bn = !v && b.topBit;
  final less = an != bn
      ? an
      : u
      ? a.less(b)
      : a.sub(b).topBit;
  return less ? -1 : 1;
}

String evaluate(Command c) {
  var a = Word.input(c.a);
  final b = Word.input(c.b), n = c.n;
  switch (c.op) {
    case 'fromBits':
    case 'toString':
      break;
    case 'fromString':
      a = parse(c.s, n);
    case 'add':
      a = a.add(b);
    case 'subtract':
      a = a.sub(b);
    case 'multiply':
      a = a.mul(b);
    case 'divide':
      a = a.divide(b, c.u);
    case 'modulo':
      a = a.divmod(b, c.u).$2;
    case 'and':
      a = a.and(b);
    case 'or':
      a = a.or(b);
    case 'xor':
      a = a.xor(b);
    case 'shiftLeft':
      a = a.shl(n);
    case 'shiftRight':
      a = a.shr(n, true);
    case 'shiftRightUnsigned':
      a = a.shr(n);
    case 'rotl':
      a = a.rotl(n);
    case 'clz':
      return a.clz.toString();
    case 'ctz':
      return a.ctz.toString();
    case 'compare':
      return compare(a, b, c.u, c.v).toString();
    case 'mix':
      var check = 0;
      final factor = b.or(Word.one);
      for (var i = 0; i < n; i++) {
        a = a.mul(factor).add(Word(i + 1, 0)).rotl(11).xor(b);
        a = a.add(a.divide(Word(3 + (i & 255), 0), c.u));
        if ((i & 7) == 0) {
          final s = a.format(c.u, c.r);
          check = (check * 33 + s.length + s.codeUnitAt(0)).toSigned(32);
        }
      }
      return '${a.format(c.u, c.r)}:$check';
    default:
      return '!';
  }
  return a.format(c.u, c.r);
}

void main() {
  final exports = JSObject();
  exports['run'] =
      ((Input input) => input.cases.toDart
              .map((c) {
                try {
                  return evaluate(c);
                } on FormatException {
                  return '!';
                }
              })
              .join('\n')
              .toJS)
          .toJS;
  globalContext['__matrixExports'] = exports;
}
