// The MiniSearch type: documents, the inverted index, search, autoSuggest and vacuuming, following
// minisearch/dist/es/index.js (7.2.0) method by method.

package main

import (
	"math"
	"slices"
)

// Kind is the JavaScript type of a Value.
type Kind uint8

const (
	Undefined Kind = iota // an absent key
	Null
	Number
	String
)

// Value is a value MiniSearch reads from a document (extractField), or an id: undefined, null, a
// number or a string.
type Value struct {
	Kind   Kind
	Number float64
	Text   []uint16
}

// nullish is value == null.
func (v Value) nullish() bool {
	return v.Kind == Undefined || v.Kind == Null
}

// sameValueZero is how a Map compares keys and Array.prototype.includes compares values.
func sameValueZero(a, b *Value) bool {
	if a.Kind != b.Kind {
		return false
	}
	switch a.Kind {
	case Number:
		return a.Number == b.Number || (a.Number != a.Number && b.Number != b.Number)
	case String:
		return equal(a.Text, b.Text)
	}
	return true
}

// strictEquals is a === b.
func strictEquals(a, b *Value) bool {
	return sameValueZero(a, b) && !(a.Kind == Number && a.Number != a.Number)
}

// text is the value as a template literal writes it (`${value}`).
func (v Value) text() []uint16 {
	switch v.Kind {
	case Number:
		return numberText(v.Number)
	case String:
		return v.Text
	case Null:
		return encode("null")
	}
	return encode("undefined")
}

// idMap is _idToShortId, a Map from ids to short ids: numbers and strings are different keys (1
// is not "1"), so they are kept apart. (One map keyed by a struct would do, but GopherJS hashes a
// struct key by building a string from its fields on every lookup.) Its order is never observed.
type idMap struct {
	numbers map[float64]int
	strings map[string]int
}

func newIDMap() idMap {
	return idMap{numbers: map[float64]int{}, strings: map[string]int{}}
}

// get is the short id of id. An id is a number or a string (a document's id is never nullish
// here, and discard, has and getStoredFields of null or undefined find nothing).
func (m idMap) get(id Value) (int, bool) {
	var shortID int
	found := false
	switch id.Kind {
	case Number:
		shortID, found = m.numbers[id.Number]
	case String:
		shortID, found = m.strings[key(id.Text)]
	}
	return shortID, found
}

func (m idMap) set(id Value, shortID int) {
	if id.Kind == Number {
		m.numbers[id.Number] = shortID
	} else {
		m.strings[key(id.Text)] = shortID
	}
}

func (m idMap) delete(id Value) {
	if id.Kind == Number {
		delete(m.numbers, id.Number)
	} else {
		delete(m.strings, key(id.Text))
	}
}

// Document is a document as MiniSearch reads it: the values of its keys by name.
type Document map[string]Value

// get is extractField(document, name): undefined when the key is absent.
func (d Document) get(name string) Value {
	return d[name]
}

// StoredField is a stored field of a document: its name and value.
type StoredField struct {
	Name  string
	Value Value
}

// posting is one entry of a posting list: a document's short id and the term's frequency in it.
type posting struct {
	doc  int
	freq int
}

// fieldPostings is a term's posting list in one field (fieldIndex, a Map from short id to
// frequency). Its keys are always inserted in increasing order (a document only ever adds
// postings under its own short id, the largest so far), so the list is sorted by short id, which
// is that Map's insertion order.
//
// The postings are pointers: under GopherJS a slice of struct values is an array of objects that
// every growth fills with zero structs and every copy clones, and a pointer to an element is slow
// (NOTES.md).
type fieldPostings struct {
	field    int
	postings []*posting
}

// find is the position of doc's posting (a binary search over the sorted list), or -1.
func (p *fieldPostings) find(doc int) int {
	low, high := 0, len(p.postings)
	for low < high {
		middle := int(uint(low+high) >> 1)
		if p.postings[middle].doc < doc {
			low = middle + 1
		} else {
			high = middle
		}
	}
	if low < len(p.postings) && p.postings[low].doc == doc {
		return low
	}
	return -1
}

