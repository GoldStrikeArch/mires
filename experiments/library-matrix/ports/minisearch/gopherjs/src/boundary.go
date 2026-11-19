// The boundary: the corpus input (a JavaScript object) read into Go values, the operation script
// run on MiniSearch, and its results written back as JavaScript objects (CONTRACT.md, "Output").
// It does what libraries/minisearch/reference.mjs does around the package.
//
// Text that reaches the index (document values, queries, terms in options, string ids) crosses
// as UTF-16 code units (units, jsString in text.go). Names (fields, stored fields, the id field,
// operation names) are ASCII identifiers in the parity domain and cross as Go strings.

package main

import "github.com/gopherjs/gopherjs/js"

// typeOf is the typeof operator, as the GopherJS prelude exposes it ($typeOf).
func typeOf(v *js.Object) string {
	return js.Global.Call("$typeOf", v).String()
}

// get is object[key], and whether it is defined.
func get(object *js.Object, key string) (*js.Object, bool) {
	value := object.Get(key)
	return value, value != js.Undefined
}

// elements is the elements of a JavaScript array.
func elements(array *js.Object) []*js.Object {
	items := make([]*js.Object, array.Length())
	for i := range items {
		items[i] = array.Index(i)
	}
	return items
}

func names(array *js.Object) []string {
	items := elements(array)
	out := make([]string, len(items))
	for i, item := range items {
		out[i] = item.String()
	}
	return out
}

// value reads a document value or an id.
func value(v *js.Object) Value {
	switch typeOf(v) {
	case "undefined":
		return Value{}
	case "string":
		return Value{Kind: String, Text: units(v)}
	case "number":
		return Value{Kind: Number, Number: v.Float()}
	case "object":
		if v == nil {
			return Value{Kind: Null}
		}
	}
	panic("minisearch (GopherJS port): document values and ids are strings, numbers or null")
}

// number is object[key] as a number, nil when it is undefined.
func number(object *js.Object, key string) *float64 {
	if v, ok := get(object, key); ok {
		n := v.Float()
		return &n
	}
	return nil
}

func searchOptions(object *js.Object) *SearchOptions {
	o := &SearchOptions{}
	if v, ok := get(object, "fields"); ok {
		fields := names(v)
		o.Fields = &fields
	}
	if v, ok := get(object, "boost"); ok {
		o.Boost = map[string]float64{}
		for _, field := range js.Keys(v) {
			o.Boost[field] = v.Get(field).Float()
		}
	}
	if v, ok := get(object, "prefix"); ok {
		o.Prefix = &Prefix{Value: v.Bool()}
	}
	if v, ok := get(object, "fuzzy"); ok {
		if typeOf(v) == "boolean" {
			o.Fuzzy = &Fuzzy{Flag: v.Bool()}
		} else {
			o.Fuzzy = &Fuzzy{IsNumber: true, Number: v.Float()}
		}
	}
	o.MaxFuzzy = number(object, "maxFuzzy")
	if v, ok := get(object, "combineWith"); ok {
		combineWith := units(v)
		o.CombineWith = &combineWith
	}
	if v, ok := get(object, "weights"); ok {
		o.Weights = &Weights{Fuzzy: number(v, "fuzzy"), Prefix: number(v, "prefix")}
	}
	if v, ok := get(object, "bm25"); ok {
		// A missing parameter would be undefined in MiniSearch's arithmetic, as NaN is here.
		o.BM25 = &BM25{K: v.Get("k").Float(), B: v.Get("b").Float(), D: v.Get("d").Float()}
	}
	if v, ok := get(object, "filter"); ok {
		o.Filter = &Filter{Field: v.Get("field").String()}
		for _, item := range elements(v.Get("in")) {
			value := value(item)
			o.Filter.Values = append(o.Filter.Values, &value)
		}
	}
	if v, ok := get(object, "boostDocument"); ok {
		pairs := []*DocumentBoost{}
		for _, pair := range elements(v) {
			pairs = append(pairs, &DocumentBoost{ID: value(pair.Index(0)), Factor: pair.Index(1).Float()})
		}
		o.BoostDocument = &pairs
	}
	if v, ok := get(object, "boostTerm"); ok {
		pairs := []*TermBoost{}
		for _, pair := range elements(v) {
			pairs = append(pairs, &TermBoost{Term: units(pair.Index(0)), Factor: pair.Index(1).Float()})
		}
		o.BoostTerm = &pairs
	}
	return o
}

