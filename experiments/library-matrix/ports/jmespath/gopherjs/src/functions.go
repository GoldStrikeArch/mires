// jmespath's Runtime: the function table, argument validation and the 26 built-in functions.

package main

import (
	"math"

	"github.com/gopherjs/gopherjs/js"
)

// jmesType is one of jmespath's TYPE_* constants.
type jmesType int

const (
	typeNumber jmesType = iota
	typeAny
	typeString
	typeArray
	typeObject
	typeBoolean
	typeExpref
	typeNull
	typeArrayNumber
	typeArrayString
)

// typeName is jmespath's _getTypeName; ok is false where its switch falls through (undefined).
func typeName(v *js.Object) (t jmesType, ok bool) {
	switch typeTag(v) {
	case "[object String]":
		return typeString, true
	case "[object Number]":
		return typeNumber, true
	case "[object Array]":
		return typeArray, true
	case "[object Boolean]":
		return typeBoolean, true
	case "[object Null]":
		return typeNull, true
	case "[object Object]":
		if tag := v.Get("jmespathType"); isString(tag) && tag.String() == "Expref" {
			return typeExpref, true
		}
		return typeObject, true
	}
	return 0, false
}

// hasType is _getTypeName(v) === t.
func hasType(v *js.Object, t jmesType) bool {
	actual, ok := typeName(v)
	return ok && actual == t
}

type parameter struct {
	types    []jmesType
	variadic bool
}

func params(types ...jmesType) parameter { return parameter{types: types} }

// signatures is jmespath's function table without the functions, which callFunction dispatches
// with a switch: GopherJS compiles a call through a function value as a resumable state machine,
// and so every function that makes one, visit included, which would cost the recursion its depth.
var signatures = map[string][]parameter{
	"abs":         {params(typeNumber)},
	"avg":         {params(typeArrayNumber)},
	"ceil":        {params(typeNumber)},
	"contains":    {params(typeString, typeArray), params(typeAny)},
	"ends_with":   {params(typeString), params(typeString)},
	"floor":       {params(typeNumber)},
	"length":      {params(typeString, typeArray, typeObject)},
	"map":         {params(typeExpref), params(typeArray)},
	"max":         {params(typeArrayNumber, typeArrayString)},
	"merge":       {{types: []jmesType{typeObject}, variadic: true}},
	"max_by":      {params(typeArray), params(typeExpref)},
	"sum":         {params(typeArrayNumber)},
	"starts_with": {params(typeString), params(typeString)},
	"min":         {params(typeArrayNumber, typeArrayString)},
	"min_by":      {params(typeArray), params(typeExpref)},
	"type":        {params(typeAny)},
	"keys":        {params(typeObject)},
	"values":      {params(typeObject)},
	"sort":        {params(typeArrayString, typeArrayNumber)},
	"sort_by":     {params(typeArray), params(typeExpref)},
	"join":        {params(typeString), params(typeArrayString)},
	"reverse":     {params(typeString, typeArray)},
	"to_array":    {params(typeAny)},
	"to_string":   {params(typeAny)},
	"to_number":   {params(typeAny)},
	"not_null":    {{types: []jmesType{typeAny}, variadic: true}},
}

var typeLabels = map[jmesType]string{typeNumber: "number", typeString: "string", typeArray: "array",
	typeObject: "object", typeBoolean: "boolean", typeExpref: "expref", typeNull: "null"}

