/// noble's _u64.ts: a 64-bit word is a high and a low 32-bit half, each an int in
/// 0..0xffffffff (the values noble's Uint32Arrays hold).
///
/// The sums are noble's: the low halves add up exactly, since an int holds the sum on the web
/// and on the VM, and the carry into the high half is that sum divided by 2^32.
library;

/// The 32-bit mask every shifted or summed half is reduced by.
const mask32 = 0xffffffff;

const _pow32 = 0x100000000;

/// noble's fromNumH: the high half of an integer in 0..2^53.
int fromNumH(int n) => n ~/ _pow32;

/// noble's fromNumL (`n >>> 0`): the low half of an integer in 0..2^53.
int fromNumL(int n) => n & mask32;

/// High half of a 64-bit logical right shift by [s] in 0..31.
int shrSH(int h, int l, int s) => h >> s;

/// Low half of a 64-bit logical right shift by [s] in 1..31.
int shrSL(int h, int l, int s) => (h << (32 - s) | l >> s) & mask32;

/// High half of a 64-bit right rotation by [s] in 1..31.
int rotrSH(int h, int l, int s) => (h >> s | l << (32 - s)) & mask32;

/// Low half of a 64-bit right rotation by [s] in 1..31.
int rotrSL(int h, int l, int s) => (h << (32 - s) | l >> s) & mask32;

/// High half of a 64-bit right rotation by [s] in 33..63.
int rotrBH(int h, int l, int s) => (h << (64 - s) | l >> (s - 32)) & mask32;

/// Low half of a 64-bit right rotation by [s] in 33..63.
int rotrBL(int h, int l, int s) => (h >> (s - 32) | l << (64 - s)) & mask32;

/// High half of a 64-bit right rotation by 32: the low half.
int rotr32H(int h, int l) => l;

/// Low half of a 64-bit right rotation by 32: the high half.
int rotr32L(int h, int l) => h;

/// The sum of two 64-bit words.
({int h, int l}) add(int ah, int al, int bh, int bl) {
  final l = al + bl;
  return (h: (ah + bh + l ~/ _pow32) & mask32, l: l & mask32);
}

/// The unreduced sum of three low halves, for [add3H].
int add3L(int al, int bl, int cl) => al + bl + cl;

/// The high half of a three-word sum; [low] is the unreduced [add3L].
int add3H(int low, int ah, int bh, int ch) => (ah + bh + ch + low ~/ _pow32) & mask32;

/// The unreduced sum of four low halves, for [add4H].
int add4L(int al, int bl, int cl, int dl) => al + bl + cl + dl;

/// The high half of a four-word sum; [low] is the unreduced [add4L].
int add4H(int low, int ah, int bh, int ch, int dh) => (ah + bh + ch + dh + low ~/ _pow32) & mask32;

/// The unreduced sum of five low halves, for [add5H].
int add5L(int al, int bl, int cl, int dl, int el) => al + bl + cl + dl + el;

/// The high half of a five-word sum; [low] is the unreduced [add5L].
int add5H(int low, int ah, int bh, int ch, int dh, int eh) =>
    (ah + bh + ch + dh + eh + low ~/ _pow32) & mask32;
