@file:OptIn(ExperimentalUnsignedTypes::class)

// noble's sha2.ts: SHA-256 (SHA2_32B) and SHA-512 (SHA2_64B), with the IVs from _md.ts.

internal val SHA256_IV = words(
    0x6a09e667u, 0xbb67ae85u, 0x3c6ef372u, 0xa54ff53au, 0x510e527fu, 0x9b05688cu, 0x1f83d9abu, 0x5be0cd19u,
)

private val SHA256_K = words(
    0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u, 0x3956c25bu, 0x59f111f1u, 0x923f82a4u, 0xab1c5ed5u,
    0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u, 0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u,
    0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu, 0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
    0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u, 0xc6e00bf3u, 0xd5a79147u, 0x06ca6351u, 0x14292967u,
    0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u, 0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u,
    0xa2bfe8a1u, 0xa81a664bu, 0xc24b8b70u, 0xc76c51a3u, 0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
    0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u, 0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu, 0x682e6ff3u,
    0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u, 0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u,
)

/** noble's SHA2_32B with the SHA-256 IV. */
class Sha256 : HashMD(64, 32, 8) {
    private var a = SHA256_IV[0]
    private var b = SHA256_IV[1]
    private var c = SHA256_IV[2]
    private var d = SHA256_IV[3]
    private var e = SHA256_IV[4]
    private var f = SHA256_IV[5]
    private var g = SHA256_IV[6]
    private var h = SHA256_IV[7]

    /** The message schedule; noble shares one module-level SHA256_W. */
    private val w = IntArray(64)

    override fun get(): IntArray = intArrayOf(a, b, c, d, e, f, g, h)

    override fun process(data: ByteArray, offset: Int) {
        for (i in 0 until 16) w[i] = data.getUint32BE(offset + 4 * i)
        for (i in 16 until 64) {
            val w15 = w[i - 15]
            val w2 = w[i - 2]
            val s0 = w15.rotateRight(7) xor w15.rotateRight(18) xor (w15 ushr 3)
            val s1 = w2.rotateRight(17) xor w2.rotateRight(19) xor (w2 ushr 10)
            w[i] = s1 + w[i - 7] + s0 + w[i - 16]
        }
        var a = a
        var b = b
        var c = c
        var d = d
        var e = e
        var f = f
        var g = g
        var h = h
        for (i in 0 until 64) {
            val sigma1 = e.rotateRight(6) xor e.rotateRight(11) xor e.rotateRight(25)
            val t1 = h + sigma1 + chi(e, f, g) + SHA256_K[i] + w[i]
            val sigma0 = a.rotateRight(2) xor a.rotateRight(13) xor a.rotateRight(22)
            val t2 = sigma0 + maj(a, b, c)
            h = g
            g = f
            f = e
            e = d + t1
            d = c
            c = b
            b = a
            a = t1 + t2
        }
        this.a += a
        this.b += b
        this.c += c
        this.d += d
        this.e += e
        this.f += f
        this.g += g
        this.h += h
    }
}

private val SHA512_IV = words(
    0x6a09e667u, 0xf3bcc908u, 0xbb67ae85u, 0x84caa73bu, 0x3c6ef372u, 0xfe94f82bu, 0xa54ff53au, 0x5f1d36f1u,
    0x510e527fu, 0xade682d1u, 0x9b05688cu, 0x2b3e6c1fu, 0x1f83d9abu, 0xfb41bd6bu, 0x5be0cd19u, 0x137e2179u,
)