func optionalSearchOptions(object *js.Object, key string) *SearchOptions {
	if v, ok := get(object, key); ok {
		return searchOptions(v)
	}
	return &SearchOptions{}
}

func indexOptions(object *js.Object) Options {
	options := Options{
		Fields:             names(object.Get("fields")),
		IDField:            "id",
		SearchOptions:      defaultSearchOptions().over(optionalSearchOptions(object, "searchOptions")),
		AutoSuggestOptions: defaultAutoSuggestOptions().over(optionalSearchOptions(object, "autoSuggestOptions")),
		AutoVacuum:         &AutoVacuum{BatchSize: 1000, MinDirtCount: 20, MinDirtFactor: 0.1},
	}
	if v, ok := get(object, "idField"); ok {
		options.IDField = v.String()
	}
	if v, ok := get(object, "storeFields"); ok {
		options.StoreFields = names(v)
	}
	if v, ok := get(object, "autoVacuum"); ok {
		if typeOf(v) == "boolean" {
			if !v.Bool() {
				options.AutoVacuum = nil
			}
		} else {
			options.AutoVacuum = &AutoVacuum{
				BatchSize:     numberOrZero(v, "batchSize"),
				MinDirtCount:  numberOrZero(v, "minDirtCount"),
				MinDirtFactor: numberOrZero(v, "minDirtFactor"),
			}
		}
	}
	return options
}

// numberOrZero is object[key] as a number, 0 when it is undefined (for values MiniSearch reads
// with `|| default`).
func numberOrZero(object *js.Object, key string) float64 {
	if n := number(object, key); n != nil {
		return *n
	}
	return 0
}

func query(v *js.Object) *Query {
	if typeOf(v) == "string" {
		return &Query{Kind: TextQuery, Text: units(v)}
	}
	if wildcard, ok := get(v, "wildcard"); ok && wildcard.Bool() {
		return &Query{Kind: WildcardQuery}
	}
	q := &Query{Kind: TreeQuery, Options: searchOptions(v)}
	for _, subquery := range elements(v.Get("queries")) {
		q.Queries = append(q.Queries, query(subquery))
	}
	return q
}

// operation is one step of the script, read from the input before the script runs.
type operation struct {
	op        string
	document  Document   // add, remove, replace
	documents []Document // addAll, removeAll(documents)
	// removeAll: the documents key is absent (removeAll()), or null (removeAll(null)).
	noDocuments, nullDocuments bool
	id                         Value    // discard, has, getStoredFields
	ids                        []Value  // discardAll
	batchSize                  float64  // vacuum: 0 when not given
	query                      *Query   // search
	text                       []uint16 // autoSuggest
	options                    *SearchOptions
}

// reader reads the operations. keys are the names extractField is asked for: the id, the indexed
// and the stored fields.
type reader struct {
	keys []string
}

func (r reader) document(v *js.Object) Document {
	doc := make(Document, len(r.keys))
	for _, name := range r.keys {
		if field := value(v.Get(name)); field.Kind != Undefined {
			doc[name] = field
		}
	}
	return doc
}

func (r reader) documents(v *js.Object) []Document {
	items := elements(v)
	docs := make([]Document, len(items))
	for i, item := range items {
		docs[i] = r.document(item)
	}
	return docs
}

func (r reader) operation(v *js.Object) *operation {
	o := &operation{op: v.Get("op").String()}
	switch o.op {
	case "add", "remove", "replace":
		o.document = r.document(v.Get("document"))
	case "addAll":
		o.documents = r.documents(v.Get("documents"))
	case "removeAll":
		switch documents, ok := get(v, "documents"); {
		case !ok:
			o.noDocuments = true
		case documents == nil:
			o.nullDocuments = true
		default:
			o.documents = r.documents(documents)
		}
	case "discard", "has", "getStoredFields":
		o.id = value(v.Get("id"))
	case "discardAll":
		for _, id := range elements(v.Get("ids")) {
			o.ids = append(o.ids, value(id))
		}
	case "vacuum":
		if options, ok := get(v, "options"); ok {
			o.batchSize = numberOrZero(options, "batchSize")
		}
	case "search":
		o.query, o.options = query(v.Get("query")), optionalSearchOptions(v, "options")
	case "autoSuggest":
		o.text, o.options = units(v.Get("query")), optionalSearchOptions(v, "options")
	case "stats", "toJSON":
	default:
		panic("minisearch (GopherJS port): unknown operation " + o.op)
	}
	return o
}

