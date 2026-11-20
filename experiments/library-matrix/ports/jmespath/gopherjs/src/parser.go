// jmespath's Pratt parser, method by method. Nodes are plain JS objects with jmespath's property
// order, since compile returns them and `&expr` passes them around as values. js.Undefined stands
// for jmespath's undefined nodes (`a.`, `()`).

package main

import "github.com/gopherjs/gopherjs/js"

func newNode(typ string) *js.Object {
	node := newObject()
	node.Set("type", typ)
	return node
}

// nodeWith is a node with one more property after its type: Field's name, Literal's value.
func nodeWith(typ, key string, value *js.Object) *js.Object {
	node := newNode(typ)
	node.Set(key, value)
	return node
}

func branch(typ string, children ...*js.Object) *js.Object {
	node := newNode(typ)
	node.Set("children", newArray(children...))
	return node
}

// binaryNodeTypes are the node types of the binary operators led builds alike.
var binaryNodeTypes = map[tokenType]string{tokPipe: "Pipe", tokOr: "OrExpression", tokAnd: "AndExpression"}

type parser struct {
	tokens []token
	index  int
}

// compile is jmespath's compile: the AST of an expression.
func compile(expression *js.Object) (*js.Object, error) {
	tokens, err := tokenize(expression)
	if err != nil {
		return nil, err
	}
	p := &parser{tokens: append(tokens, token{tokEOF, jsString(""), expression.Length()})}
	ast, err := p.expression(0)
	if err != nil {
		return nil, err
	}
	if p.lookahead(0) != tokEOF {
		return nil, syntaxError("Unexpected token")
	}
	return ast, nil
}

func (p *parser) expression(rbp int) (*js.Object, error) {
	leftToken := p.lookaheadToken(0)
	p.advance()
	left, err := p.nud(leftToken)
	if err != nil {
		return nil, err
	}
	currentToken := p.lookahead(0)
	// `rbp < bindingPower[currentToken]`, false where the table has no entry.
	for power, ok := bindingPower[currentToken]; ok && rbp < power; power, ok = bindingPower[currentToken] {
		p.advance()
		if left, err = p.led(currentToken, left); err != nil {
			return nil, err
		}
		currentToken = p.lookahead(0)
	}
	return left, nil
}

func (p *parser) lookahead(number int) tokenType  { return p.tokens[p.index+number].typ }
func (p *parser) lookaheadToken(number int) token { return p.tokens[p.index+number] }
func (p *parser) advance()                        { p.index++ }

func (p *parser) match(typ tokenType) error {
	if p.lookahead(0) != typ {
		return syntaxError("Unexpected token")
	}
	p.advance()
	return nil
}

func (p *parser) nud(t token) (*js.Object, error) {
	switch t.typ {
	case tokLiteral:
		return nodeWith("Literal", "value", t.value), nil
	case tokUnquotedIdentifier:
		return nodeWith("Field", "name", t.value), nil
	case tokQuotedIdentifier:
		field := nodeWith("Field", "name", t.value)
		if p.lookahead(0) == tokLparen {
			return nil, syntaxError("Quoted identifier not allowed for function names.")
		}
		return field, nil
	case tokNot:
		right, err := p.expression(bindingPower[tokNot])
		if err != nil {
			return nil, err
		}
		return branch("NotExpression", right), nil
	case tokStar:
		right := newNode("Identity")
		if p.lookahead(0) != tokRbracket {
			var err error
			if right, err = p.parseProjectionRHS(bindingPower[tokStar]); err != nil {
				return nil, err
			}
		}
		return branch("ValueProjection", newNode("Identity"), right), nil
	case tokFilter:
		return p.led(t.typ, newNode("Identity"))
	case tokLbrace:
		return p.parseMultiselectHash()
	case tokFlatten:
		left := branch("Flatten", newNode("Identity"))
		right, err := p.parseProjectionRHS(bindingPower[tokFlatten])
		if err != nil {
			return nil, err
		}
		return branch("Projection", left, right), nil
	case tokLbracket:
		switch {
		case p.lookahead(0) == tokNumber || p.lookahead(0) == tokColon:
			right, err := p.parseIndexExpression()
			if err != nil {
				return nil, err
			}
			return p.projectIfSlice(newNode("Identity"), right)
		case p.lookahead(0) == tokStar && p.lookahead(1) == tokRbracket:
			p.advance()
			p.advance()
			right, err := p.parseProjectionRHS(bindingPower[tokStar])
			if err != nil {
				return nil, err
			}
			return branch("Projection", newNode("Identity"), right), nil
		}
		return p.parseMultiselectList()
	case tokCurrent:
		return newNode("Current"), nil
	case tokExpref:
		expression, err := p.expression(bindingPower[tokExpref])
		if err != nil {
			return nil, err
		}
		return branch("ExpressionReference", expression), nil
	case tokLparen:
		args, err := p.parseArguments(false)
		if err != nil {
			return nil, err
		}
		// args[0]: undefined for `()`.
		if len(args) == 0 {
			return js.Undefined, nil
		}
		return args[0], nil
	}
	return nil, syntaxError("Invalid token")
}

