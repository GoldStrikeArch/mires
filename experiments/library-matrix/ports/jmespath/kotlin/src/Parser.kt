// jmespath's Pratt parser, method by method. Nodes are plain JS objects with jmespath's property
// order, since `compile` returns them and `&expr` passes them around as values.

package jmespath

import kotlin.js.json

private fun node(type: String, vararg properties: Pair<String, Any?>): Node = json("type" to type, *properties).asNode()

private fun identity() = node("Identity")

private fun branch(type: String, vararg children: Node?) = node(type, "children" to arrayOf(*children))

private val TokenType.power: Int get() = checkNotNull(bindingPower)

/** `rbp < bindingPower[type]`, false where jmespath's table has no entry. */
private fun TokenType.bindsTighterThan(rbp: Int) = bindingPower.let { it != null && rbp < it }

internal class Parser {
    private var tokens: List<Token> = emptyList()
    private var index = 0

    fun parse(expression: String): Node? {
        tokens = Lexer().tokenize(expression).also { it += Token(TokenType.EOF, "", expression.length) }
        index = 0
        val ast = expression(0)
        if (lookahead(0) != TokenType.EOF) {
            val t = lookaheadToken(0)
            throw syntaxError("Unexpected token type: ${t.type}")
        }
        return ast
    }

    private fun expression(rbp: Int): Node? {
        val leftToken = lookaheadToken(0)
        advance()
        var left = nud(leftToken)
        var currentToken = lookahead(0)
        while (currentToken.bindsTighterThan(rbp)) {
            advance()
            left = led(currentToken, left)
            currentToken = lookahead(0)
        }
        return left
    }

    private fun lookahead(number: Int) = tokens[index + number].type

    private fun lookaheadToken(number: Int) = tokens[index + number]

    private fun advance() {
        index++
    }

    private fun nud(token: Token): Node? = when (token.type) {
        TokenType.Literal -> node("Literal", "value" to token.value)
        TokenType.UnquotedIdentifier -> node("Field", "name" to token.value)
        TokenType.QuotedIdentifier -> {
            val field = node("Field", "name" to token.value)
            if (lookahead(0) == TokenType.Lparen) throw syntaxError("Quoted identifier not allowed for function names.")
            field
        }
        TokenType.Not -> branch("NotExpression", expression(TokenType.Not.power))
        TokenType.Star -> {
            val right = if (lookahead(0) == TokenType.Rbracket) identity() else parseProjectionRHS(TokenType.Star.power)
            branch("ValueProjection", identity(), right)
        }
        TokenType.Filter -> led(token.type, identity())
        TokenType.Lbrace -> parseMultiselectHash()
        TokenType.Flatten -> {
            val left = branch("Flatten", identity())
            branch("Projection", left, parseProjectionRHS(TokenType.Flatten.power))
        }
        TokenType.Lbracket -> when {
            lookahead(0) == TokenType.Number || lookahead(0) == TokenType.Colon -> projectIfSlice(identity(), parseIndexExpression())
            lookahead(0) == TokenType.Star && lookahead(1) == TokenType.Rbracket -> {
                advance()
                advance()
                branch("Projection", identity(), parseProjectionRHS(TokenType.Star.power))
            }
            else -> parseMultiselectList()
        }
        TokenType.Current -> node("Current")
        TokenType.Expref -> branch("ExpressionReference", expression(TokenType.Expref.power))
        TokenType.Lparen -> {
            val args = parseArguments(commas = false)
            // `args[0]`: undefined for `()`.
            if (args.isEmpty()) undefined else args[0]
        }
        else -> throw errorToken(token)
    }

    private fun led(tokenName: TokenType, left: Node?): Node? = when (tokenName) {
        TokenType.Dot -> {
            val rbp = TokenType.Dot.power
            if (lookahead(0) != TokenType.Star) {
                branch("Subexpression", left, parseDotRHS(rbp))
            } else {
                advance()
                branch("ValueProjection", left, parseProjectionRHS(rbp))
            }
        }
        TokenType.Pipe -> branch("Pipe", left, expression(TokenType.Pipe.power))
        TokenType.Or -> branch("OrExpression", left, expression(TokenType.Or.power))
        TokenType.And -> branch("AndExpression", left, expression(TokenType.And.power))
        TokenType.Lparen -> {
            // `left.name`: a TypeError (syntax) when there is no left node, as after `()`.
            val name = (left ?: throw syntaxError("Function call without a name")).name
            node("Function", "name" to name, "children" to parseArguments(commas = true).toTypedArray())
        }
        TokenType.Filter -> {
            val condition = expression(0)
            match(TokenType.Rbracket)
            val right = if (lookahead(0) == TokenType.Flatten) identity() else parseProjectionRHS(TokenType.Filter.power)
            branch("FilterProjection", left, right, condition)
        }
        TokenType.Flatten -> branch("Projection", branch("Flatten", left), parseProjectionRHS(TokenType.Flatten.power))
        TokenType.EQ, TokenType.NE, TokenType.GT, TokenType.GTE, TokenType.LT, TokenType.LTE -> parseComparator(left, tokenName)
        TokenType.Lbracket -> {
            val token = lookaheadToken(0)
            if (token.type == TokenType.Number || token.type == TokenType.Colon) {
                projectIfSlice(left, parseIndexExpression())
            } else {
                match(TokenType.Star)
                match(TokenType.Rbracket)
                branch("Projection", left, parseProjectionRHS(TokenType.Star.power))
            }
        }
        else -> throw errorToken(lookaheadToken(0))
    }

