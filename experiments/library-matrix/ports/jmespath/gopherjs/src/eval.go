package main

import "github.com/gopherjs/gopherjs/js"
import "strings"

func one(a value) value {
	xs := arr()
	push(xs, a)
	return xs
}
func two(a value, b value) value {
	xs := one(a)
	push(xs, b)
	return xs
}
func three(a value, b value, c value) value {
	xs := two(a, b)
	push(xs, c)
	return xs
}
func node(t string) value {
	n := obj()
	put(n, "type", str(t))
	return n
}
func named(t string, name value) value {
	n := node(t)
	put(n, "name", name)
	return n
}
func leaf(t string, literalValue value) value {
	n := node(t)
	put(n, "value", literalValue)
	return n
}
func branch(t string, xs value) value {
	n := node(t)
	put(n, "children", xs)
	return n
}
func child(n value, i int) value {
	return at(field(n, "children"), i)
}
func truth(v value) bool {
	t := kind(v)
	if t == "null" {
		return false
	}
	if t == "boolean" {
		return flag(v)
	}
	if t == "array" || t == "string" {
		return size(v) != 0
	}
	if t == "object" || t == "expref" {
		return size(keys(v)) != 0
	}
	return true
}
func equal(a value, b value) bool {
	if same(a, b) {
		return true
	}
	t := kind(a)
	if t != kind(b) {
		return false
	}
	if t == "array" {
		if size(a) != size(b) {
			return false
		}
		i := 0
		for i < size(a) {
			if !equal(at(a, i), at(b, i)) {
				return false
			}
			i += 1
		}
		return true
	}
	if t == "object" {
		ks := keys(a)
		if size(ks) != size(keys(b)) {
			return false
		}
		i := 0
		for i < size(ks) {
			key := stringValue(at(ks, i))
			if !equal(field(a, key), field(b, key)) {
				return false
			}
			i += 1
		}
		return true
	}
	return false
}
func cap(v value, defaultValue, step, length float64) float64 {
	if isNil(v) {
		return defaultValue
	}
	y := toNumber(v)
	if y < 0 {
		y += length
	}
	if step < 0 {
		return clamp(y, -1, length-1)
	}
	return clamp(y, 0, length)
}
func evaluate(n value, v value) value {
	k := stringValue(field(n, "type"))
	if k == "Field" {
		if kind(v) != "object" && kind(v) != "expref" {
			return nil
		}
		result := field(v, stringValue(field(n, "name")))
		if missing(result) {
			return nil
		}
		return result
	}
	if k == "Current" || k == "Identity" {
		return v
	}
	if k == "Literal" {
		return field(n, "value")
	}
	if k == "Pipe" || k == "Subexpression" || k == "IndexExpression" {
		return evaluate(child(n, 1), evaluate(child(n, 0), v))
	}
	if k == "Index" {
		if kind(v) != "array" {
			return nil
		}
		ix := field(n, "value")
		if nan(toNumber(ix)) {
			return nil
		}
		i := toNumber(ix)
		if i < 0 {
			i += float64(size(v))
		}
		if i < 0 || i >= float64(size(v)) {
			return nil
		}
		return at(v, int(i))
	}
	if k == "Slice" {
		if kind(v) != "array" {
			return nil
		}
		p := child(n, 2)
		step := 1.0
		if !isNil(p) {
			step = toNumber(p)
		}
		if step == 0 {
			panic("invalid-value")
		}
		end := float64(size(v))
		startDefault := 0.0
		stopDefault := end
		if step < 0 {
			startDefault = end - 1
			stopDefault = -1
		}
		i := cap(child(n, 0), startDefault, step, end)
		stop := cap(child(n, 1), stopDefault, step, end)
		out := arr()
		for (step > 0 && i < stop) || (step < 0 && i > stop) {
			push(out, at(v, int(i)))
			i += step
		}
		return out
	}
	if k == "Projection" || k == "ValueProjection" || k == "FilterProjection" || k == "Flatten" {
		base := evaluate(child(n, 0), v)
		if k == "ValueProjection" {
			if kind(base) != "object" && kind(base) != "expref" {
				return nil
			}
			base = objectValues(base)
		}
		if kind(base) != "array" {
			return nil
		}
		out := arr()
		i := 0
		for i < size(base) {
			item := at(base, i)
			if k == "Flatten" {
				if kind(item) == "array" {
					j := 0
					for j < size(item) {
						push(out, at(item, j))
						j += 1
					}
				} else {
					push(out, item)
				}
			} else if k != "FilterProjection" || truth(evaluate(child(n, 2), item)) {
				x := evaluate(child(n, 1), item)
				if !isNil(x) {
					push(out, x)
				}
			}
			i += 1
		}
		return out
	}
	if k == "Comparator" {
		a := evaluate(child(n, 0), v)
		b := evaluate(child(n, 1), v)
		op := stringValue(field(n, "name"))
		if op == "EQ" {
			return boolValue(equal(a, b))
		}
		if op == "NE" {
			return boolValue(!equal(a, b))
		}
		ord := compare(a, b)
		return boolValue((ord == -1 && (op == "LT" || op == "LTE")) || (ord == 1 && (op == "GT" || op == "GTE")) || (ord == 0 && (op == "LTE" || op == "GTE")))
	}
	if k == "NotExpression" {
		return boolValue(!truth(evaluate(child(n, 0), v)))
	}
	if k == "AndExpression" || k == "OrExpression" {
		left := evaluate(child(n, 0), v)
		if truth(left) == (k == "AndExpression") {
			return evaluate(child(n, 1), v)
		}
		return left
	}
	if k == "ExpressionReference" {
		result := child(n, 0)
		put(result, "jmespathType", str("Expref"))
		return result
	}
	cs := field(n, "children")
	if k == "Function" {
		args := arr()
		i := 0
		for i < size(cs) {
			push(args, evaluate(at(cs, i), v))
			i += 1
		}
		return callFunction(stringValue(field(n, "name")), args)
	}
	if k == "MultiSelectList" || k == "MultiSelectHash" {
		if isNil(v) {
			return nil
		}
		out := arr()
		if k == "MultiSelectHash" {
			out = obj()
		}
		i := 0
		for i < size(cs) {
			c := at(cs, i)
			if k == "MultiSelectHash" {
				put(out, stringValue(field(c, "name")), evaluate(field(c, "value"), v))
			} else {
				push(out, evaluate(c, v))
			}
			i += 1
		}
		return out
	}
	panic("syntax")
}
func objectValues(v value) value {
	ks := keys(v)
	result := arr()
	i := 0
	for i < size(ks) {
		push(result, field(v, stringValue(at(ks, i))))
		i += 1
	}
	return result
}
func matchesType(v value, t string) bool {
	if t == "any" {
		return true
	}
	if t == "numbers" || t == "strings" {
		if kind(v) != "array" {
			return false
		}
		i := 0
		for i < size(v) {
			if kind(at(v, i))+"s" != t {
				return false
			}
			i += 1
		}
		return true
	}
	return kind(v) == t
}
func callFunction(name string, args value) value {
	checkArguments(name, args)
	a := at(args, 0)
	b := at(args, 1)
	if name == "type" {
		return str(kind(a))
	}
	if name == "length" {
		if kind(a) == "object" {
			return intValue(size(keys(a)))
		}
		return intValue(size(a))
	}
	if name == "keys" {
		return keys(a)
	}
	if name == "values" {
		return objectValues(a)
	}
	if name == "to_array" {
		if kind(a) == "array" {
			return a
		}
		return one(a)
	}
	if name == "to_number" {
		if kind(a) == "number" {
			return a
		}
		if kind(a) == "string" && !nan(toNumber(a)) {
			return numeric(toNumber(a))
		}
		return nil
	}
	if name == "to_string" {
		if kind(a) == "string" {
			return a
		}
		return jsonText(a)
	}
	if name == "not_null" {
		i := 0
		for i < size(args) {
			if !isNil(at(args, i)) {
				return at(args, i)
			}
			i += 1
		}
		return nil
	}
	if name == "sum" || name == "avg" {
		sum := 0.0
		i := 0
		for i < size(a) {
			sum += toNumber(at(a, i))
			i += 1
		}
		if name == "avg" {
			return numeric(sum / float64(size(a)))
		}
		return numeric(sum)
	}
	if name == "min" || name == "max" {
		if size(a) == 0 {
			return nil
		}
		best := at(a, 0)
		i := 1
		for i < size(a) {
			item := at(a, i)
			ord := compare(item, best)
			if kind(item) == "string" {
				ord = localeOrder(item, best)
			}
			if (name == "min" && ord < 0) || (name == "max" && ord > 0) {
				best = item
			}
			i += 1
		}
		return best
	}
	if name == "max_by" || name == "min_by" {
		extreme := infinity(name == "min_by")
		result := js.Undefined
		i := 0
		for i < size(a) {
			item := at(a, i)
			key := evaluate(b, item)
			t := kind(key)
			if t != "number" && t != "string" {
				panic("invalid-type")
			}
			ord := compare(key, extreme)
			if (ord == -1 && name == "min_by") || (ord == 1 && name == "max_by") {
				extreme = key
				result = item
			}
			i += 1
		}
		return result
	}
	if name == "merge" {
		out := obj()
		i := 0
		for i < size(args) {
			item := at(args, i)
			if !isNil(item) {
				ks := keys(item)
				j := 0
				for j < size(ks) {
					key := stringValue(at(ks, j))
					put(out, key, field(item, key))
					j += 1
				}
			}
			i += 1
		}
		return out
	}
	if name == "map" {
		out := arr()
		i := 0
		for i < size(b) {
			push(out, evaluate(a, at(b, i)))
			i += 1
		}
		return out
	}
	if name == "sort_by" {
		return sortByExpression(a, b)
	}
	return primitiveFunction(name, a, b)
}