func (p *parser) led(tokenName tokenType, left *js.Object) (*js.Object, error) {
	switch tokenName {
	case tokDot:
		rbp := bindingPower[tokDot]
		if p.lookahead(0) != tokStar {
			right, err := p.parseDotRHS(rbp)
			if err != nil {
				return nil, err
			}
			return branch("Subexpression", left, right), nil
		}
		p.advance()
		right, err := p.parseProjectionRHS(rbp)
		if err != nil {
			return nil, err
		}
		return branch("ValueProjection", left, right), nil
	case tokPipe, tokOr, tokAnd:
		right, err := p.expression(bindingPower[tokenName])
		if err != nil {
			return nil, err
		}
		return branch(binaryNodeTypes[tokenName], left, right), nil
	case tokLparen:
		// left.name: a TypeError (syntax) when there is no left node, as after `()`.
		if isUndefined(left) || isNull(left) {
			return nil, syntaxError("Function call without a name")
		}
		name := left.Get("name")
		args, err := p.parseArguments(true)
		if err != nil {
			return nil, err
		}
		function := nodeWith("Function", "name", name)
		function.Set("children", newArray(args...))
		return function, nil
	case tokFilter:
		condition, err := p.expression(0)
		if err != nil {
			return nil, err
		}
		if err := p.match(tokRbracket); err != nil {
			return nil, err
		}
		var right *js.Object
		if p.lookahead(0) == tokFlatten {
			right = newNode("Identity")
		} else if right, err = p.parseProjectionRHS(bindingPower[tokFilter]); err != nil {
			return nil, err
		}
		return branch("FilterProjection", left, right, condition), nil
	case tokFlatten:
		leftNode := branch("Flatten", left)
		right, err := p.parseProjectionRHS(bindingPower[tokFlatten])
		if err != nil {
			return nil, err
		}
		return branch("Projection", leftNode, right), nil
	case tokEQ, tokNE, tokGT, tokGTE, tokLT, tokLTE:
		return p.parseComparator(left, tokenName)
	case tokLbracket:
		if t := p.lookaheadToken(0); t.typ == tokNumber || t.typ == tokColon {
			right, err := p.parseIndexExpression()
			if err != nil {
				return nil, err
			}
			return p.projectIfSlice(left, right)
		}
		if err := p.match(tokStar); err != nil {
			return nil, err
		}
		if err := p.match(tokRbracket); err != nil {
			return nil, err
		}
		right, err := p.parseProjectionRHS(bindingPower[tokStar])
		if err != nil {
			return nil, err
		}
		return branch("Projection", left, right), nil
	}
	return nil, syntaxError("Invalid token")
}

// parseArguments is the argument loop of `(…)` and of a function call. An argument that starts with
// `@` is the bare Current node, so `abs(@.a)` fails on the `.` as in jmespath. Calls may separate
// arguments with commas; parentheses may not.
func (p *parser) parseArguments(commas bool) ([]*js.Object, error) {
	var args []*js.Object
	for p.lookahead(0) != tokRparen {
		var expression *js.Object
		if p.lookahead(0) == tokCurrent {
			p.advance()
			expression = newNode("Current")
		} else {
			var err error
			if expression, err = p.expression(0); err != nil {
				return nil, err
			}
		}
		if commas && p.lookahead(0) == tokComma {
			p.advance()
		}
		args = append(args, expression)
	}
	return args, p.match(tokRparen)
}