// execute runs the operation on the index and writes its payload into the record when it returns.
func execute(index *MiniSearch, o *operation, record *js.Object) error {
	switch o.op {
	case "add":
		return index.Add(o.document)
	case "addAll":
		return index.AddAll(o.documents)
	case "remove":
		return index.Remove(o.document)
	case "removeAll":
		switch {
		case o.noDocuments:
			index.Clear()
			return nil
		case o.nullDocuments:
			return newError(encode("Expected documents to be present. Omit the argument to remove all documents."))
		}
		return index.RemoveAll(o.documents)
	case "discard":
		return index.Discard(o.id)
	case "discardAll":
		return index.DiscardAll(o.ids)
	case "replace":
		return index.Replace(o.document)
	case "vacuum":
		index.Vacuum(o.batchSize)
	case "search":
		hits, err := index.Search(o.query, o.options)
		if err != nil {
			return err
		}
		record.Set("hits", hitsArray(hits))
	case "autoSuggest":
		suggestions, err := index.AutoSuggest(o.text, o.options)
		if err != nil {
			return err
		}
		record.Set("suggestions", suggestionsArray(suggestions))
	case "has":
		record.Set("value", index.Has(o.id))
	case "getStoredFields":
		if stored, ok := index.StoredFields(o.id); ok {
			record.Set("value", storedObject(stored))
		} else {
			record.Set("value", nil)
		}
	case "stats":
		record.Set("documentCount", index.DocumentCount())
		record.Set("termCount", index.TermCount())
		record.Set("dirtCount", index.DirtCount())
		record.Set("dirtFactor", index.DirtFactor())
		record.Set("isVacuuming", index.IsVacuuming())
	case "toJSON":
		record.Set("value", serialized(index.ToJSON()))
	}
	return nil
}

// run is the module's export: one corpus input, { options, operations }, to { results }.
func run(input *js.Object) *js.Object {
	options := indexOptions(input.Get("options"))
	var r reader
	for _, name := range append(append([]string{options.IDField}, options.Fields...), options.StoreFields...) {
		if !contains(r.keys, name) {
			r.keys = append(r.keys, name)
		}
	}
	items := elements(input.Get("operations"))
	operations := make([]*operation, len(items))
	for i, item := range items {
		operations[i] = r.operation(item)
	}

	index := New(options)
	results := newArray()
	for i, o := range operations {
		record := newObject()
		record.Set("op", o.op)
		err := execute(index, o, record)
		if warnings := index.takeWarnings(); len(warnings) > 0 {
			record.Set("warnings", stringArray(warnings))
		}
		if err != nil {
			record.Set("error", jsString(err.(*Error).Message))
		}
		results.SetIndex(i, record)
	}
	output := newObject()
	output.Set("results", results)
	return output
}

func contains(list []string, name string) bool {
	for _, item := range list {
		if item == name {
			return true
		}
	}
	return false
}

// ---------------------------------------------------------------------------------------------
// Writing.

func newObject() *js.Object { return js.Global.Get("Object").New() }

func newArray() *js.Object { return js.Global.Get("Array").New() }

func stringArray(texts [][]uint16) *js.Object {
	array := newArray()
	for i, text := range texts {
		array.SetIndex(i, jsString(text))
	}
	return array
}

// scalar is a Value as a JavaScript value.
func scalar(v *Value) any {
	switch v.Kind {
	case Number:
		return v.Number
	case String:
		return jsString(v.Text)
	case Null:
		return nil
	}
	return js.Undefined
}