// termData is a term's value in the index (fieldsData): its posting list per field id.
type termData struct {
	fields []*fieldPostings
}

func (d *termData) postings(field int) *fieldPostings {
	for _, p := range d.fields {
		if p.field == field {
			return p
		}
	}
	return nil
}

// deleteField is fieldsData.delete(fieldId).
func (d *termData) deleteField(field int) {
	for i, p := range d.fields {
		if p.field == field {
			d.fields = slices.Delete(d.fields, i, i+1)
			return
		}
	}
}

// document is what MiniSearch keeps under a short id in _documentIds, _fieldLength and
// _storedFields, which it always deletes together.
type document struct {
	id Value
	// The unique token count per field id (_fieldLength): an array with holes, a hole being NaN
	// (a hole reads as undefined, which arithmetic turns into NaN and JSON into null). nil when
	// the document set no field length.
	fieldLength []float64
	// The stored fields, in storeFields order; nil (no entry) when storeFields is empty.
	stored []*StoredField
}

// Error is an error MiniSearch throws. Its message is UTF-16: it may quote a term or an id.
type Error struct {
	Message []uint16
}

func (e *Error) Error() string {
	return decode(e.Message)
}

func newError(parts ...[]uint16) *Error {
	var message []uint16
	for _, part := range parts {
		message = append(message, part...)
	}
	return &Error{Message: message}
}

// MiniSearch is the search engine: one index and its documents.
type MiniSearch struct {
	options        Options
	fieldIDs       map[string]int // _fieldIds
	index          *SearchableMap
	documentCount  int
	documents      []*document // by short id; nil once removed or discarded
	idToShortID    idMap
	avgFieldLength []float64 // _avgFieldLength: holes are NaN, as in document.fieldLength
	nextID         int
	dirtCount      int
	// _currentVacuum != null. Once a vacuum starts, MiniSearch clears this only in a later
	// microtask, so for the rest of a synchronous run it stays set (CONTRACT.md, "Vacuuming").
	vacuuming bool
	// What the logger option received since the last takeWarnings.
	warnings [][]uint16
}

// New is new MiniSearch(options).
func New(options Options) *MiniSearch {
	s := &MiniSearch{
		options:     options,
		fieldIDs:    make(map[string]int, len(options.Fields)),
		index:       newSearchableMap(),
		idToShortID: newIDMap(),
	}
	for i, field := range options.Fields {
		s.fieldIDs[field] = i
	}
	return s
}

// takeWarnings is the warnings logged since the last call.
func (s *MiniSearch) takeWarnings() [][]uint16 {
	warnings := s.warnings
	s.warnings = nil
	return warnings
}

// ---------------------------------------------------------------------------------------------
// Adding and removing documents.

func (s *MiniSearch) documentID(doc Document) (Value, error) {
	id := doc.get(s.options.IDField)
	if id.nullish() {
		return id, newError(encode(`MiniSearch: document does not have ID field "`), encode(s.options.IDField), encode(`"`))
	}
	return id, nil
}

// Add indexes a document.
func (s *MiniSearch) Add(doc Document) error {
	id, err := s.documentID(doc)
	if err != nil {
		return err
	}
	if _, found := s.idToShortID.get(id); found {
		return newError(encode("MiniSearch: duplicate ID "), id.text())
	}
	shortID := s.addDocumentID(id)
	s.saveStoredFields(shortID, doc)
	for _, field := range s.options.Fields {
		value := doc.get(field)
		if value.nullish() {
			continue
		}
		tokens := tokenize(value.text())
		fieldID := s.fieldIDs[field]
		s.addFieldLength(shortID, fieldID, s.documentCount-1, distinctCount(tokens))
		for _, token := range tokens {
			if term := toLowerCase(token); len(term) > 0 {
				s.addTerm(fieldID, shortID, term)
			}
		}
	}
	return nil
}

// AddAll adds each document in order; the first error ends it.
func (s *MiniSearch) AddAll(docs []Document) error {
	for _, doc := range docs {
		if err := s.Add(doc); err != nil {
			return err
		}
	}
	return nil
}

