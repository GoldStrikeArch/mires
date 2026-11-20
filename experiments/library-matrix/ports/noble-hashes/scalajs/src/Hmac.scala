package noblehashes

/** noble's hmac.ts: _HMAC over the hash that `create` makes. */
final class Hmac(create: () => Hash, key: Array[Byte]) extends Hash:
  private val iHash = create()
  val blockLen: Int = iHash.blockLen
  val outputLen: Int = iHash.outputLen
  private val oHash = create()

  locally {
    val pad = new Array[Byte](blockLen)
    // A key longer than a block is replaced by its digest.
    val k = if key.length > blockLen then create().update(key).digest() else key
    System.arraycopy(k, 0, pad, 0, k.length)
    for i <- pad.indices do pad(i) = (pad(i) ^ 0x36).toByte
    iHash.update(pad)
    // Undo the inner XOR and apply the outer one.
    for i <- pad.indices do pad(i) = (pad(i) ^ (0x36 ^ 0x5c)).toByte
    oHash.update(pad)
  }

  def update(data: Array[Byte]): this.type =
    iHash.update(data)
    this

  def digest(): Array[Byte] = oHash.update(iHash.digest()).digest()