// call runs a built-in whose arguments validateArgs accepted.
func call(name string, args []*js.Object) (*js.Object, error) {
	switch name {
	case "abs":
		return jsNumber(math.Abs(args[0].Float())), nil
	case "avg":
		return jsNumber(sum(args[0]) / float64(args[0].Length())), nil
	case "ceil":
		return jsNumber(math.Ceil(args[0].Float())), nil
	case "contains":
		// indexOf: String.prototype.indexOf converts its argument with String(); the array
		// version compares with ===, so only the same object is found.
		if isString(args[0]) {
			return jsBool(args[0].Call("indexOf", stringOf(args[1])).Int() >= 0), nil
		}
		for _, element := range elements(args[0]) {
			if strictEquals(element, args[1]) {
				return jsBool(true), nil
			}
		}
		return jsBool(false), nil
	case "ends_with":
		searchStr, suffix := args[0], args[1]
		return jsBool(searchStr.Call("indexOf", suffix, searchStr.Length()-suffix.Length()).Int() != -1), nil
	case "floor":
		return jsNumber(math.Floor(args[0].Float())), nil
	case "length":
		if isObject(args[0]) {
			return jsNumber(float64(objectKeys(args[0]).Length())), nil
		}
		return jsNumber(float64(args[0].Length())), nil
	case "map":
		mapped := newArray()
		for _, element := range elements(args[1]) {
			result, err := visit(args[0], element)
			if err != nil {
				return nil, err
			}
			mapped.Call("push", result)
		}
		return mapped, nil
	case "max":
		return extreme(elements(args[0]), true), nil
	case "merge":
		return merge(args), nil
	case "max_by":
		return extremeBy(args, true)
	case "sum":
		return jsNumber(sum(args[0])), nil
	case "starts_with":
		// lastIndexOf(prefix) === 0, searching from the end: starts_with('abab', 'ab') is false.
		return jsBool(args[0].Call("lastIndexOf", args[1]).Int() == 0), nil
	case "min":
		return extreme(elements(args[0]), false), nil
	case "min_by":
		return extremeBy(args, false)
	case "type":
		if t, ok := typeName(args[0]); ok {
			return jsString(typeLabels[t]), nil
		}
		return js.Undefined, nil
	case "keys":
		return objectKeys(args[0]), nil
	case "values":
		return newArray(objValues(args[0])...), nil
	case "sort":
		// The engine's default sort: elements compared as strings, numbers too.
		return args[0].Call("slice").Call("sort"), nil
	case "sort_by":
		return sortBy(args[0], args[1])
	case "join":
		return args[1].Call("join", args[0]), nil
	case "reverse":
		// Code unit by code unit, as jmespath does: surrogate pairs come out reversed.
		if isString(args[0]) {
			return args[0].Call("split", "").Call("reverse").Call("join", ""), nil
		}
		return args[0].Call("slice").Call("reverse"), nil
	case "to_array":
		if hasType(args[0], typeArray) {
			return args[0], nil
		}
		return newArray(args[0]), nil
	case "to_string":
		if hasType(args[0], typeString) {
			return args[0], nil
		}
		return stringify(args[0]), nil
	case "to_number":
		switch {
		case hasType(args[0], typeNumber):
			return args[0], nil
		case hasType(args[0], typeString):
			if convertedValue := number(args[0]); !math.IsNaN(convertedValue) {
				return jsNumber(convertedValue), nil
			}
		}
		return nil, nil
	case "not_null":
		// Undefined is not null here: not_null(max_by(`[]`, &a)) is undefined.
		for _, arg := range args {
			if !hasType(arg, typeNull) {
				return arg, nil
			}
		}
		return nil, nil
	}
	return nil, unknownFunction(name)
}

func sum(array *js.Object) float64 {
	total := 0.0
	for _, x := range elements(array) {
		total += x.Float()
	}
	return total
}

// merge is `for (key in current)` over every argument: only the first one is type-checked, so
// later ones may be strings or arrays (their index keys) or primitives and null (no keys).
func merge(args []*js.Object) *js.Object {
	merged := newObject()
	for _, current := range args {
		switch {
		case isString(current):
			for i := 0; i < current.Length(); i++ {
				setProperty(merged, stringOf(jsNumber(float64(i))), current.Call("charAt", i))
			}
		case isArray(current) || isObject(current):
			for _, key := range elements(objectKeys(current)) {
				setProperty(merged, key, property(current, key))
			}
		}
	}
	return merged
}

// extreme is max or min: Math.max.apply or Math.min.apply over numbers (NaN checked explicitly
// because Go's math.Max(+Inf, NaN) returns +Inf and math.Min(-Inf, NaN) returns -Inf; math.Max and
// math.Min agree with JS on signed zeros), and localeCompare over strings.
func extreme(items []*js.Object, max bool) *js.Object {
	if len(items) == 0 {
		return nil
	}
	if hasType(items[0], typeNumber) {
		result := math.Inf(1)
		if max {
			result = math.Inf(-1)
		}
		for _, x := range items {
			v := x.Float()
			if math.IsNaN(v) {
				return jsNumber(math.NaN())
			}
			if max {
				result = math.Max(result, v)
			} else {
				result = math.Min(result, v)
			}
		}
		return jsNumber(result)
	}
	best := items[0]
	for _, x := range items[1:] {
		if max && best.Call("localeCompare", x).Int() < 0 || !max && x.Call("localeCompare", best).Int() < 0 {
			best = x
		}
	}
	return best
}

