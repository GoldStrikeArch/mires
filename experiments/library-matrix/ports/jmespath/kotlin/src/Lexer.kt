// jmespath's Lexer, method by method.

package jmespath

/**
 * Token types and jmespath's `bindingPower` table. Colon and Literal have no entry there, so their
 * power is null: `rbp < bindingPower[Colon]` is false, and so is `bindingPower[Literal] < 10` in
 * `_parseProjectionRHS`, which therefore rejects a literal after a projection.
 */
internal enum class TokenType(val bindingPower: Int?) {
    EOF(0),
    UnquotedIdentifier(0),
    QuotedIdentifier(0),
    Rbracket(0),
    Rparen(0),
    Comma(0),
    Colon(null),
    Rbrace(0),
    Number(0),
    Current(0),
    Expref(0),
    Pipe(1),
    Or(2),
    And(3),
    EQ(5),
    GT(5),
    LT(5),
    GTE(5),
    LTE(5),
    NE(5),
    Flatten(9),
    Star(20),
    Filter(21),
    Dot(40),
    Not(45),
    Lbrace(50),
    Lbracket(55),
    Lparen(60),
    Literal(null),
}

internal class Token(val type: TokenType, val value: Any?, val start: Int)

private fun isAlpha(ch: Char) = ch in 'a'..'z' || ch in 'A'..'Z' || ch == '_'

private fun isNum(ch: Char) = ch in '0'..'9' || ch == '-'

private fun isAlphaNum(ch: Char) = ch in 'a'..'z' || ch in 'A'..'Z' || ch in '0'..'9' || ch == '_'

private fun basicToken(ch: Char): TokenType? = when (ch) {
    '.' -> TokenType.Dot
    '*' -> TokenType.Star
    ',' -> TokenType.Comma
    ':' -> TokenType.Colon
    '{' -> TokenType.Lbrace
    '}' -> TokenType.Rbrace
    ']' -> TokenType.Rbracket
    '(' -> TokenType.Lparen
    ')' -> TokenType.Rparen
    '@' -> TokenType.Current
    else -> null
}

/**
 * `String.prototype.trimLeft`'s set: ECMAScript WhiteSpace and LineTerminator. Kotlin's
 * `Char.isWhitespace` differs (it has U+001C–U+001F and U+0085, and lacks U+FEFF).
 */
private fun isJsWhiteSpace(ch: Char) = when (ch) {
    '\t', '\n', '\u000B', '\u000C', '\r', ' ', ' ', ' ', ' ', ' ', ' ', ' ', '　', '﻿' -> true
    else -> ch in ' '..' '
}

internal class Lexer {
    private var current = 0

    fun tokenize(stream: String): MutableList<Token> {
        val tokens = mutableListOf<Token>()
        current = 0
        while (current < stream.length) {
            val ch = stream[current]
            val start = current
            val basic = basicToken(ch)
            when {
                isAlpha(ch) -> tokens += Token(TokenType.UnquotedIdentifier, consumeUnquotedIdentifier(stream), start)
                basic != null -> {
                    tokens += Token(basic, ch.toString(), start)
                    current++
                }
                isNum(ch) -> tokens += consumeNumber(stream)
                ch == '[' -> tokens += consumeLBracket(stream)
                ch == '"' -> tokens += Token(TokenType.QuotedIdentifier, consumeQuotedIdentifier(stream), start)
                ch == '\'' -> tokens += Token(TokenType.Literal, consumeRawStringLiteral(stream), start)
                ch == '`' -> tokens += Token(TokenType.Literal, consumeLiteral(stream), start)
                ch == '<' || ch == '>' || ch == '=' || ch == '!' -> tokens += consumeOperator(stream)
                ch == ' ' || ch == '\t' || ch == '\n' -> current++
                ch == '&' -> {
                    current++
                    tokens += if (stream.getOrNull(current) == '&') {
                        current++
                        Token(TokenType.And, "&&", start)
                    } else {
                        Token(TokenType.Expref, "&", start)
                    }
                }
                ch == '|' -> {
                    current++
                    tokens += if (stream.getOrNull(current) == '|') {
                        current++
                        Token(TokenType.Or, "||", start)
                    } else {
                        Token(TokenType.Pipe, "|", start)
                    }
                }
                else -> throw syntaxError("Unknown character:$ch")
            }
        }
        return tokens
    }

