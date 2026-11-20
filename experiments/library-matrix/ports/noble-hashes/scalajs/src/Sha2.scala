package noblehashes

import Md.*
import U64.*
import Utils.*

/** noble's sha2.ts: SHA-256 (SHA2_32B) and SHA-512 (SHA2_64B), with the IVs from _md.ts. */
object Sha2:
  val SHA256_IV: Array[Int] = Array(
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  )

  val SHA256_K: Array[Int] = Array(
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  )

  val SHA512_IV: Array[Int] = Array(
    0x6a09e667, 0xf3bcc908, 0xbb67ae85, 0x84caa73b, 0x3c6ef372, 0xfe94f82b, 0xa54ff53a, 0x5f1d36f1,
    0x510e527f, 0xade682d1, 0x9b05688c, 0x2b3e6c1f, 0x1f83d9ab, 0xfb41bd6b, 0x5be0cd19, 0x137e2179,
  )

  // The 80 round constants of RFC 6234 §5.2, split into high and low halves as noble's K512 is.
  val SHA512_KH: Array[Int] = Array(
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    0xca273ece, 0xd186b8c7, 0xeada7dd6, 0xf57d4f7f, 0x06f067aa, 0x0a637dc5, 0x113f9804, 0x1b710b35,
    0x28db77f5, 0x32caab7b, 0x3c9ebe0a, 0x431d67c4, 0x4cc5d4be, 0x597f299c, 0x5fcb6fab, 0x6c44198c,
  )

  val SHA512_KL: Array[Int] = Array(
    0xd728ae22, 0x23ef65cd, 0xec4d3b2f, 0x8189dbbc, 0xf348b538, 0xb605d019, 0xaf194f9b, 0xda6d8118,
    0xa3030242, 0x45706fbe, 0x4ee4b28c, 0xd5ffb4e2, 0xf27b896f, 0x3b1696b1, 0x25c71235, 0xcf692694,
    0x9ef14ad2, 0x384f25e3, 0x8b8cd5b5, 0x77ac9c65, 0x592b0275, 0x6ea6e483, 0xbd41fbd4, 0x831153b5,
    0xee66dfab, 0x2db43210, 0x98fb213f, 0xbeef0ee4, 0x3da88fc2, 0x930aa725, 0xe003826f, 0x0a0e6e70,
    0x46d22ffc, 0x5c26c926, 0x5ac42aed, 0x9d95b3df, 0x8baf63de, 0x3c77b2a8, 0x47edaee6, 0x1482353b,
    0x4cf10364, 0xbc423001, 0xd0f89791, 0x0654be30, 0xd6ef5218, 0x5565a910, 0x5771202a, 0x32bbd1b8,
    0xb8d2d0c8, 0x5141ab53, 0xdf8eeb99, 0xe19b48a8, 0xc5c95a63, 0xe3418acb, 0x7763e373, 0xd6b2b8a3,
    0x5defb2fc, 0x43172f60, 0xa1f0ab72, 0x1a6439ec, 0x23631e28, 0xde82bde9, 0xb2c67915, 0xe372532b,
    0xea26619c, 0x21c0c207, 0xcde0eb1e, 0xee6ed178, 0x72176fba, 0xa2c898a6, 0xbef90dae, 0x131c471b,
    0x23047d84, 0x40c72493, 0x15c9bebc, 0x9c100d4c, 0xcb3e42b6, 0xfc657e2a, 0x3ad6faec, 0x4a475817,
  )

import Sha2.*

/** noble's SHA2_32B with the SHA-256 IV. */
final class Sha256 extends HashMD(64, 32, 8):
  private var a = SHA256_IV(0)
  private var b = SHA256_IV(1)
  private var c = SHA256_IV(2)
  private var d = SHA256_IV(3)
  private var e = SHA256_IV(4)
  private var f = SHA256_IV(5)
  private var g = SHA256_IV(6)
  private var h = SHA256_IV(7)

  /** The message schedule; noble shares one module-level SHA256_W. */
  private val w = new Array[Int](64)

  protected def get(): Array[Int] = Array(a, b, c, d, e, f, g, h)

  protected def process(data: Array[Byte], offset: Int): Unit =
    for i <- 0 until 16 do w(i) = getUint32BE(data, offset + 4 * i)
    for i <- 16 until 64 do
      val w15 = w(i - 15)
      val w2 = w(i - 2)
      val s0 = Integer.rotateRight(w15, 7) ^ Integer.rotateRight(w15, 18) ^ (w15 >>> 3)
      val s1 = Integer.rotateRight(w2, 17) ^ Integer.rotateRight(w2, 19) ^ (w2 >>> 10)
      w(i) = s1 + w(i - 7) + s0 + w(i - 16)
    var a = this.a
    var b = this.b
    var c = this.c
    var d = this.d
    var e = this.e
    var f = this.f
    var g = this.g
    var h = this.h
    for i <- 0 until 64 do
      val sigma1 = Integer.rotateRight(e, 6) ^ Integer.rotateRight(e, 11) ^ Integer.rotateRight(e, 25)
      val t1 = h + sigma1 + chi(e, f, g) + SHA256_K(i) + w(i)
      val sigma0 = Integer.rotateRight(a, 2) ^ Integer.rotateRight(a, 13) ^ Integer.rotateRight(a, 22)
      val t2 = sigma0 + maj(a, b, c)
      h = g
      g = f
      f = e
      e = d + t1
      d = c
      c = b
      b = a
      a = t1 + t2
    this.a += a
    this.b += b
    this.c += c
    this.d += d
    this.e += e
    this.f += f
    this.g += g
    this.h += h