func (p *parser) parseIndexExpression() (*js.Object, error) {
	if p.lookahead(0) == tokColon || p.lookahead(1) == tokColon {
		return p.parseSliceExpression()
	}
	index := nodeWith("Index", "value", p.lookaheadToken(0).value)
	p.advance()
	return index, p.match(tokRbracket)
}

func (p *parser) projectIfSlice(left, right *js.Object) (*js.Object, error) {
	indexExpr := branch("IndexExpression", left, right)
	if right.Get("type").String() != "Slice" {
		return indexExpr, nil
	}
	rhs, err := p.parseProjectionRHS(bindingPower[tokStar])
	if err != nil {
		return nil, err
	}
	return branch("Projection", indexExpr, rhs), nil
}

// parseSliceExpression: `[start:stop:step]`, each part optional; a later number in one part
// replaces an earlier one.
func (p *parser) parseSliceExpression() (*js.Object, error) {
	parts := []*js.Object{nil, nil, nil}
	index := 0
	for currentToken := p.lookahead(0); currentToken != tokRbracket && index < 3; currentToken = p.lookahead(0) {
		switch currentToken {
		case tokColon:
			index++
		case tokNumber:
			parts[index] = p.lookaheadToken(0).value
		default:
			return nil, syntaxError("Syntax error, unexpected token")
		}
		p.advance()
	}
	return branch("Slice", parts...), p.match(tokRbracket)
}

func (p *parser) parseComparator(left *js.Object, comparator tokenType) (*js.Object, error) {
	right, err := p.expression(bindingPower[comparator])
	if err != nil {
		return nil, err
	}
	node := nodeWith("Comparator", "name", jsString(comparatorNames[comparator]))
	node.Set("children", newArray(left, right))
	return node, nil
}

// parseDotRHS is undefined (no error) when nothing that can follow a dot follows it, as in jmespath.
func (p *parser) parseDotRHS(rbp int) (*js.Object, error) {
	switch p.lookahead(0) {
	case tokUnquotedIdentifier, tokQuotedIdentifier, tokStar:
		return p.expression(rbp)
	case tokLbracket:
		p.advance()
		return p.parseMultiselectList()
	case tokLbrace:
		p.advance()
		return p.parseMultiselectHash()
	}
	return js.Undefined, nil
}

func (p *parser) parseProjectionRHS(rbp int) (*js.Object, error) {
	next := p.lookahead(0)
	if power, ok := bindingPower[next]; ok && power < 10 {
		return newNode("Identity"), nil
	}
	switch next {
	case tokLbracket, tokFilter:
		return p.expression(rbp)
	case tokDot:
		p.advance()
		return p.parseDotRHS(rbp)
	}
	return nil, syntaxError("Sytanx error, unexpected token")
}

// parseMultiselectList: `[a, b]`; the comma between elements is optional, a trailing one is an
// error.
func (p *parser) parseMultiselectList() (*js.Object, error) {
	var expressions []*js.Object
	for p.lookahead(0) != tokRbracket {
		expression, err := p.expression(0)
		if err != nil {
			return nil, err
		}
		expressions = append(expressions, expression)
		if p.lookahead(0) == tokComma {
			p.advance()
			if p.lookahead(0) == tokRbracket {
				return nil, syntaxError("Unexpected token Rbracket")
			}
		}
	}
	p.advance()
	return branch("MultiSelectList", expressions...), nil
}

// parseMultiselectHash: `{k: v, …}`; as in jmespath, the comma between pairs is optional too.
func (p *parser) parseMultiselectHash() (*js.Object, error) {
	var pairs []*js.Object
	for {
		keyToken := p.lookaheadToken(0)
		if keyToken.typ != tokUnquotedIdentifier && keyToken.typ != tokQuotedIdentifier {
			return nil, syntaxError("Expecting an identifier token")
		}
		p.advance()
		if err := p.match(tokColon); err != nil {
			return nil, err
		}
		value, err := p.expression(0)
		if err != nil {
			return nil, err
		}
		pair := nodeWith("KeyValuePair", "name", keyToken.value)
		pair.Set("value", value)
		pairs = append(pairs, pair)
		if p.lookahead(0) == tokComma {
			p.advance()
		} else if p.lookahead(0) == tokRbrace {
			p.advance()
			return branch("MultiSelectHash", pairs...), nil
		}
	}
}
