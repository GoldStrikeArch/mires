package main

import "github.com/gopherjs/gopherjs/js"

func (p *parser) current(offset int) string {
	return p.tokenAt(p.index + offset).kind
}
func (p *parser) next() token {
	result := p.tokenAt(p.index)
	if result.kind == "EOF" {
		panic("syntax")
	}
	p.index += 1
	return result
}
func (p *parser) take(kind string) {
	if p.next().kind != kind {
		panic("syntax")
	}
}
func (p *parser) power(k string) int {
	if k == "Pipe" {
		return 1
	}
	if k == "Or" {
		return 2
	}
	if k == "And" {
		return 3
	}
	if k == "EQ" || k == "NE" || k == "GT" || k == "LT" || k == "GTE" || k == "LTE" {
		return 5
	}
	if k == "Flatten" {
		return 9
	}
	if k == "Star" {
		return 20
	}
	if k == "Filter" {
		return 21
	}
	if k == "Dot" {
		return 40
	}
	if k == "Not" {
		return 45
	}
	if k == "Lbrace" {
		return 50
	}
	if k == "Lbracket" {
		return 55
	}
	if k == "Lparen" {
		return 60
	}
	return 0
}
func (p *parser) parse() value {
	ast := p.expression(0)
	if p.current(0) != "EOF" {
		panic("syntax")
	}
	return ast
}
func (p *parser) expression(rbp int) value {
	left := p.applyPrefix(p.next())
	for p.power(p.current(0)) > rbp {
		op := p.next().kind
		left = p.applyInfix(op, left)
	}
	return left
}
func (p *parser) projection(rbp int) value {
	k := p.current(0)
	if p.power(k) < 10 {
		return node("Identity")
	}
	if k == "Lbracket" || k == "Filter" {
		return p.expression(rbp)
	}
	if k == "Dot" {
		p.next()
		return p.dot(rbp)
	}
	panic("syntax")
}
func (p *parser) dot(rbp int) value {
	k := p.current(0)
	if k == "Field" || k == "Quoted" || k == "Star" {
		return p.expression(rbp)
	}
	if k == "Lbracket" {
		p.next()
		return p.listExpression()
	}
	if k == "Lbrace" {
		p.next()
		return p.hashExpression()
	}
	return js.Undefined
}
func (p *parser) listExpression() value {
	items := arr()
	for p.current(0) != "Rbracket" {
		push(items, p.expression(0))
		if p.current(0) == "Comma" {
			p.next()
			if p.current(0) == "Rbracket" {
				panic("syntax")
			}
		}
	}
	p.take("Rbracket")
	return branch("MultiSelectList", items)
}
func (p *parser) hashExpression() value {
	items := arr()
	for true {
		key := p.next()
		if key.kind != "Field" && key.kind != "Quoted" {
			panic("syntax")
		}
		p.take("Colon")
		pair := named("KeyValuePair", key.value)
		put(pair, "value", p.expression(0))
		push(items, pair)
		if p.current(0) == "Comma" {
			p.next()
		} else if p.current(0) == "Rbrace" {
			p.next()
			return branch("MultiSelectHash", items)
		}
	}
	panic("syntax")
}
func (p *parser) bracket(left value) value {
	if p.current(0) == "Colon" || p.current(1) == "Colon" {
		parts := three(nil, nil, nil)
		i := 0
		for p.current(0) != "Rbracket" && i < 3 {
			token := p.next()
			if token.kind == "Colon" {
				i += 1
			} else if token.kind == "Number" {
				setIndex(parts, i, token.value)
			} else {
				panic("syntax")
			}
		}
		p.take("Rbracket")
		ix := branch("IndexExpression", two(left, branch("Slice", parts)))
		return branch("Projection", two(ix, p.projection(20)))
	}
	token := p.next()
	p.take("Rbracket")
	return branch("IndexExpression", two(left, leaf("Index", token.value)))
}
func (p *parser) arguments(commas bool) value {
	args := arr()
	for p.current(0) != "Rparen" {
		if p.current(0) == "Current" {
			p.next()
			push(args, node("Current"))
		} else {
			push(args, p.expression(0))
		}
		if commas && p.current(0) == "Comma" {
			p.next()
		}
	}
	p.take("Rparen")
	return args
}
func (p *parser) applyPrefix(t token) value {
	k := t.kind
	if k == "Literal" {
		return leaf("Literal", t.value)
	}
	if k == "Quoted" && p.current(0) == "Lparen" {
		panic("syntax")
	}
	if k == "Quoted" || k == "Field" {
		return named("Field", t.value)
	}
	if k == "Not" {
		return branch("NotExpression", one(p.expression(45)))
	}
	if k == "Current" {
		return node("Current")
	}
	if k == "Expref" {
		return branch("ExpressionReference", one(p.expression(0)))
	}
	if k == "Star" {
		return branch("ValueProjection", two(node("Identity"), p.projection(20)))
	}
	if k == "Filter" || k == "Flatten" {
		return p.applyInfix(k, node("Identity"))
	}
	if k == "Lbrace" {
		return p.hashExpression()
	}
	if k == "Lbracket" {
		if p.current(0) == "Colon" || p.current(0) == "Number" {
			return p.bracket(node("Identity"))
		}
		if p.current(0) == "Star" && p.current(1) == "Rbracket" {
			p.index += 2
			return branch("Projection", two(node("Identity"), p.projection(20)))
		}
		return p.listExpression()
	}
	if k == "Lparen" {
		return at(p.arguments(false), 0)
	}
	panic("syntax")
}
func (p *parser) applyInfix(k string, left value) value {
	if k == "Dot" {
		if p.current(0) == "Star" {
			p.next()
			return branch("ValueProjection", two(left, p.projection(40)))
		}
		return branch("Subexpression", two(left, p.dot(40)))
	}
	if k == "Or" || k == "And" {
		return branch(k+"Expression", two(left, p.expression(p.power(k))))
	}
	if k == "Pipe" {
		return branch(k, two(left, p.expression(1)))
	}
	if k == "Lparen" {
		f := named("Function", field(left, "name"))
		put(f, "children", p.arguments(true))
		return f
	}
	if k == "Filter" {
		condition := p.expression(0)
		p.take("Rbracket")
		rhs := node("Identity")
		if p.current(0) != "Flatten" {
			rhs = p.projection(21)
		}
		return branch("FilterProjection", three(left, rhs, condition))
	}
	if k == "Flatten" {
		return branch("Projection", two(branch("Flatten", one(left)), p.projection(9)))
	}
	if p.power(k) == 5 {
		cmp := named("Comparator", str(k))
		put(cmp, "children", two(left, p.expression(5)))
		return cmp
	}
	if k == "Lbracket" {
		if p.current(0) == "Colon" || p.current(0) == "Number" {
			return p.bracket(left)
		}
		p.take("Star")
		p.take("Rbracket")
		return branch("Projection", two(left, p.projection(20)))
	}
	panic("syntax")
}