    /**
     * The argument loops of `(…)` and of a function call. An argument that starts with `@` is
     * the bare Current node, so `abs(@.a)` fails on the `.` as in jmespath. Calls may separate
     * arguments with commas; parentheses may not.
     */
    private fun parseArguments(commas: Boolean): List<Node?> {
        val args = mutableListOf<Node?>()
        while (lookahead(0) != TokenType.Rparen) {
            val expression = if (lookahead(0) == TokenType.Current) {
                advance()
                node("Current")
            } else {
                expression(0)
            }
            if (commas && lookahead(0) == TokenType.Comma) match(TokenType.Comma)
            args += expression
        }
        match(TokenType.Rparen)
        return args
    }

    private fun match(tokenType: TokenType) {
        if (lookahead(0) == tokenType) advance() else throw syntaxError("Expected $tokenType, got: ${lookahead(0)}")
    }

    private fun errorToken(token: Token) = syntaxError("Invalid token (${token.type})")

    private fun parseIndexExpression(): Node =
        if (lookahead(0) == TokenType.Colon || lookahead(1) == TokenType.Colon) {
            parseSliceExpression()
        } else {
            val index = node("Index", "value" to lookaheadToken(0).value)
            advance()
            match(TokenType.Rbracket)
            index
        }

    private fun projectIfSlice(left: Node?, right: Node): Node {
        val indexExpr = branch("IndexExpression", left, right)
        return if (right.type == "Slice") branch("Projection", indexExpr, parseProjectionRHS(TokenType.Star.power)) else indexExpr
    }

    /** `[start:stop:step]`, each part optional; a later number in one part replaces an earlier one. */
    private fun parseSliceExpression(): Node {
        val parts = arrayOf<Any?>(null, null, null)
        var index = 0
        var currentToken = lookahead(0)
        while (currentToken != TokenType.Rbracket && index < 3) {
            when (currentToken) {
                TokenType.Colon -> {
                    index++
                    advance()
                }
                TokenType.Number -> {
                    parts[index] = lookaheadToken(0).value
                    advance()
                }
                else -> throw syntaxError("Syntax error, unexpected token: $currentToken")
            }
            currentToken = lookahead(0)
        }
        match(TokenType.Rbracket)
        return node("Slice", "children" to parts)
    }

    private fun parseComparator(left: Node?, comparator: TokenType): Node {
        val right = expression(comparator.power)
        return node("Comparator", "name" to comparator.name, "children" to arrayOf(left, right))
    }

    /** Undefined (no error) when nothing that can follow a dot follows it, as in jmespath. */
    private fun parseDotRHS(rbp: Int): Node? = when (lookahead(0)) {
        TokenType.UnquotedIdentifier, TokenType.QuotedIdentifier, TokenType.Star -> expression(rbp)
        TokenType.Lbracket -> {
            match(TokenType.Lbracket)
            parseMultiselectList()
        }
        TokenType.Lbrace -> {
            match(TokenType.Lbrace)
            parseMultiselectHash()
        }
        else -> undefined
    }

    private fun parseProjectionRHS(rbp: Int): Node? {
        val next = lookahead(0)
        val power = next.bindingPower
        return when {
            power != null && power < 10 -> identity()
            next == TokenType.Lbracket || next == TokenType.Filter -> expression(rbp)
            next == TokenType.Dot -> {
                match(TokenType.Dot)
                parseDotRHS(rbp)
            }
            else -> throw syntaxError("Sytanx error, unexpected token: $next")
        }
    }

    /** `[a, b]`; the comma between elements is optional, a trailing one is an error. */
    private fun parseMultiselectList(): Node {
        val expressions = mutableListOf<Node?>()
        while (lookahead(0) != TokenType.Rbracket) {
            expressions += expression(0)
            if (lookahead(0) == TokenType.Comma) {
                match(TokenType.Comma)
                if (lookahead(0) == TokenType.Rbracket) throw syntaxError("Unexpected token Rbracket")
            }
        }
        match(TokenType.Rbracket)
        return node("MultiSelectList", "children" to expressions.toTypedArray())
    }

    /** `{k: v, …}`; as in jmespath, the comma between pairs is optional too. */
    private fun parseMultiselectHash(): Node {
        val pairs = mutableListOf<Node>()
        while (true) {
            val keyToken = lookaheadToken(0)
            if (keyToken.type != TokenType.UnquotedIdentifier && keyToken.type != TokenType.QuotedIdentifier) {
                throw syntaxError("Expecting an identifier token, got: ${keyToken.type}")
            }
            advance()
            match(TokenType.Colon)
            val value = expression(0)
            pairs += node("KeyValuePair", "name" to keyToken.value, "value" to value)
            if (lookahead(0) == TokenType.Comma) {
                match(TokenType.Comma)
            } else if (lookahead(0) == TokenType.Rbrace) {
                match(TokenType.Rbrace)
                break
            }
        }
        return node("MultiSelectHash", "children" to pairs.toTypedArray())
    }
}
