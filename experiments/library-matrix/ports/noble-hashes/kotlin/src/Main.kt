// The boundary: libraries/noble-hashes/reference.mjs in Kotlin. The input is the corpus object
// itself, read through an external interface; byte arrays are copied into ByteArrays.

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

/** Uint8Array.from(values). */
private fun toBytes(values: Array<Int>): ByteArray = ByteArray(values.size) { values[it].toByte() }

/** reference.mjs's `hashes` table plus HMAC-SHA-256: the constructor for [algorithm]. */
private fun hasher(algorithm: String): (Blake2Opts) -> Hash = when (algorithm) {
    "sha256" -> { _ -> Sha256() }
    "sha512" -> { _ -> Sha512() }
    "sha3_256" -> { _ -> Keccak(136, 0x06, 32) }
    "keccak_256" -> { _ -> Keccak(136, 0x01, 32) }
    "blake2b" -> { opts -> Blake2b(opts) }
    "blake2s" -> { opts -> Blake2s(opts) }
    "hmac_sha256" -> { opts -> Hmac({ Sha256() }, requireNotNull(opts.key) { "\"key\" expected Uint8Array" }) }
    else -> throw IllegalArgumentException("unknown hash")
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: HashInput): String {
    val create = hasher(input.algorithm)
    val opts = Blake2Opts(
        input.dkLen,
        input.key?.let { toBytes(it) },
        input.salt?.let { toBytes(it) },
        input.personalization?.let { toBytes(it) },
    )
    val bytes = input.bytes
    if (bytes != null) {
        val message = toBytes(bytes)
        return bytesToHex(create(opts).update(message).digest())
    }
    val state = create(opts)
    val chunks = requireNotNull(input.chunks) { "expected bytes or chunks" }
    val blocks = Array(chunks.size) { toBytes(chunks[it]) }
    repeat(input.repeat ?: 1) {
        for (block in blocks) state.update(block)
    }
    return bytesToHex(state.digest())
}