// jsonNumber is a number as JSON.parse(JSON.stringify(n)) gives it back: null when it is not
// finite, and 0 for -0.
func jsonNumber(n float64) any {
	if n != n || n-n != 0 {
		return nil
	}
	if n == 0 {
		return 0
	}
	return n
}

func jsonScalar(v *Value) any {
	if v.Kind == Number {
		return jsonNumber(v.Number)
	}
	return scalar(v)
}

func storedObject(stored []*StoredField) *js.Object {
	object := newObject()
	for _, field := range stored {
		object.Set(field.Name, scalar(&field.Value))
	}
	return object
}

// hitsArray writes the hits as the adapter copies them: { id, score, terms, queryTerms, match }
// with match as [term, fields] pairs, then the stored fields (Object.assign: a stored id replaces
// the id in place).
func hitsArray(hits []*Hit) *js.Object {
	array := newArray()
	for i, hit := range hits {
		object := newObject()
		object.Set("id", scalar(hit.ID))
		object.Set("score", hit.Score)
		terms, match := newArray(), newArray()
		for j, m := range hit.Match {
			term := jsString(m.term)
			terms.SetIndex(j, term)
			fields := newArray()
			for k, field := range m.fields {
				fields.SetIndex(k, field)
			}
			pair := newArray()
			pair.SetIndex(0, term)
			pair.SetIndex(1, fields)
			match.SetIndex(j, pair)
		}
		object.Set("terms", terms)
		object.Set("queryTerms", stringArray(hit.QueryTerms))
		object.Set("match", match)
		for _, field := range hit.Stored {
			object.Set(field.Name, scalar(&field.Value))
		}
		array.SetIndex(i, object)
	}
	return array
}

func suggestionsArray(suggestions []*Suggestion) *js.Object {
	array := newArray()
	for i, s := range suggestions {
		object := newObject()
		object.Set("suggestion", jsString(s.Suggestion))
		object.Set("terms", stringArray(s.Terms))
		object.Set("score", s.Score)
		array.SetIndex(i, object)
	}
	return array
}

// numbers is an array of numbers with holes as JSON writes it (NaN, holes and infinities null).
func numbers(values []float64) *js.Object {
	array := newArray()
	for i, n := range values {
		array.SetIndex(i, jsonNumber(n))
	}
	return array
}

// serialized writes toJSON's value. Integer keys (short ids, field ids) are set on plain objects,
// which list them in ascending order, as JSON.stringify does after Object.fromEntries.
func serialized(index Serialized) *js.Object {
	documentIDs, fieldLength, storedFields := newObject(), newObject(), newObject()
	for _, doc := range index.Documents {
		documentIDs.SetIndex(doc.ShortID, jsonScalar(&doc.ID))
		if doc.FieldLength != nil {
			fieldLength.SetIndex(doc.ShortID, numbers(doc.FieldLength))
		}
		if index.StoreFields {
			stored := newObject()
			for _, field := range doc.Stored {
				stored.Set(field.Name, jsonScalar(&field.Value))
			}
			storedFields.SetIndex(doc.ShortID, stored)
		}
	}
	fieldIDs := newObject()
	for id, name := range index.FieldIDs {
		fieldIDs.Set(name, id) // addFields: a repeated name keeps its first place and its last id
	}
	terms := newArray()
	for i, entry := range index.Index {
		data := newObject()
		for _, list := range entry.data.fields {
			frequencies := newObject()
			for _, p := range list.postings {
				frequencies.SetIndex(p.doc, p.freq)
			}
			data.SetIndex(list.field, frequencies)
		}
		pair := newArray()
		pair.SetIndex(0, jsString(entry.term))
		pair.SetIndex(1, data)
		terms.SetIndex(i, pair)
	}
	object := newObject()
	object.Set("documentCount", index.DocumentCount)
	object.Set("nextId", index.NextID)
	object.Set("documentIds", documentIDs)
	object.Set("fieldIds", fieldIDs)
	object.Set("fieldLength", fieldLength)
	object.Set("averageFieldLength", numbers(index.AverageFieldLength))
	object.Set("storedFields", storedFields)
	object.Set("dirtCount", index.DirtCount)
	object.Set("index", terms)
	object.Set("serializationVersion", 2)
	return object
}
