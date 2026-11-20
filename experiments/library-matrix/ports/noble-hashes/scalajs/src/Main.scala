package noblehashes

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait HashInput extends js.Object {
  val algorithm: String = js.native
  val bytes: js.UndefOr[js.Array[Int]] = js.native
  val chunks: js.UndefOr[js.Array[js.Array[Int]]] = js.native
  val repeat: js.UndefOr[Int] = js.native
  val dkLen: js.UndefOr[Double] = js.native
  val key: js.UndefOr[js.Array[Int]] = js.native
  val salt: js.UndefOr[js.Array[Int]] = js.native
  val personalization: js.UndefOr[js.Array[Int]] = js.native
}

object Main {
  private def read(x: js.UndefOr[js.Array[Int]]): Array[Int] =
    x.toOption.fold(Array.empty[Int])(_.toArray)

  @JSExportTopLevel("run")
  def run(input: HashInput): String = {
    val chunks = input.bytes.toOption match {
      case Some(one) => Array(one.toArray)
      case None =>
        val blocks = input.chunks.get.toArray.map(_.toArray)
        Array.tabulate(input.repeat.getOrElse(1) * blocks.length)(i => blocks(i % blocks.length))
    }
    val key = read(input.key)
    val result = input.algorithm match {
      case "sha256" => Sha2.digest(0, chunks, key)
      case "sha512" => Sha2.digest(1, chunks, key)
      case "hmac_sha256" => require(input.key.isDefined); Sha2.digest(2, chunks, key)
      case "sha3_256" | "keccak_256" => Sha3.digest(input.algorithm == "keccak_256", chunks)
      case "blake2b" | "blake2s" =>
        val wide = input.algorithm == "blake2b"
        val limit = if (wide) 64 else 32
        val size = input.dkLen.getOrElse(limit.toDouble)
        require(size == size.toInt && size >= 1 && size <= limit)
        val salt = read(input.salt)
        val pers = read(input.personalization)
        require(input.key.isEmpty || (key.nonEmpty && key.length <= limit))
        require(input.salt.isEmpty || salt.length == limit / 4)
        require(input.personalization.isEmpty || pers.length == limit / 4)
        Blake2.digest(wide, chunks, size.toInt, key, salt, pers)
      case _ => throw new IllegalArgumentException("unknown hash")
    }
    val digits = "0123456789abcdef"
    val buffer = new StringBuilder
    for (b <- result) {
      buffer.append(digits.charAt(b >>> 4))
      buffer.append(digits.charAt(b & 15))
    }
    buffer.toString
  }
}
