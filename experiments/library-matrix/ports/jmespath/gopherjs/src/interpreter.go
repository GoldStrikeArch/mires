// jmespath's TreeInterpreter and the value helpers it uses (strictDeepEqual, isFalse, objValues).

package main

import "github.com/gopherjs/gopherjs/js"

// strictDeepEqual is jmespath's: ===, then the same Object.prototype.toString tag, then by content.
// Objects compare every own key of first against second[key] (so an undefined value equals a
// missing key), then require second's own keys to be among them.
func strictDeepEqual(first, second *js.Object) bool {
	if strictEquals(first, second) {
		return true
	}
	firstType := typeTag(first)
	if firstType != typeTag(second) {
		return false
	}
	switch firstType {
	case "[object Array]":
		if first.Length() != second.Length() {
			return false
		}
		for i := 0; i < first.Length(); i++ {
			if !strictDeepEqual(first.Index(i), second.Index(i)) {
				return false
			}
		}
		return true
	case "[object Object]":
		keysSeen := js.Global.Get("Set").New()
		for _, key := range elements(objectKeys(first)) {
			if !strictDeepEqual(property(first, key), property(second, key)) {
				return false
			}
			keysSeen.Call("add", key)
		}
		for _, key := range elements(objectKeys(second)) {
			if !keysSeen.Call("has", key).Bool() {
				return false
			}
		}
		return true
	}
	return false
}

// isFalse is jmespath's: "", false, null, an empty array and an object with no own keys.
func isFalse(v *js.Object) bool {
	switch typeTag(v) {
	case "[object String]":
		return v.Length() == 0
	case "[object Boolean]":
		return !v.Bool()
	case "[object Null]":
		return true
	case "[object Array]":
		return v.Length() == 0
	case "[object Object]":
		return objectKeys(v).Length() == 0
	}
	return false
}

func objValues(object *js.Object) []*js.Object {
	keys := elements(objectKeys(object))
	values := make([]*js.Object, len(keys))
	for i, key := range keys {
		values[i] = property(object, key)
	}
	return values
}

// childOf is node.children[i].
func childOf(node *js.Object, i int) *js.Object { return node.Get("children").Index(i) }

