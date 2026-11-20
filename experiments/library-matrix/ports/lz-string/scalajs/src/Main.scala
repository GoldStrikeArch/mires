package lzstring

import scala.collection.mutable
import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

@js.native
trait Input extends js.Object {
  val op: String = js.native
  val format: String = js.native
  val data: js.Array[Int] = js.native
  val repeat: js.UndefOr[Int] = js.native
}

object Main {
  private val base64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/="
  private val uri = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-$"
  private def width(format: String): Int = format match {
    case "raw" | "bytes" => 16
    case "utf16" => 15
    case "base64" | "uri" => 6
    case _ => throw new IllegalArgumentException("unknown format")
  }

  private final class Width(var bits: Int, var left: Int) {
    def advance(): Unit = {
      left -= 1
      if (left == 0) { left = 1 << bits; bits += 1 }
    }
  }

  private def compress(text: String, format: String): Array[Int] = {
    val dictionary = mutable.HashMap.empty[String,Int]
    val fresh = mutable.HashSet.empty[String]
    val words = mutable.ArrayBuffer.empty[Int]
    val size = width(format)
    val codes = new Width(2, 2)
    var used = 0
    var word = 0
    var next = 3
    def write(n: Int, value: Int): Unit = {
      var bit = 0
      while (bit < n) {
        word = (word << 1) | ((value >>> bit) & 1)
        used += 1
        if (used == size) { words += word; word = 0; used = 0 }
        bit += 1
      }
    }
    def emit(w: String): Unit = {
      if (fresh.remove(w)) {
        val c = w.charAt(0).toInt
        write(codes.bits, if (c < 256) 0 else 1)
        write(if (c < 256) 8 else 16, c)
        codes.advance()
      } else write(codes.bits, dictionary(w))
      codes.advance()
    }
    var prefix = ""
    text.foreach { unit =>
      val c = unit.toString
      if (!dictionary.contains(c)) { dictionary(c) = next; next += 1; fresh += c }
      val phrase = prefix + c
      if (dictionary.contains(phrase)) prefix = phrase
      else {
        emit(prefix)
        if (phrase != "__proto__") dictionary(phrase) = next
        next += 1
        prefix = c
      }
    }
    if (prefix.nonEmpty) emit(prefix)
    write(codes.bits, 2)
    write(size - used, 0)
    format match {
      case "raw" => words.toArray
      case "bytes" => words.iterator.flatMap(w => Iterator(w >>> 8, w & 255)).toArray
      case "utf16" => (words.map(_ + 32) += 32).toArray
      case _ =>
        val alphabet = if (format == "base64") base64 else uri
        val encoded = words.map(w => alphabet.charAt(w).toInt)
        if (format == "base64") while (encoded.size % 4 != 0) encoded += 61
        encoded.toArray
    }
  }

  private def decompress(data: Array[Int], format: String): Option[String] = {
    if (data.isEmpty) return None
    val size = width(format)
    val words = format match {
      case "raw" => data
      case "bytes" =>
        require(data.length % 2 == 0)
        Array.tabulate(data.length / 2)(i => data(2*i) * 256 + data(2*i+1))
      case "utf16" => data.map(_ - 32)
      case _ =>
        val alphabet = if (format == "base64") base64 else uri
        data.map(n => alphabet.indexOf(if (format == "uri" && n == 32) '+' else n.toChar).max(0))
    }
    var index = 0
    var position = size
    def read(n: Int): Int = {
      var value = 0
      var bit = 0
      while (bit < n) {
        val word = if (index < words.length) words(index) else 0
        position -= 1
        value |= ((word >>> position) & 1) << bit
        if (position == 0) { position = size; index += 1 }
        bit += 1
      }
      value
    }
    val header = read(2)
    if (header == 2) return Some("")
    var previous = if (header == 3) null else read(if (header == 0) 8 else 16).toChar.toString
    val dictionary = mutable.ArrayBuffer[String](null, null, null, previous)
    val result = new StringBuilder
    if (previous != null) result.append(previous)
    val codes = new Width(3, 4)
    while (index < words.length) {
      var code = read(codes.bits)
      if (code == 2) return Some(result.toString)
      if (code <= 1) {
        val c = read(if (code == 0) 8 else 16).toChar.toString
        code = dictionary.size
        dictionary += c
        codes.advance()
      }
      val entry = if (code < dictionary.size) dictionary(code)
        else if (code == dictionary.size) {
          require(previous != null)
          previous + previous.charAt(0)
        } else return None
      if (entry == null) return None
      result.append(entry)
      dictionary += (if (previous == null) "undefined" else previous) + entry.charAt(0)
      previous = entry
      codes.advance()
    }
    Some("")
  }

  @JSExportTopLevel("run")
  def run(input: Input): js.Array[Int] = {
    if (input.data == null) return js.Array[Int]()
    val data = input.data.toArray
    if (input.op == "decompress") {
      return decompress(data, input.format).map(s => js.Array(s.iterator.map(_.toInt).toSeq*)).orNull
    }
    val text = data.iterator.map(_.toChar).mkString * input.repeat.getOrElse(1)
    val packed = compress(text, input.format)
    if (input.op == "compress") return js.Array(packed.toSeq*)
    require(input.op == "roundtrip" && decompress(packed, input.format).contains(text))
    js.Array(packed.length, text.length)
  }
}