// The 80 round constants of RFC 6234 §5.2, split into high and low halves as noble's K512 is.
private val SHA512_KH = words(
    0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u, 0x3956c25bu, 0x59f111f1u, 0x923f82a4u, 0xab1c5ed5u,
    0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u, 0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u,
    0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu, 0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
    0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u, 0xc6e00bf3u, 0xd5a79147u, 0x06ca6351u, 0x14292967u,
    0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u, 0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u,
    0xa2bfe8a1u, 0xa81a664bu, 0xc24b8b70u, 0xc76c51a3u, 0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
    0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u, 0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu, 0x682e6ff3u,
    0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u, 0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u,
    0xca273eceu, 0xd186b8c7u, 0xeada7dd6u, 0xf57d4f7fu, 0x06f067aau, 0x0a637dc5u, 0x113f9804u, 0x1b710b35u,
    0x28db77f5u, 0x32caab7bu, 0x3c9ebe0au, 0x431d67c4u, 0x4cc5d4beu, 0x597f299cu, 0x5fcb6fabu, 0x6c44198cu,
)

private val SHA512_KL = words(
    0xd728ae22u, 0x23ef65cdu, 0xec4d3b2fu, 0x8189dbbcu, 0xf348b538u, 0xb605d019u, 0xaf194f9bu, 0xda6d8118u,
    0xa3030242u, 0x45706fbeu, 0x4ee4b28cu, 0xd5ffb4e2u, 0xf27b896fu, 0x3b1696b1u, 0x25c71235u, 0xcf692694u,
    0x9ef14ad2u, 0x384f25e3u, 0x8b8cd5b5u, 0x77ac9c65u, 0x592b0275u, 0x6ea6e483u, 0xbd41fbd4u, 0x831153b5u,
    0xee66dfabu, 0x2db43210u, 0x98fb213fu, 0xbeef0ee4u, 0x3da88fc2u, 0x930aa725u, 0xe003826fu, 0x0a0e6e70u,
    0x46d22ffcu, 0x5c26c926u, 0x5ac42aedu, 0x9d95b3dfu, 0x8baf63deu, 0x3c77b2a8u, 0x47edaee6u, 0x1482353bu,
    0x4cf10364u, 0xbc423001u, 0xd0f89791u, 0x0654be30u, 0xd6ef5218u, 0x5565a910u, 0x5771202au, 0x32bbd1b8u,
    0xb8d2d0c8u, 0x5141ab53u, 0xdf8eeb99u, 0xe19b48a8u, 0xc5c95a63u, 0xe3418acbu, 0x7763e373u, 0xd6b2b8a3u,
    0x5defb2fcu, 0x43172f60u, 0xa1f0ab72u, 0x1a6439ecu, 0x23631e28u, 0xde82bde9u, 0xb2c67915u, 0xe372532bu,
    0xea26619cu, 0x21c0c207u, 0xcde0eb1eu, 0xee6ed178u, 0x72176fbau, 0xa2c898a6u, 0xbef90daeu, 0x131c471bu,
    0x23047d84u, 0x40c72493u, 0x15c9bebcu, 0x9c100d4cu, 0xcb3e42b6u, 0xfc657e2au, 0x3ad6faecu, 0x4a475817u,
)

/** noble's SHA2_64B with the SHA-512 IV: eight 64-bit words as high and low halves. */
class Sha512 : HashMD(128, 64, 16) {
    private var ah = SHA512_IV[0]
    private var al = SHA512_IV[1]
    private var bh = SHA512_IV[2]
    private var bl = SHA512_IV[3]
    private var ch = SHA512_IV[4]
    private var cl = SHA512_IV[5]
    private var dh = SHA512_IV[6]
    private var dl = SHA512_IV[7]
    private var eh = SHA512_IV[8]
    private var el = SHA512_IV[9]
    private var fh = SHA512_IV[10]
    private var fl = SHA512_IV[11]
    private var gh = SHA512_IV[12]
    private var gl = SHA512_IV[13]
    private var hh = SHA512_IV[14]
    private var hl = SHA512_IV[15]

    /** The high and low halves of the message schedule (noble's SHA512_W_H and SHA512_W_L). */
    private val wh = IntArray(80)
    private val wl = IntArray(80)

    override fun get(): IntArray = intArrayOf(ah, al, bh, bl, ch, cl, dh, dl, eh, el, fh, fl, gh, gl, hh, hl)

