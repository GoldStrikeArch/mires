// jmespath's Pratt parser, method by method. Nodes are plain JS objects with jmespath's property
// order, since `compile` returns them and `&expr` passes them around as values.

package jmespath

import scala.collection.mutable.ArrayBuffer
import scala.scalajs.js
import scala.scalajs.js.JSConverters.*

import Errors.syntaxError
import TokenType.*

private object Parser:
  /** A node with its properties in jmespath's order: `type` first, then the given ones. */
  def node(`type`: String, properties: (String, js.Any)*): Node =
    val node = js.Dictionary[js.Any]("type" -> `type`)
    for (key, value) <- properties do node(key) = value
    node.asInstanceOf[Node]

  def identity(): Node = node("Identity")

  def branch(`type`: String, children: js.UndefOr[Node]*): Node = node(`type`, "children" -> children.toJSArray)

  /** A node that may be undefined (`a.`, `()`), stored as the JS value it is. */
  def jsValue(node: js.UndefOr[Node]): js.Any = node.asInstanceOf[js.Any]

  /** `rbp < bindingPower[type]`, false where jmespath's table has no entry. */
  extension (tokenType: TokenType) def bindsTighterThan(rbp: Int): Boolean = tokenType.bindingPower.exists(rbp < _)

final class Parser:
  import Parser.*

  private var tokens = ArrayBuffer.empty[Token]
  private var index = 0

  def parse(expression: String): js.UndefOr[Node] =
    tokens = Lexer().tokenize(expression)
    tokens += Token(EOF, "", expression.length)
    index = 0
    val ast = this.expression(0)
    if lookahead(0) != EOF then throw syntaxError(s"Unexpected token type: ${lookahead(0)}")
    ast

  private def expression(rbp: Int): js.UndefOr[Node] =
    val leftToken = lookaheadToken(0)
    advance()
    var left = nud(leftToken)
    var currentToken = lookahead(0)
    while currentToken.bindsTighterThan(rbp) do
      advance()
      left = led(currentToken, left)
      currentToken = lookahead(0)
    left

  private def lookahead(number: Int): TokenType = tokens(index + number).`type`

  private def lookaheadToken(number: Int): Token = tokens(index + number)

  private def advance(): Unit = index += 1

  private def nud(token: Token): js.UndefOr[Node] = token.`type` match
    case Literal => node("Literal", "value" -> token.value)
    case UnquotedIdentifier => node("Field", "name" -> token.value)
    case QuotedIdentifier =>
      val field = node("Field", "name" -> token.value)
      if lookahead(0) == Lparen then throw syntaxError("Quoted identifier not allowed for function names.")
      field
    case Not => branch("NotExpression", expression(Not.power))
    case Star =>
      val right = if lookahead(0) == Rbracket then identity() else parseProjectionRHS(Star.power)
      branch("ValueProjection", identity(), right)
    case Filter => led(token.`type`, identity())
    case Lbrace => parseMultiselectHash()
    case Flatten =>
      val left = branch("Flatten", identity())
      branch("Projection", left, parseProjectionRHS(Flatten.power))
    case Lbracket =>
      if lookahead(0) == Number || lookahead(0) == Colon then projectIfSlice(identity(), parseIndexExpression())
      else if lookahead(0) == Star && lookahead(1) == Rbracket then
        advance()
        advance()
        branch("Projection", identity(), parseProjectionRHS(Star.power))
      else parseMultiselectList()
    case Current => node("Current")
    case Expref => branch("ExpressionReference", expression(Expref.power))
    case Lparen =>
      val args = parseArguments(commas = false)
      // `args[0]`: undefined for `()`.
      if args.isEmpty then js.undefined else args(0)
    case _ => throw errorToken(token)

  private def led(tokenName: TokenType, left: js.UndefOr[Node]): js.UndefOr[Node] = tokenName match
    case Dot =>
      val rbp = Dot.power
      if lookahead(0) != Star then branch("Subexpression", left, parseDotRHS(rbp))
      else
        advance()
        branch("ValueProjection", left, parseProjectionRHS(rbp))
    case Pipe => branch("Pipe", left, expression(Pipe.power))
    case Or => branch("OrExpression", left, expression(Or.power))
    case And => branch("AndExpression", left, expression(And.power))
    case Lparen =>
      // `left.name`: a TypeError (syntax) when there is no left node, as after `()`.
      val name = left.getOrElse(throw syntaxError("Function call without a name")).name
      node("Function", "name" -> name, "children" -> parseArguments(commas = true).toJSArray)
    case Filter =>
      val condition = expression(0)
      `match`(Rbracket)
      val right = if lookahead(0) == Flatten then identity() else parseProjectionRHS(Filter.power)
      branch("FilterProjection", left, right, condition)
    case Flatten => branch("Projection", branch("Flatten", left), parseProjectionRHS(Flatten.power))
    case EQ | NE | GT | GTE | LT | LTE => parseComparator(left, tokenName)
    case Lbracket =>
      val token = lookaheadToken(0)
      if token.`type` == Number || token.`type` == Colon then projectIfSlice(left, parseIndexExpression())
      else
        `match`(Star)
        `match`(Rbracket)
        branch("Projection", left, parseProjectionRHS(Star.power))
    case _ => throw errorToken(lookaheadToken(0))

  /** The argument loops of `(…)` and of a function call. An argument that starts with `@` is the bare
    * Current node, so `abs(@.a)` fails on the `.` as in jmespath. Calls may separate arguments with
    * commas; parentheses may not.
    */
  private def parseArguments(commas: Boolean): ArrayBuffer[js.UndefOr[Node]] =
    val args = ArrayBuffer.empty[js.UndefOr[Node]]
    while lookahead(0) != Rparen do
      val argument: js.UndefOr[Node] =
        if lookahead(0) == Current then
          advance()
          node("Current")
        else expression(0)
      if commas && lookahead(0) == Comma then `match`(Comma)
      args += argument
    `match`(Rparen)
    args

  private def `match`(tokenType: TokenType): Unit =
    if lookahead(0) == tokenType then advance()
    else throw syntaxError(s"Expected $tokenType, got: ${lookahead(0)}")

  private def errorToken(token: Token) = syntaxError(s"Invalid token (${token.`type`})")

  private def parseIndexExpression(): Node =
    if lookahead(0) == Colon || lookahead(1) == Colon then parseSliceExpression()
    else
      val index = node("Index", "value" -> lookaheadToken(0).value)
      advance()
      `match`(Rbracket)
      index

  private def projectIfSlice(left: js.UndefOr[Node], right: Node): Node =
    val indexExpr = branch("IndexExpression", left, right)
    if Host.stringValue(right.`type`).contains("Slice") then branch("Projection", indexExpr, parseProjectionRHS(Star.power))
    else indexExpr

  /** `[start:stop:step]`, each part optional; a later number in one part replaces an earlier one. */
  private def parseSliceExpression(): Node =
    val parts = js.Array[js.Any](null, null, null)
    var index = 0
    var currentToken = lookahead(0)
    while currentToken != Rbracket && index < 3 do
      currentToken match
        case Colon =>
          index += 1
          advance()
        case Number =>
          parts(index) = lookaheadToken(0).value
          advance()
        case _ => throw syntaxError(s"Syntax error, unexpected token: $currentToken")
      currentToken = lookahead(0)
    `match`(Rbracket)
    node("Slice", "children" -> parts)

  private def parseComparator(left: js.UndefOr[Node], comparator: TokenType): Node =
    val right = expression(comparator.power)
    node("Comparator", "name" -> comparator.toString, "children" -> js.Array(left, right))

  /** Undefined (no error) when nothing that can follow a dot follows it, as in jmespath. */
  private def parseDotRHS(rbp: Int): js.UndefOr[Node] = lookahead(0) match
    case UnquotedIdentifier | QuotedIdentifier | Star => expression(rbp)
    case Lbracket =>
      `match`(Lbracket)
      parseMultiselectList()
    case Lbrace =>
      `match`(Lbrace)
      parseMultiselectHash()
    case _ => js.undefined

  private def parseProjectionRHS(rbp: Int): js.UndefOr[Node] =
    val next = lookahead(0)
    if next.bindingPower.exists(_ < 10) then identity()
    else if next == Lbracket || next == Filter then expression(rbp)
    else if next == Dot then
      `match`(Dot)
      parseDotRHS(rbp)
    else throw syntaxError(s"Sytanx error, unexpected token: $next")

  /** `[a, b]`; the comma between elements is optional, a trailing one is an error. */
  private def parseMultiselectList(): Node =
    val expressions = ArrayBuffer.empty[js.UndefOr[Node]]
    while lookahead(0) != Rbracket do
      expressions += expression(0)
      if lookahead(0) == Comma then
        `match`(Comma)
        if lookahead(0) == Rbracket then throw syntaxError("Unexpected token Rbracket")
    `match`(Rbracket)
    node("MultiSelectList", "children" -> expressions.toJSArray)

  /** `{k: v, …}`; as in jmespath, the comma between pairs is optional too. */
  private def parseMultiselectHash(): Node =
    val pairs = ArrayBuffer.empty[Node]
    var done = false
    while !done do
      val keyToken = lookaheadToken(0)
      if keyToken.`type` != UnquotedIdentifier && keyToken.`type` != QuotedIdentifier then
        throw syntaxError(s"Expecting an identifier token, got: ${keyToken.`type`}")
      advance()
      `match`(Colon)
      val value = expression(0)
      pairs += node("KeyValuePair", "name" -> keyToken.value, "value" -> jsValue(value))
      if lookahead(0) == Comma then `match`(Comma)
      else if lookahead(0) == Rbrace then
        `match`(Rbrace)
        done = true
    node("MultiSelectHash", "children" -> pairs.toJSArray)