/** noble's SHA2_64B with the SHA-512 IV: eight 64-bit words as high and low halves. */
final class Sha512 extends HashMD(128, 64, 16):
  private var ah = SHA512_IV(0)
  private var al = SHA512_IV(1)
  private var bh = SHA512_IV(2)
  private var bl = SHA512_IV(3)
  private var ch = SHA512_IV(4)
  private var cl = SHA512_IV(5)
  private var dh = SHA512_IV(6)
  private var dl = SHA512_IV(7)
  private var eh = SHA512_IV(8)
  private var el = SHA512_IV(9)
  private var fh = SHA512_IV(10)
  private var fl = SHA512_IV(11)
  private var gh = SHA512_IV(12)
  private var gl = SHA512_IV(13)
  private var hh = SHA512_IV(14)
  private var hl = SHA512_IV(15)

  /** The high and low halves of the message schedule (noble's SHA512_W_H and SHA512_W_L). */
  private val wh = new Array[Int](80)
  private val wl = new Array[Int](80)

  protected def get(): Array[Int] = Array(ah, al, bh, bl, ch, cl, dh, dl, eh, el, fh, fl, gh, gl, hh, hl)

  protected def process(data: Array[Byte], offset: Int): Unit =
    for i <- 0 until 16 do
      wh(i) = getUint32BE(data, offset + 8 * i)
      wl(i) = getUint32BE(data, offset + 8 * i + 4)
    for i <- 16 until 80 do
      // s0 = rotr(w[i-15], 1) ^ rotr(w[i-15], 8) ^ (w[i-15] >>> 7)
      val w15h = wh(i - 15)
      val w15l = wl(i - 15)
      val s0h = rotrSH(w15h, w15l, 1) ^ rotrSH(w15h, w15l, 8) ^ shrSH(w15h, w15l, 7)
      val s0l = rotrSL(w15h, w15l, 1) ^ rotrSL(w15h, w15l, 8) ^ shrSL(w15h, w15l, 7)
      // s1 = rotr(w[i-2], 19) ^ rotr(w[i-2], 61) ^ (w[i-2] >>> 6)
      val w2h = wh(i - 2)
      val w2l = wl(i - 2)
      val s1h = rotrSH(w2h, w2l, 19) ^ rotrBH(w2h, w2l, 61) ^ shrSH(w2h, w2l, 6)
      val s1l = rotrSL(w2h, w2l, 19) ^ rotrBL(w2h, w2l, 61) ^ shrSL(w2h, w2l, 6)
      val (sumH, sumL) = add4(s0h, s0l, s1h, s1l, wh(i - 7), wl(i - 7), wh(i - 16), wl(i - 16))
      wh(i) = sumH
      wl(i) = sumL
    var ah = this.ah
    var al = this.al
    var bh = this.bh
    var bl = this.bl
    var ch = this.ch
    var cl = this.cl
    var dh = this.dh
    var dl = this.dl
    var eh = this.eh
    var el = this.el
    var fh = this.fh
    var fl = this.fl
    var gh = this.gh
    var gl = this.gl
    var hh = this.hh
    var hl = this.hl
    for i <- 0 until 80 do
      // S1 = rotr(e, 14) ^ rotr(e, 18) ^ rotr(e, 41)
      val sigma1h = rotrSH(eh, el, 14) ^ rotrSH(eh, el, 18) ^ rotrBH(eh, el, 41)
      val sigma1l = rotrSL(eh, el, 14) ^ rotrSL(eh, el, 18) ^ rotrBL(eh, el, 41)
      // T1 = H + S1 + Chi(E, F, G) + K[i] + W[i]
      val (t1h, t1l) = add5(hh, hl, sigma1h, sigma1l, chi(eh, fh, gh), chi(el, fl, gl), SHA512_KH(i), SHA512_KL(i), wh(i), wl(i))
      // S0 = rotr(a, 28) ^ rotr(a, 34) ^ rotr(a, 39)
      val sigma0h = rotrSH(ah, al, 28) ^ rotrBH(ah, al, 34) ^ rotrBH(ah, al, 39)
      val sigma0l = rotrSL(ah, al, 28) ^ rotrBL(ah, al, 34) ^ rotrBL(ah, al, 39)
      val majH = maj(ah, bh, ch)
      val majL = maj(al, bl, cl)
      hh = gh
      hl = gl
      gh = fh
      gl = fl
      fh = eh
      fl = el
      val (nextEh, nextEl) = add(dh, dl, t1h, t1l)
      eh = nextEh
      el = nextEl
      dh = ch
      dl = cl
      ch = bh
      cl = bl
      bh = ah
      bl = al
      val (nextAh, nextAl) = add3(t1h, t1l, sigma0h, sigma0l, majH, majL)
      ah = nextAh
      al = nextAl
    // Add the compressed block to the state.
    add(this.ah, this.al, ah, al) match { case (h, l) => this.ah = h; this.al = l }
    add(this.bh, this.bl, bh, bl) match { case (h, l) => this.bh = h; this.bl = l }
    add(this.ch, this.cl, ch, cl) match { case (h, l) => this.ch = h; this.cl = l }
    add(this.dh, this.dl, dh, dl) match { case (h, l) => this.dh = h; this.dl = l }
    add(this.eh, this.el, eh, el) match { case (h, l) => this.eh = h; this.el = l }
    add(this.fh, this.fl, fh, fl) match { case (h, l) => this.fh = h; this.fl = l }
    add(this.gh, this.gl, gh, gl) match { case (h, l) => this.gh = h; this.gl = l }
    add(this.hh, this.hl, hh, hl) match { case (h, l) => this.hh = h; this.hl = l }
