// jmespath's Lexer, method by method.

package jmespath

import scala.collection.mutable.ArrayBuffer
import scala.scalajs.js

import Errors.syntaxError
import Host.*

/** Token types and jmespath's `bindingPower` table. Colon and Literal have no entry there, so their
  * power is None: `rbp < bindingPower[Colon]` is false, and so is `bindingPower[Literal] < 10` in
  * `_parseProjectionRHS`, which therefore rejects a literal after a projection.
  */
enum TokenType(val bindingPower: Option[Int]):
  case EOF extends TokenType(Some(0))
  case UnquotedIdentifier extends TokenType(Some(0))
  case QuotedIdentifier extends TokenType(Some(0))
  case Rbracket extends TokenType(Some(0))
  case Rparen extends TokenType(Some(0))
  case Comma extends TokenType(Some(0))
  case Colon extends TokenType(None)
  case Rbrace extends TokenType(Some(0))
  case Number extends TokenType(Some(0))
  case Current extends TokenType(Some(0))
  case Expref extends TokenType(Some(0))
  case Pipe extends TokenType(Some(1))
  case Or extends TokenType(Some(2))
  case And extends TokenType(Some(3))
  case EQ extends TokenType(Some(5))
  case GT extends TokenType(Some(5))
  case LT extends TokenType(Some(5))
  case GTE extends TokenType(Some(5))
  case LTE extends TokenType(Some(5))
  case NE extends TokenType(Some(5))
  case Flatten extends TokenType(Some(9))
  case Star extends TokenType(Some(20))
  case Filter extends TokenType(Some(21))
  case Dot extends TokenType(Some(40))
  case Not extends TokenType(Some(45))
  case Lbrace extends TokenType(Some(50))
  case Lbracket extends TokenType(Some(55))
  case Lparen extends TokenType(Some(60))
  case Literal extends TokenType(None)

  /** For the operators jmespath looks up by name (`bindingPower.Not`), which all have an entry. */
  def power: Int = bindingPower.get

final case class Token(`type`: TokenType, value: js.Any, start: Int)