// Remove removes a document from the index at once. Its terms are those of the document as
// given, which must not have changed since it was added (nothing checks).
func (s *MiniSearch) Remove(doc Document) error {
	id, err := s.documentID(doc)
	if err != nil {
		return err
	}
	shortID, found := s.idToShortID.get(id)
	if !found {
		return newError(encode("MiniSearch: cannot remove document with ID "), id.text(), encode(": it is not in the index"))
	}
	for _, field := range s.options.Fields {
		value := doc.get(field)
		if value.nullish() {
			continue
		}
		tokens := tokenize(value.text())
		fieldID := s.fieldIDs[field]
		s.removeFieldLength(fieldID, s.documentCount, distinctCount(tokens))
		for _, token := range tokens {
			if term := toLowerCase(token); len(term) > 0 {
				s.removeTerm(fieldID, shortID, term)
			}
		}
	}
	s.documents[shortID] = nil
	s.idToShortID.delete(id)
	s.documentCount--
	return nil
}

// RemoveAll removes the given documents. With none (removeAll()), it resets the index, keeping
// dirtCount and the vacuum state.
func (s *MiniSearch) RemoveAll(docs []Document) error {
	for _, doc := range docs {
		if err := s.Remove(doc); err != nil {
			return err
		}
	}
	return nil
}

// Clear is removeAll() with no argument.
func (s *MiniSearch) Clear() {
	s.index = newSearchableMap()
	s.documentCount = 0
	s.documents = nil
	s.idToShortID = newIDMap()
	s.avgFieldLength = nil
	s.nextID = 0
}

// Discard removes a document from searches, leaving its postings in the index until a search
// meets them or a vacuum removes them.
func (s *MiniSearch) Discard(id Value) error {
	if err := s.discard(id); err != nil {
		return err
	}
	s.maybeAutoVacuum()
	return nil
}

func (s *MiniSearch) discard(id Value) error {
	shortID, found := s.idToShortID.get(id)
	if !found {
		return newError(encode("MiniSearch: cannot discard document with ID "), id.text(), encode(": it is not in the index"))
	}
	s.idToShortID.delete(id)
	doc := s.documents[shortID]
	s.documents[shortID] = nil
	// forEach over the array skips its holes: the fields the document did not have.
	for fieldID, length := range doc.fieldLength {
		if length == length {
			s.removeFieldLength(fieldID, s.documentCount, int(length))
		}
	}
	s.documentCount--
	s.dirtCount++
	return nil
}

// DiscardAll discards each id with automatic vacuuming off, then considers one automatic vacuum.
// A failing discard ends it there, without that vacuum.
func (s *MiniSearch) DiscardAll(ids []Value) error {
	for _, id := range ids {
		if err := s.discard(id); err != nil {
			return err
		}
	}
	s.maybeAutoVacuum()
	return nil
}

// Replace is Discard of the document's id (which may vacuum), then Add.
func (s *MiniSearch) Replace(doc Document) error {
	if err := s.Discard(doc.get(s.options.IDField)); err != nil {
		return err
	}
	return s.Add(doc)
}

// Has reports whether a document with this id is in the index.
func (s *MiniSearch) Has(id Value) bool {
	_, found := s.idToShortID.get(id)
	return found
}

// StoredFields is getStoredFields(id); false when MiniSearch returns undefined.
func (s *MiniSearch) StoredFields(id Value) ([]*StoredField, bool) {
	shortID, found := s.idToShortID.get(id)
	if !found || len(s.options.StoreFields) == 0 {
		return nil, false
	}
	return s.documents[shortID].stored, true
}

func (s *MiniSearch) addDocumentID(id Value) int {
	shortID := s.nextID
	s.idToShortID.set(id, shortID)
	s.documents = append(s.documents, &document{id: id})
	s.documentCount++
	s.nextID++
	return shortID
}