// keyFunction is jmespath's createKeyFunction for max_by and min_by: the key must be a number or a
// string.
func keyFunction(exprefNode, x *js.Object) (*js.Object, error) {
	current, err := visit(exprefNode, x)
	if err != nil {
		return nil, err
	}
	if !hasType(current, typeNumber) && !hasType(current, typeString) {
		return nil, typeError("expected one of number,string")
	}
	return current, nil
}

// extremeBy is max_by or min_by: the first element whose key beats the running one, starting from
// ∓∞, and undefined when none does.
func extremeBy(args []*js.Object, max bool) (*js.Object, error) {
	best, record := jsNumber(math.Inf(1)), js.Undefined
	if max {
		best = jsNumber(math.Inf(-1))
	}
	for _, element := range elements(args[0]) {
		current, err := keyFunction(args[1], element)
		if err != nil {
			return nil, err
		}
		if max && greater(current, best) || !max && less(current, best) {
			best, record = current, element
		}
	}
	return record, nil
}

// sortBy decorates with the index, sorts with the engine's Array.prototype.sort and checks each
// key's type inside the comparator, as jmespath does. jmespath's first error ends its sort; here
// the comparator keeps that error and answers 0 to the calls the engine makes after it, and the
// sorted result is discarded. With NaN keys the comparator is inconsistent, and the engine's
// algorithm decides the order.
func sortBy(array, exprefNode *js.Object) (*js.Object, error) {
	sortedArray := elements(array)
	if len(sortedArray) == 0 {
		return newArray(), nil
	}
	first, err := visit(exprefNode, sortedArray[0])
	if err != nil {
		return nil, err
	}
	requiredType, ok := typeName(first)
	if !ok || requiredType != typeNumber && requiredType != typeString {
		return nil, typeError("sort_by")
	}
	decorated := newArray()
	for i := range sortedArray {
		decorated.Call("push", i)
	}
	var sortErr error
	decorated.Call("sort", func(a, b int) int {
		if sortErr != nil {
			return 0
		}
		exprA, err := visit(exprefNode, sortedArray[a])
		if err == nil {
			var exprB *js.Object
			if exprB, err = visit(exprefNode, sortedArray[b]); err == nil {
				switch {
				case !hasType(exprA, requiredType) || !hasType(exprB, requiredType):
					err = typeError("expected the first key's type")
				case greater(exprA, exprB):
					return 1
				case less(exprA, exprB):
					return -1
				default:
					return a - b
				}
			}
		}
		sortErr = err
		return 0
	})
	if sortErr != nil {
		return nil, sortErr
	}
	sorted := newArray()
	for _, index := range elements(decorated) {
		sorted.Call("push", sortedArray[index.Int()])
	}
	return sorted, nil
}

// callFunction is jmespath's Runtime.callFunction.
func callFunction(name *js.Object, resolvedArgs []*js.Object) (*js.Object, error) {
	signature, ok := signatures[name.String()]
	if !isString(name) || !ok {
		return nil, unknownFunction(name.String())
	}
	if err := validateArgs(resolvedArgs, signature); err != nil {
		return nil, err
	}
	return call(name.String(), resolvedArgs)
}

func validateArgs(args []*js.Object, signature []parameter) error {
	if signature[len(signature)-1].variadic {
		if len(args) < len(signature) {
			return argumentError("too few arguments")
		}
	} else if len(args) != len(signature) {
		return argumentError("wrong number of arguments")
	}
	for i, parameter := range signature {
		actualType, ok := typeName(args[i])
		matched := false
		for _, expected := range parameter.types {
			if typeMatches(actualType, ok, expected, args[i]) {
				matched = true
				break
			}
		}
		if !matched {
			return typeError("unexpected argument type")
		}
	}
	return nil
}

func typeMatches(actual jmesType, ok bool, expected jmesType, argValue *js.Object) bool {
	switch expected {
	case typeAny:
		return true
	case typeArray:
		return ok && actual == typeArray
	case typeArrayNumber, typeArrayString:
		if !ok || actual != typeArray {
			return false
		}
		subtype := typeString
		if expected == typeArrayNumber {
			subtype = typeNumber
		}
		for _, element := range elements(argValue) {
			elementType, elementOK := typeName(element)
			if !typeMatches(elementType, elementOK, subtype, element) {
				return false
			}
		}
		return true
	}
	return ok && actual == expected
}