// Each element is an alternative type list for a required argument. As in
// 0.16.0, the two variadic functions validate their first argument here.
var signatures = map[string][]string{
	"abs": {"number"}, "ceil": {"number"}, "floor": {"number"},
	"avg": {"numbers"}, "sum": {"numbers"},
	"contains": {"string|array", "any"}, "starts_with": {"string", "string"}, "ends_with": {"string", "string"},
	"length": {"object|string|array"}, "map": {"expref", "array"},
	"max": {"numbers|strings"}, "min": {"numbers|strings"}, "sort": {"numbers|strings"},
	"keys": {"object"}, "values": {"object"}, "merge": {"object"},
	"max_by": {"array", "expref"}, "min_by": {"array", "expref"}, "sort_by": {"array", "expref"},
	"join": {"string", "strings"}, "reverse": {"array|string"},
	"to_array": {"any"}, "to_string": {"any"}, "to_number": {"any"}, "type": {"any"}, "not_null": {"any"},
}

func checkArguments(name string, args value) {
	sig, ok := signatures[name]
	if !ok {
		panic("unknown-function")
	}
	count := size(args)
	if count < len(sig) || (count > len(sig) && name != "not_null" && name != "merge") {
		panic("invalid-arity")
	}
	for i, allowed := range sig {
		valid := false
		for _, t := range strings.Split(allowed, "|") {
			if matchesType(at(args, i), t) {
				valid = true
				break
			}
		}
		if !valid {
			panic("invalid-type")
		}
	}
}
