package noblehashes

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

/** The corpus input, read in place through a facade. */
@js.native
trait HashInput extends js.Object:
  val algorithm: String = js.native
  val bytes: js.UndefOr[js.Array[Int]] = js.native
  val chunks: js.UndefOr[js.Array[js.Array[Int]]] = js.native
  val repeat: js.UndefOr[Int] = js.native
  val dkLen: js.UndefOr[Double] = js.native
  val key: js.UndefOr[js.Array[Int]] = js.native
  val salt: js.UndefOr[js.Array[Int]] = js.native
  val personalization: js.UndefOr[js.Array[Int]] = js.native

/** The boundary: libraries/noble-hashes/reference.mjs in Scala. */
object Main:
  /** Uint8Array.from(values). */
  private def toBytes(values: js.Array[Int]): Array[Byte] = Array.tabulate(values.length)(i => values(i).toByte)

  /** reference.mjs's `hashes` table plus HMAC-SHA-256: the constructor for `algorithm`. */
  private def hasher(algorithm: String): Blake2Opts => Hash = algorithm match
    case "sha256" => _ => Sha256()
    case "sha512" => _ => Sha512()
    case "sha3_256" => _ => Keccak(136, 0x06, 32)
    case "keccak_256" => _ => Keccak(136, 0x01, 32)
    case "blake2b" => opts => Blake2b(opts)
    case "blake2s" => opts => Blake2s(opts)
    case "hmac_sha256" =>
      opts => Hmac(() => Sha256(), opts.key.getOrElse(throw IllegalArgumentException("\"key\" expected Uint8Array")))
    case _ => throw IllegalArgumentException("unknown hash")

  @JSExportTopLevel("run")
  def run(input: HashInput): String =
    val create = hasher(input.algorithm)
    val opts = Blake2Opts(
      input.dkLen.toOption,
      input.key.toOption.map(toBytes),
      input.salt.toOption.map(toBytes),
      input.personalization.toOption.map(toBytes),
    )
    input.bytes.toOption match
      case Some(bytes) =>
        val message = toBytes(bytes)
        Utils.bytesToHex(create(opts).update(message).digest())
      case None =>
        val state = create(opts)
        val blocks = input.chunks.get.toArray.map(toBytes)
        for _ <- 0 until input.repeat.getOrElse(1); block <- blocks do state.update(block)
        Utils.bytesToHex(state.digest())