func (s *MiniSearch) saveStoredFields(shortID int, doc Document) {
	if len(s.options.StoreFields) == 0 {
		return
	}
	stored := []*StoredField{}
fields:
	for _, name := range s.options.StoreFields {
		value := doc.get(name)
		if value.Kind == Undefined {
			continue
		}
		// documentFields[fieldName] = fieldValue: a repeated name keeps its first place.
		for _, field := range stored {
			if field.Name == name {
				field.Value = value
				continue fields
			}
		}
		stored = append(stored, &StoredField{Name: name, Value: value})
	}
	s.documents[shortID].stored = stored
}

// at is array[index] for the arrays with holes: NaN (undefined) past the end or at a hole.
func at(array []float64, index int) float64 {
	if index < len(array) {
		return array[index]
	}
	return math.NaN()
}

// setAt is array[index] = value, growing the array with holes.
func setAt(array []float64, index int, value float64) []float64 {
	for len(array) <= index {
		array = append(array, math.NaN())
	}
	array[index] = value
	return array
}

func (s *MiniSearch) addFieldLength(shortID, fieldID, count, length int) {
	doc := s.documents[shortID]
	doc.fieldLength = setAt(doc.fieldLength, fieldID, float64(length))
	average := or(at(s.avgFieldLength, fieldID), 0)
	totalFieldLength := (average * float64(count)) + float64(length)
	s.avgFieldLength = setAt(s.avgFieldLength, fieldID, totalFieldLength/float64(count+1))
}

func (s *MiniSearch) removeFieldLength(fieldID, count, length int) {
	if count == 1 {
		s.avgFieldLength = setAt(s.avgFieldLength, fieldID, 0)
		return
	}
	totalFieldLength := (at(s.avgFieldLength, fieldID) * float64(count)) - float64(length)
	s.avgFieldLength = setAt(s.avgFieldLength, fieldID, totalFieldLength/float64(count-1))
}

func (s *MiniSearch) addTerm(fieldID, shortID int, term []uint16) {
	data := s.index.fetch(term)
	list := data.postings(fieldID)
	if list == nil {
		data.fields = append(data.fields, &fieldPostings{field: fieldID, postings: []*posting{{doc: shortID, freq: 1}}})
		return
	}
	// The document being added has the largest short id: its posting is last, or absent.
	if last := list.postings[len(list.postings)-1]; last.doc == shortID {
		last.freq++
	} else {
		list.postings = append(list.postings, &posting{doc: shortID, freq: 1})
	}
}

func (s *MiniSearch) removeTerm(fieldID, shortID int, term []uint16) {
	if !s.index.has(term) {
		s.warnDocumentChanged(shortID, fieldID, term)
		return
	}
	data := s.index.fetch(term)
	list := data.postings(fieldID)
	i := -1
	if list != nil {
		i = list.find(shortID)
	}
	switch {
	case i < 0:
		s.warnDocumentChanged(shortID, fieldID, term)
	case list.postings[i].freq <= 1:
		if len(list.postings) <= 1 {
			data.deleteField(fieldID)
		} else {
			list.postings = slices.Delete(list.postings, i, i+1)
		}
	default:
		list.postings[i].freq--
	}
	if len(s.index.get(term).fields) == 0 {
		s.index.delete(term)
	}
}

func (s *MiniSearch) warnDocumentChanged(shortID, fieldID int, term []uint16) {
	// The name whose _fieldIds entry is fieldId, in the order of Object.keys(this._fieldIds).
	for _, name := range s.options.Fields {
		if s.fieldIDs[name] != fieldID {
			continue
		}
		var docID Value // undefined when the short id has no document
		if doc := s.documents[shortID]; doc != nil {
			docID = doc.id
		}
		// The logger option (level warn, code version_conflict) records the message.
		s.warnings = append(s.warnings, newError(
			encode("MiniSearch: document with ID "), docID.text(),
			encode(` has changed before removal: term "`), term,
			encode(`" was not present in field "`), encode(name),
			encode(`". Removing a document after it has changed can corrupt the index!`),
		).Message)
		return
	}
}

// ---------------------------------------------------------------------------------------------
// Vacuuming.

func (s *MiniSearch) maybeAutoVacuum() {
	if conditions := s.options.AutoVacuum; conditions != nil {
		s.conditionalVacuum(conditions.BatchSize, conditions)
	}
}