private object Lexer:
  def isAlpha(ch: Char): Boolean = (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || ch == '_'

  def isNum(ch: Char): Boolean = (ch >= '0' && ch <= '9') || ch == '-'

  def isAlphaNum(ch: Char): Boolean = isAlpha(ch) || (ch >= '0' && ch <= '9')

  def basicToken(ch: Char): Option[TokenType] = ch match
    case '.' => Some(TokenType.Dot)
    case '*' => Some(TokenType.Star)
    case ',' => Some(TokenType.Comma)
    case ':' => Some(TokenType.Colon)
    case '{' => Some(TokenType.Lbrace)
    case '}' => Some(TokenType.Rbrace)
    case ']' => Some(TokenType.Rbracket)
    case '(' => Some(TokenType.Lparen)
    case ')' => Some(TokenType.Rparen)
    case '@' => Some(TokenType.Current)
    case _ => None

  /** `String.prototype.trimLeft`'s set: ECMAScript WhiteSpace and LineTerminator. Java's
    * `Character.isWhitespace` differs (it has U+001C–U+001F, and lacks U+00A0, U+202F and U+FEFF).
    */
  def isJsWhiteSpace(ch: Char): Boolean = ch match
    case '\t' | '\n' | '\u000b' | '\u000c' | '\r' | ' ' | ' ' | ' ' | ' ' | ' ' | ' ' | ' ' |
        '　' | '﻿' =>
      true
    case _ => ch >= ' ' && ch <= ' '

final class Lexer:
  import Lexer.*

  private var current = 0

  def tokenize(stream: String): ArrayBuffer[Token] =
    val tokens = ArrayBuffer.empty[Token]
    current = 0
    while current < stream.length do
      val start = current
      val ch = stream.charAt(current)
      if isAlpha(ch) then tokens += Token(TokenType.UnquotedIdentifier, consumeUnquotedIdentifier(stream), start)
      else
        basicToken(ch) match
          case Some(basic) =>
            tokens += Token(basic, ch.toString, start)
            current += 1
          case None =>
            if isNum(ch) then tokens += consumeNumber(stream)
            else if ch == '[' then tokens += consumeLBracket(stream)
            else if ch == '"' then tokens += Token(TokenType.QuotedIdentifier, consumeQuotedIdentifier(stream), start)
            else if ch == '\'' then tokens += Token(TokenType.Literal, consumeRawStringLiteral(stream), start)
            else if ch == '`' then tokens += Token(TokenType.Literal, consumeLiteral(stream), start)
            else if ch == '<' || ch == '>' || ch == '=' || ch == '!' then tokens += consumeOperator(stream)
            else if ch == ' ' || ch == '\t' || ch == '\n' then current += 1
            else if ch == '&' || ch == '|' then
              current += 1
              val doubled = current < stream.length && stream.charAt(current) == ch
              if doubled then current += 1
              val token = (ch, doubled) match
                case ('&', true) => Token(TokenType.And, "&&", start)
                case ('&', false) => Token(TokenType.Expref, "&", start)
                case (_, true) => Token(TokenType.Or, "||", start)
                case (_, false) => Token(TokenType.Pipe, "|", start)
              tokens += token
            else throw syntaxError(s"Unknown character:$ch")
    tokens

  private def consumeUnquotedIdentifier(stream: String): String =
    val start = current
    current += 1
    while current < stream.length && isAlphaNum(stream.charAt(current)) do current += 1
    stream.substring(start, current)

  /** Skips a quoted body, where a backslash escapes `quote` or another backslash. */
  private def skipQuoted(stream: String, quote: Char): Unit =
    while current < stream.length && stream.charAt(current) != quote do
      val escaped = stream.charAt(current) == '\\' && current + 1 < stream.length &&
        (stream.charAt(current + 1) == '\\' || stream.charAt(current + 1) == quote)
      current += (if escaped then 2 else 1)

  private def consumeQuotedIdentifier(stream: String): js.Any =
    val start = current
    current += 1
    skipQuoted(stream, '"')
    current += 1
    parseJsonToken(stream.substring(start, current min stream.length))

  /** An unterminated raw string runs to the end of the expression, as in jmespath. */
  private def consumeRawStringLiteral(stream: String): String =
    val start = current
    current += 1
    skipQuoted(stream, '\'')
    current += 1
    replaceFirst(stream.substring(start + 1, (current - 1) min stream.length), "\\'", "'")

  private def consumeNumber(stream: String): Token =
    val start = current
    current += 1
    while current < stream.length && isNum(stream.charAt(current)) do current += 1
    Token(TokenType.Number, Globals.parseInt(stream.substring(start, current)), start)

  private def consumeLBracket(stream: String): Token =
    val start = current
    current += 1
    val next = if current < stream.length then stream.charAt(current) else '\u0000'
    if next == '?' then
      current += 1
      Token(TokenType.Filter, "[?", start)
    else if next == ']' then
      current += 1
      Token(TokenType.Flatten, "[]", start)
    else Token(TokenType.Lbracket, "[", start)

  /** A lone `=` is the one character jmespath accepts here without making a token: it pushes
    * `undefined`, and parsing fails on it with a TypeError, which is a syntax error. That error is
    * raised here.
    */
  private def consumeOperator(stream: String): Token =
    val start = current
    val startingChar = stream.charAt(start)
    current += 1
    val equalsFollows = current < stream.length && stream.charAt(current) == '='
    if equalsFollows then current += 1
    (startingChar, equalsFollows) match
      case ('!', true) => Token(TokenType.NE, "!=", start)
      case ('!', false) => Token(TokenType.Not, "!", start)
      case ('<', true) => Token(TokenType.LTE, "<=", start)
      case ('<', false) => Token(TokenType.LT, "<", start)
      case ('>', true) => Token(TokenType.GTE, ">=", start)
      case ('>', false) => Token(TokenType.GT, ">", start)
      case (_, true) => Token(TokenType.EQ, "==", start)
      case (_, false) => throw syntaxError("Unknown operator: =")

  private def consumeLiteral(stream: String): js.Any =
    current += 1
    val start = current
    skipQuoted(stream, '`')
    val literalString = replaceFirst(stream.substring(start, current).dropWhile(isJsWhiteSpace), "\\`", "`")
    val literal = if looksLikeJson(literalString) then parseJsonToken(literalString) else parseJsonToken("\"" + literalString + "\"")
    current += 1
    literal

  /** `JSON.parse`, whose SyntaxError `category()` files as "syntax". Anything else it throws, such as
    * stack exhaustion on a deep literal, propagates.
    */
  private def parseJsonToken(text: String): js.Any =
    try parseJson(text)
    catch case js.JavaScriptException(_: js.SyntaxError) => throw syntaxError(s"Invalid JSON: $text")

  private def looksLikeJson(literalString: String): Boolean =
    if literalString.isEmpty then false
    else if "[{\"".indexOf(literalString.charAt(0).toInt) >= 0 then true
    else if literalString == "true" || literalString == "false" || literalString == "null" then true
    else if "-0123456789".indexOf(literalString.charAt(0).toInt) >= 0 then
      try
        parseJson(literalString)
        true
      catch case _: Throwable => false
    else false