    private fun consumeUnquotedIdentifier(stream: String): String {
        val start = current
        current++
        while (current < stream.length && isAlphaNum(stream[current])) current++
        return stream.substring(start, current)
    }

    /** Skips a quoted body, where a backslash escapes [quote] or another backslash. */
    private fun skipQuoted(stream: String, quote: Char) {
        while (current < stream.length && stream[current] != quote) {
            val next = stream.getOrNull(current + 1)
            current += if (stream[current] == '\\' && (next == '\\' || next == quote)) 2 else 1
        }
    }

    private fun consumeQuotedIdentifier(stream: String): Any? {
        val start = current
        current++
        skipQuoted(stream, '"')
        current++
        return parseJson(stream.substring(start, minOf(current, stream.length)))
    }

    /** An unterminated raw string runs to the end of the expression, as in jmespath. */
    private fun consumeRawStringLiteral(stream: String): String {
        val start = current
        current++
        skipQuoted(stream, '\'')
        current++
        return stream.substring(start + 1, minOf(current - 1, stream.length)).replaceFirst("\\'", "'")
    }

    private fun consumeNumber(stream: String): Token {
        val start = current
        current++
        while (current < stream.length && isNum(stream[current])) current++
        return Token(TokenType.Number, parseInt(stream.substring(start, current)), start)
    }

    private fun consumeLBracket(stream: String): Token {
        val start = current
        current++
        return when (stream.getOrNull(current)) {
            '?' -> {
                current++
                Token(TokenType.Filter, "[?", start)
            }
            ']' -> {
                current++
                Token(TokenType.Flatten, "[]", start)
            }
            else -> Token(TokenType.Lbracket, "[", start)
        }
    }

    /**
     * A lone `=` is the one character jmespath accepts here without making a token: it pushes
     * `undefined`, and parsing fails on it with a TypeError, which is a syntax error. That error
     * is raised here.
     */
    private fun consumeOperator(stream: String): Token {
        val start = current
        val startingChar = stream[start]
        current++
        val equalsFollows = stream.getOrNull(current) == '='
        if (equalsFollows) current++
        return when (startingChar) {
            '!' -> if (equalsFollows) Token(TokenType.NE, "!=", start) else Token(TokenType.Not, "!", start)
            '<' -> if (equalsFollows) Token(TokenType.LTE, "<=", start) else Token(TokenType.LT, "<", start)
            '>' -> if (equalsFollows) Token(TokenType.GTE, ">=", start) else Token(TokenType.GT, ">", start)
            else -> if (equalsFollows) Token(TokenType.EQ, "==", start) else throw syntaxError("Unknown operator: =")
        }
    }

    private fun consumeLiteral(stream: String): Any? {
        current++
        val start = current
        skipQuoted(stream, '`')
        val literalString = stream.substring(start, current).trimStart(::isJsWhiteSpace).replaceFirst("\\`", "`")
        val literal = if (looksLikeJson(literalString)) parseJson(literalString) else parseJson("\"$literalString\"")
        current++
        return literal
    }

    private fun looksLikeJson(literalString: String): Boolean = when {
        literalString.isEmpty() -> false
        literalString[0] in "[{\"" -> true
        literalString == "true" || literalString == "false" || literalString == "null" -> true
        literalString[0] in "-0123456789" -> try {
            parseJson(literalString)
            true
        } catch (e: Throwable) {
            false
        }
        else -> false
    }
}