// Vacuum is vacuum({ batchSize }): a vacuum without conditions (batchSize 0 when not given).
func (s *MiniSearch) Vacuum(batchSize float64) {
	s.conditionalVacuum(batchSize, nil)
}

func (s *MiniSearch) conditionalVacuum(batchSize float64, conditions *AutoVacuum) {
	// A request made while a vacuum is current is chained after it, on a promise that settles
	// after the synchronous run: it never runs here.
	if s.vacuuming {
		return
	}
	if conditions != nil && !s.vacuumConditionsMet(conditions) {
		return
	}
	s.vacuuming = true
	s.performVacuuming(batchSize)
}

func (s *MiniSearch) vacuumConditionsMet(conditions *AutoVacuum) bool {
	minDirtCount := or(conditions.MinDirtCount, 20)
	minDirtFactor := or(conditions.MinDirtFactor, 0.1)
	return float64(s.dirtCount) >= minDirtCount && s.DirtFactor() >= minDirtFactor
}

// performVacuuming is the synchronous part of MiniSearch's async performVacuuming: the terms in
// iteration order up to the first batch boundary, where it awaits a timer and the rest happens
// after the run. Only a walk that ends before a boundary gives the dirt count back.
func (s *MiniSearch) performVacuuming(batchSize float64) {
	initialDirtCount := s.dirtCount
	batchSize = or(batchSize, 1000)
	for i, entry := range s.index.entries() {
		data := entry.data
		for _, list := range slices.Clone(data.fields) {
			// For each posting of a document no longer present: if it is the list's only one, the
			// field goes; else the posting goes. That leaves the present documents' postings, or no
			// field at all.
			var present []*posting
			for _, p := range list.postings {
				if s.documents[p.doc] != nil {
					present = append(present, p)
				}
			}
			if len(present) == 0 {
				data.deleteField(list.field)
			} else {
				list.postings = present
			}
		}
		if len(s.index.get(entry.term).fields) == 0 {
			s.index.delete(entry.term)
		}
		if math.Mod(float64(i+1), batchSize) == 0 {
			return
		}
	}
	s.dirtCount -= initialDirtCount
}

// ---------------------------------------------------------------------------------------------
// State.

func (s *MiniSearch) DocumentCount() int { return s.documentCount }

// TermCount is the number of terms in the index.
func (s *MiniSearch) TermCount() int { return s.index.size() }

func (s *MiniSearch) DirtCount() int { return s.dirtCount }

func (s *MiniSearch) DirtFactor() float64 {
	return float64(s.dirtCount) / (1 + float64(s.documentCount) + float64(s.dirtCount))
}

func (s *MiniSearch) IsVacuuming() bool { return s.vacuuming }

// Serialized is the value of JSON.parse(JSON.stringify(index)), as references into the index:
// short-id keyed maps are listed in ascending short id, which is both their insertion order and
// the order JSON gives integer keys.
type Serialized struct {
	DocumentCount      int
	NextID             int
	Documents          []SerializedDocument
	FieldIDs           []string // names by field id
	AverageFieldLength []float64
	StoreFields        bool // storedFields holds an entry per document
	DirtCount          int
	Index              []termEntry
}

// SerializedDocument is a present document's entries in documentIds, fieldLength and storedFields.
type SerializedDocument struct {
	ShortID     int
	ID          Value
	FieldLength []float64 // nil: no entry
	Stored      []*StoredField
}

// ToJSON is toJSON(), the index terms in iteration order.
func (s *MiniSearch) ToJSON() Serialized {
	var documents []SerializedDocument
	for shortID, doc := range s.documents {
		if doc != nil {
			documents = append(documents, SerializedDocument{shortID, doc.id, doc.fieldLength, doc.stored})
		}
	}
	return Serialized{
		DocumentCount:      s.documentCount,
		NextID:             s.nextID,
		Documents:          documents,
		FieldIDs:           s.options.Fields,
		AverageFieldLength: s.avgFieldLength,
		StoreFields:        len(s.options.StoreFields) > 0,
		DirtCount:          s.dirtCount,
		Index:              s.index.entries(),
	}
}