    override fun process(data: ByteArray, offset: Int) {
        for (i in 0 until 16) {
            wh[i] = data.getUint32BE(offset + 8 * i)
            wl[i] = data.getUint32BE(offset + 8 * i + 4)
        }
        for (i in 16 until 80) {
            // s0 = rotr(w[i-15], 1) ^ rotr(w[i-15], 8) ^ (w[i-15] >>> 7)
            val w15h = wh[i - 15]
            val w15l = wl[i - 15]
            val s0h = rotrSH(w15h, w15l, 1) xor rotrSH(w15h, w15l, 8) xor shrSH(w15h, w15l, 7)
            val s0l = rotrSL(w15h, w15l, 1) xor rotrSL(w15h, w15l, 8) xor shrSL(w15h, w15l, 7)
            // s1 = rotr(w[i-2], 19) ^ rotr(w[i-2], 61) ^ (w[i-2] >>> 6)
            val w2h = wh[i - 2]
            val w2l = wl[i - 2]
            val s1h = rotrSH(w2h, w2l, 19) xor rotrBH(w2h, w2l, 61) xor shrSH(w2h, w2l, 6)
            val s1l = rotrSL(w2h, w2l, 19) xor rotrBL(w2h, w2l, 61) xor shrSL(w2h, w2l, 6)
            val (sumH, sumL) = add4(s0h, s0l, s1h, s1l, wh[i - 7], wl[i - 7], wh[i - 16], wl[i - 16])
            wh[i] = sumH
            wl[i] = sumL
        }
        var ah = ah
        var al = al
        var bh = bh
        var bl = bl
        var ch = ch
        var cl = cl
        var dh = dh
        var dl = dl
        var eh = eh
        var el = el
        var fh = fh
        var fl = fl
        var gh = gh
        var gl = gl
        var hh = hh
        var hl = hl
        for (i in 0 until 80) {
            // S1 = rotr(e, 14) ^ rotr(e, 18) ^ rotr(e, 41)
            val sigma1h = rotrSH(eh, el, 14) xor rotrSH(eh, el, 18) xor rotrBH(eh, el, 41)
            val sigma1l = rotrSL(eh, el, 14) xor rotrSL(eh, el, 18) xor rotrBL(eh, el, 41)
            val chiH = chi(eh, fh, gh)
            val chiL = chi(el, fl, gl)
            // T1 = H + S1 + Chi(E, F, G) + K[i] + W[i]
            val (t1h, t1l) = add5(hh, hl, sigma1h, sigma1l, chiH, chiL, SHA512_KH[i], SHA512_KL[i], wh[i], wl[i])
            // S0 = rotr(a, 28) ^ rotr(a, 34) ^ rotr(a, 39)
            val sigma0h = rotrSH(ah, al, 28) xor rotrBH(ah, al, 34) xor rotrBH(ah, al, 39)
            val sigma0l = rotrSL(ah, al, 28) xor rotrBL(ah, al, 34) xor rotrBL(ah, al, 39)
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
        }
        add(this.ah, this.al, ah, al).let { (h, l) -> this.ah = h; this.al = l }
        add(this.bh, this.bl, bh, bl).let { (h, l) -> this.bh = h; this.bl = l }
        add(this.ch, this.cl, ch, cl).let { (h, l) -> this.ch = h; this.cl = l }
        add(this.dh, this.dl, dh, dl).let { (h, l) -> this.dh = h; this.dl = l }
        add(this.eh, this.el, eh, el).let { (h, l) -> this.eh = h; this.el = l }
        add(this.fh, this.fl, fh, fl).let { (h, l) -> this.fh = h; this.fl = l }
        add(this.gh, this.gl, gh, gl).let { (h, l) -> this.gh = h; this.gl = l }
        add(this.hh, this.hl, hh, hl).let { (h, l) -> this.hh = h; this.hl = l }
    }
}
