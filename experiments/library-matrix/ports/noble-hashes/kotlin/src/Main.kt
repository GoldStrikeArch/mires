external interface HashInput {
    val algorithm: String
    val bytes: Array<Int>?
    val chunks: Array<Array<Int>>?
    val repeat: Int?
    val dkLen: Double?
    val key: Array<Int>?
    val salt: Array<Int>?
    val personalization: Array<Int>?
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: HashInput): String {
    val blocks = input.bytes?.let { arrayOf(it.toIntArray()) }
        ?: input.chunks!!.map { it.toIntArray() }.toTypedArray()
    val repeat = if (input.bytes == null) input.repeat ?: 1 else 1
    val chunks = if (repeat == 1) blocks else Array(repeat * blocks.size) { blocks[it % blocks.size] }
    val key = input.key?.toIntArray() ?: intArrayOf()
    val result = when (input.algorithm) {
        "sha256" -> Sha2.digest(0, chunks, key)
        "sha512" -> Sha2.digest(1, chunks, key)
        "hmac_sha256" -> { require(input.key != null); Sha2.digest(2, chunks, key) }
        "sha3_256", "keccak_256" -> Sha3.digest(input.algorithm == "keccak_256", chunks)
        "blake2b", "blake2s" -> {
            val wide = input.algorithm == "blake2b"
            val limit = if (wide) 64 else 32
            val size = input.dkLen ?: limit.toDouble()
            val dkLen = size.toInt()
            require(dkLen in 1..limit && dkLen.toDouble() == size)
            require(input.key == null || key.size in 1..limit)
            require(input.salt == null || input.salt!!.size == limit / 4)
            require(input.personalization == null || input.personalization!!.size == limit / 4)
            Blake2.digest(wide, chunks, dkLen, key, input.salt?.toIntArray() ?: intArrayOf(),
                input.personalization?.toIntArray() ?: intArrayOf())
        }
        else -> error("unknown hash")
    }
    val digits = "0123456789abcdef"
    return buildString { for (b in result) { append(digits[b ushr 4]); append(digits[b and 15]) } }
}