// visit is jmespath's TreeInterpreter.visit.
func visit(node, value *js.Object) (*js.Object, error) {
	// node.type of a missing node is a TypeError in jmespath.
	if isUndefined(node) || isNull(node) {
		return nil, syntaxError("Cannot read properties of undefined")
	}
	switch node.Get("type").String() {
	case "Field":
		if !isObject(value) {
			return nil, nil
		}
		if field := property(value, node.Get("name")); !isUndefined(field) {
			return field, nil
		}
		return nil, nil
	case "Subexpression", "IndexExpression", "Pipe":
		left, err := visit(childOf(node, 0), value)
		if err != nil {
			return nil, err
		}
		return visit(childOf(node, 1), left)
	case "Index":
		if !isArray(value) {
			return nil, nil
		}
		index := node.Get("value").Float()
		if index < 0 {
			index += float64(value.Length())
		}
		if result := elementAt(value, index); !isUndefined(result) {
			return result, nil
		}
		return nil, nil
	case "Slice":
		if !isArray(value) {
			return nil, nil
		}
		return slice(value, node.Get("children"))
	case "Projection":
		base, err := visit(childOf(node, 0), value)
		if err != nil || !isArray(base) {
			return nil, err
		}
		return collect(elements(base), childOf(node, 1))
	case "ValueProjection":
		base, err := visit(childOf(node, 0), value)
		if err != nil || !isObject(base) {
			return nil, err
		}
		return collect(objValues(base), childOf(node, 1))
	case "FilterProjection":
		base, err := visit(childOf(node, 0), value)
		if err != nil || !isArray(base) {
			return nil, err
		}
		// Every condition first, then the projection: the first error is jmespath's.
		var filtered []*js.Object
		for _, item := range elements(base) {
			matched, err := visit(childOf(node, 2), item)
			if err != nil {
				return nil, err
			}
			if !isFalse(matched) {
				filtered = append(filtered, item)
			}
		}
		return collect(filtered, childOf(node, 1))
	case "Comparator":
		first, err := visit(childOf(node, 0), value)
		if err != nil {
			return nil, err
		}
		second, err := visit(childOf(node, 1), value)
		if err != nil {
			return nil, err
		}
		switch node.Get("name").String() {
		case "EQ":
			return jsBool(strictDeepEqual(first, second)), nil
		case "NE":
			return jsBool(!strictDeepEqual(first, second)), nil
		case "GT":
			return jsBool(greater(first, second)), nil
		case "GTE":
			return jsBool(greaterOrEqual(first, second)), nil
		case "LT":
			return jsBool(less(first, second)), nil
		case "LTE":
			return jsBool(lessOrEqual(first, second)), nil
		}
		return nil, syntaxError("Unknown comparator")
	case "Flatten":
		original, err := visit(childOf(node, 0), value)
		if err != nil || !isArray(original) {
			return nil, err
		}
		merged := newArray()
		for _, current := range elements(original) {
			if isArray(current) {
				for _, element := range elements(current) {
					merged.Call("push", element)
				}
			} else {
				merged.Call("push", current)
			}
		}
		return merged, nil
	case "Identity", "Current":
		return value, nil
	case "MultiSelectList":
		if isNull(value) {
			return nil, nil
		}
		collected := newArray()
		for _, element := range elements(node.Get("children")) {
			result, err := visit(element, value)
			if err != nil {
				return nil, err
			}
			collected.Call("push", result)
		}
		return collected, nil
	case "MultiSelectHash":
		if isNull(value) {
			return nil, nil
		}
		collected := newObject()
		for _, pair := range elements(node.Get("children")) {
			result, err := visit(pair.Get("value"), value)
			if err != nil {
				return nil, err
			}
			setProperty(collected, pair.Get("name"), result)
		}
		return collected, nil
	case "OrExpression":
		matched, err := visit(childOf(node, 0), value)
		if err != nil || !isFalse(matched) {
			return matched, err
		}
		return visit(childOf(node, 1), value)
	case "AndExpression":
		first, err := visit(childOf(node, 0), value)
		if err != nil || isFalse(first) {
			return first, err
		}
		return visit(childOf(node, 1), value)
	case "NotExpression":
		first, err := visit(childOf(node, 0), value)
		if err != nil {
			return nil, err
		}
		return jsBool(isFalse(first)), nil
	case "Literal":
		return node.Get("value"), nil
	case "Function":
		children := elements(node.Get("children"))
		resolvedArgs := make([]*js.Object, len(children))
		for i, argument := range children {
			var err error
			if resolvedArgs[i], err = visit(argument, value); err != nil {
				return nil, err
			}
		}
		return callFunction(node.Get("name"), resolvedArgs)
	case "ExpressionReference":
		// jmespath tags the referenced node in place and returns the node itself.
		refNode := childOf(node, 0)
		if isUndefined(refNode) || isNull(refNode) {
			return nil, syntaxError("Cannot set properties of undefined")
		}
		refNode.Set("jmespathType", "Expref")
		return refNode, nil
	}
	return nil, syntaxError("Unknown node type")
}

// collect is a projection's right side over each item; it keeps every result except null, so
// undefined (an empty max_by) is kept.
func collect(items []*js.Object, right *js.Object) (*js.Object, error) {
	collected := newArray()
	for _, item := range items {
		current, err := visit(right, item)
		if err != nil {
			return nil, err
		}
		if !isNull(current) {
			collected.Call("push", current)
		}
	}
	return collected, nil
}

// slice is Slice with jmespath's computeSliceParams, in JS numbers: the tokens come from parseInt, so
// a bound may be NaN (`[0:3:-]`) or past 2^53, and the loop reads value[i] as it goes.
func slice(array, params *js.Object) (*js.Object, error) {
	length := float64(array.Length())
	step := 1.0
	if p := params.Index(2); !isNull(p) {
		if step = p.Float(); step == 0 {
			return nil, runtimeError("Invalid slice, step cannot be 0")
		}
	}
	stepValueNegative := step < 0
	start, stop := sliceBound(params.Index(0), 0, length, step), sliceBound(params.Index(1), length, length, step)
	if stepValueNegative {
		start, stop = sliceBound(params.Index(0), length-1, length, step), sliceBound(params.Index(1), -1, length, step)
	}
	result := newArray()
	if step > 0 {
		for i := start; i < stop; i += step {
			result.Call("push", elementAt(array, i))
		}
	} else {
		for i := start; i > stop; i += step {
			result.Call("push", elementAt(array, i))
		}
	}
	return result, nil
}

// sliceBound is a slice's start or stop: its default when the part is absent, else capSliceRange.
func sliceBound(part *js.Object, fallback, length, step float64) float64 {
	if isNull(part) {
		return fallback
	}
	return capSliceRange(length, part.Float(), step)
}

func capSliceRange(arrayLength, actualValue, step float64) float64 {
	if actualValue < 0 {
		actualValue += arrayLength
		if actualValue < 0 {
			if step < 0 {
				return -1
			}
			return 0
		}
	} else if actualValue >= arrayLength {
		if step < 0 {
			return arrayLength - 1
		}
		return arrayLength
	}
	return actualValue
}
