// Search and autoSuggest: query trees, executeQuerySpec, termResults with BM25+, the combinators
// and the assembly of results.

package main

import (
	"math"
	"slices"
)

// QueryKind is what a query is.
type QueryKind uint8

const (
	TextQuery     QueryKind = iota // a string
	WildcardQuery                  // MiniSearch.wildcard
	TreeQuery                      // { queries, ...options }
)

// Query is a search query.
type Query struct {
	Kind    QueryKind
	Text    []uint16
	Options *SearchOptions // a tree node's own options
	Queries []*Query       // a tree node's subqueries
}

// matchEntry is one key of a result's match object: a document term and the fields it matched.
type matchEntry struct {
	term   []uint16
	fields []string
}

// result is MiniSearch's raw result for one document, { score, terms, match }.
type result struct {
	doc   int
	score float64
	terms [][]uint16 // the query terms that matched
	// match, keys in insertion order. Object.keys lists array-index keys first (hitMatch).
	match []*matchEntry
}

func (r *result) matchFor(term []uint16) *matchEntry {
	for _, m := range r.match {
		if equal(m.term, term) {
			return m
		}
	}
	return nil
}

// results is Map<shortId, result>: insertion-ordered, which is the order of the hits before
// sorting and of equal scores after it.
type results struct {
	order []*result
	byDoc map[int]*result
}

func newResults() *results {
	return &results{byDoc: map[int]*result{}}
}

func (rs *results) add(r *result) {
	rs.order = append(rs.order, r)
	rs.byDoc[r.doc] = r
}

// Hit is a search result.
type Hit struct {
	ID         *Value
	Score      float64
	Match      []*matchEntry // in Object.keys order: these terms are the hit's terms
	QueryTerms [][]uint16
	Stored     []*StoredField // what Object.assign(result, storedFields) adds
}

// property is result[name] on MiniSearch's result object, for the filter: a stored field (which
// Object.assign set over the result's own keys), else the id; undefined otherwise. (The parity
// domain names only stored fields and id here.)
func (h *Hit) property(name string) *Value {
	for _, field := range h.Stored {
		if field.Name == name {
			return &field.Value
		}
	}
	if name == "id" {
		return h.ID
	}
	return &Value{}
}

// matches is the declarative filter: values.includes(result[field]).
func (f *Filter) matches(h *Hit) bool {
	value := h.property(f.Field)
	for _, v := range f.Values {
		if sameValueZero(v, value) {
			return true
		}
	}
	return false
}

// Suggestion is an autoSuggest result.
type Suggestion struct {
	Suggestion []uint16
	Terms      [][]uint16
	Score      float64
}

// scoreOrder is the order results.sort(byScore) puts items with these scores in, as positions:
// MiniSearch's comparator is (a, b) => b.score - a.score, so an item goes before another exactly
// when b.score - a.score < 0, and the engines' sort is stable, so equal scores (both zeros
// included) keep their order. It is a merge sort, which is stable.
//
// Not slices.SortStableFunc: GopherJS compiles every function that calls a function value (here
// the comparison) into a resumable state machine, in case the callee blocks, and that spreads to
// all its callers (NOTES.md).
func scoreOrder(scores []float64) []int {
	order := make([]int, len(scores))
	for i := range order {
		order[i] = i
	}
	mergeSort(order, make([]int, len(order)), scores, 0, len(order))
	return order
}

// mergeSort sorts order[low:high] by descending score, stably, through buffer (as long as order).
// It passes bounds rather than subslices: under GopherJS every subslice is an allocation.
func mergeSort(order, buffer []int, scores []float64, low, high int) {
	if high-low < 2 {
		return
	}
	middle := int(uint(low+high) >> 1)
	mergeSort(order, buffer, scores, low, middle)
	mergeSort(order, buffer, scores, middle, high)
	for i := low; i < high; i++ {
		buffer[i] = order[i]
	}
	left, right := low, middle
	for out := low; out < high; out++ {
		// The right item goes first only when it must: when b.score - a.score < 0 for a = right,
		// b = left.
		if left == middle || (right < high && scores[buffer[left]]-scores[buffer[right]] < 0) {
			order[out] = buffer[right]
			right++
		} else {
			order[out] = buffer[left]
			left++
		}
	}
}

// Search is search(query, options).
func (s *MiniSearch) Search(query *Query, options *SearchOptions) ([]*Hit, error) {
	withDefaults := s.options.SearchOptions.over(options)
	raw, err := s.executeQuery(query, options)
	if err != nil {
		return nil, err
	}
	hits := make([]*Hit, 0, len(raw.order))
	for _, r := range raw.order {
		// The quality is the number of distinct query terms that matched.
		quality := max(len(r.terms), 1)
		doc := s.documents[r.doc]
		hit := &Hit{ID: &doc.id, Score: r.score * float64(quality), Match: hitMatch(r.match), QueryTerms: r.terms, Stored: doc.stored}
		if withDefaults.Filter == nil || withDefaults.Filter.matches(hit) {
			hits = append(hits, hit)
		}
	}
	// A top-level wildcard without boostDocument is not sorted: every score is 1.
	if query.Kind == WildcardQuery && withDefaults.BoostDocument == nil {
		return hits, nil
	}
	scores := make([]float64, len(hits))
	for i, hit := range hits {
		scores[i] = hit.Score
	}
	sorted := make([]*Hit, len(hits))
	for i, at := range scoreOrder(scores) {
		sorted[i] = hits[at]
	}
	return sorted, nil
}

// hitMatch is the match entries in Object.keys order: the keys that are array indices first, in
// ascending numeric order (an insertion sort: they are few, and distinct), then the others in
// insertion order.
func hitMatch(match []*matchEntry) []*matchEntry {
	some := false
	for _, m := range match {
		if _, ok := arrayIndex(m.term); ok {
			some = true
			break
		}
	}
	if !some {
		return match
	}
	var indices []float64
	var ordered, others []*matchEntry
	for _, m := range match {
		value, ok := arrayIndex(m.term)
		if !ok {
			others = append(others, m)
			continue
		}
		i := len(indices)
		for i > 0 && indices[i-1] > value {
			i--
		}
		indices = slices.Insert(indices, i, value)
		ordered = slices.Insert(ordered, i, m)
	}
	return append(ordered, others...)
}

// AutoSuggest is autoSuggest(query, options).
func (s *MiniSearch) AutoSuggest(query []uint16, options *SearchOptions) ([]*Suggestion, error) {
	options = s.options.AutoSuggestOptions.over(options)
	hits, err := s.Search(&Query{Kind: TextQuery, Text: query}, options)
	if err != nil {
		return nil, err
	}
	type group struct {
		suggestion *Suggestion
		score      float64
		count      int
	}
	var groups []*group
	byPhrase := map[string]*group{}
	for _, hit := range hits {
		terms := make([][]uint16, len(hit.Match))
		for i, m := range hit.Match {
			terms[i] = m.term
		}
		phrase := join(terms, ' ')
		if g := byPhrase[key(phrase)]; g != nil {
			g.score += hit.Score
			g.count++
		} else {
			g = &group{&Suggestion{Suggestion: phrase, Terms: terms}, hit.Score, 1}
			byPhrase[key(phrase)] = g
			groups = append(groups, g)
		}
	}
	scores := make([]float64, len(groups))
	for i, g := range groups {
		g.suggestion.Score = g.score / float64(g.count)
		scores[i] = g.suggestion.Score
	}
	suggestions := make([]*Suggestion, len(groups))
	for i, at := range scoreOrder(scores) {
		suggestions[i] = groups[at].suggestion
	}
	return suggestions, nil
}

func (s *MiniSearch) executeQuery(query *Query, options *SearchOptions) (*results, error) {
	switch query.Kind {
	case WildcardQuery:
		return s.executeWildcardQuery(options), nil
	case TreeQuery:
		// { ...searchOptions, ...query }: the index defaults are not merged in here, so a node
		// with no combineWith in it or above it combines with OR.
		nodeOptions := options.over(query.Options)
		var all []*results
		for _, subquery := range query.Queries {
			rs, err := s.executeQuery(subquery, nodeOptions)
			if err != nil {
				return nil, err
			}
			all = append(all, rs)
		}
		return combineResults(all, nodeOptions.CombineWith)
	}
	options = s.options.SearchOptions.over(options)
	var terms [][]uint16
	for _, token := range tokenize(query.Text) {
		if term := toLowerCase(token); len(term) > 0 {
			terms = append(terms, term)
		}
	}
	all := make([]*results, 0, len(terms))
	for i, term := range terms {
		all = append(all, s.executeQuerySpec(termToQuerySpec(options, term, i, len(terms)), options))
	}
	return combineResults(all, options.CombineWith)
}

// querySpec is one query term with its options (termToQuerySpec).
type querySpec struct {
	term      []uint16
	fuzzy     *Fuzzy
	prefix    bool
	termBoost float64
}

// termToQuerySpec reads the merged options, which always hold fuzzy and prefix (the library
// defaults are under them).
func termToQuerySpec(options *SearchOptions, term []uint16, i, count int) *querySpec {
	spec := &querySpec{
		term:      term,
		fuzzy:     options.Fuzzy, // options.fuzzy || false: only its truthiness is read
		prefix:    options.Prefix.Value,
		termBoost: 1,
	}
	if options.Prefix.LastTerm {
		spec.prefix = i == count-1
	}
	if options.BoostTerm != nil {
		spec.termBoost = termBoost(*options.BoostTerm, term)
	}
	return spec
}

func termBoost(pairs []*TermBoost, term []uint16) float64 {
	for _, pair := range pairs {
		if equal(pair.Term, term) {
			return pair.Factor
		}
	}
	return 1
}

func documentBoost(pairs []*DocumentBoost, id *Value) float64 {
	for _, pair := range pairs {
		if strictEquals(&pair.ID, id) {
			return pair.Factor
		}
	}
	return 1
}

// boostedField is a field to score, with its boost.
type boostedField struct {
	name  string
	boost float64
}

// scoring is what termResults needs besides the terms and their data.
type scoring struct {
	fields        []*boostedField
	termBoost     float64
	boostDocument *[]*DocumentBoost
	bm25          *BM25
}

// executeQuerySpec scores the exact term, then its prefix matches, then its fuzzy matches.
func (s *MiniSearch) executeQuerySpec(query *querySpec, searchOptions *SearchOptions) *results {
	options := s.options.SearchOptions.over(searchOptions)
	// fieldBoosts: an object built from the field list, so a repeated field keeps its first place.
	fieldNames := s.options.Fields
	if options.Fields != nil {
		fieldNames = *options.Fields
	}
	var fields []*boostedField
next:
	for _, field := range fieldNames {
		for _, f := range fields {
			if f.name == field {
				continue next
			}
		}
		fields = append(fields, &boostedField{field, options.fieldBoost(field)})
	}
	// { ...defaultSearchOptions.weights, ...weights }
	fuzzyWeight, prefixWeight := 0.45, 0.375
	if options.Weights.Fuzzy != nil {
		fuzzyWeight = *options.Weights.Fuzzy
	}
	if options.Weights.Prefix != nil {
		prefixWeight = *options.Weights.Prefix
	}
	context := &scoring{fields: fields, termBoost: query.termBoost, boostDocument: options.BoostDocument, bm25: options.BM25}
	term := query.term

	results := newResults()
	s.termResults(term, term, 1, s.index.get(term), context, results)

	// The exact term's cleanup above may have deleted it and merged nodes, which the walks below
	// see (discard-exact-term-merge).
	var prefixMatches []termEntry
	if query.prefix {
		prefixMatches = s.index.atPrefix(term)
	}
	var fuzzyMatches []fuzzyMatch
	if query.fuzzy.truthy() {
		fuzzy := query.fuzzy.Number
		if !query.fuzzy.IsNumber {
			fuzzy = 0.2 // fuzzy: true
		}
		maxDistance := fuzzy
		if fuzzy < 1 {
			// Math.round of a non-negative number: math.Round rounds halves away from zero, up.
			maxDistance = min(*options.MaxFuzzy, math.Round(float64(len(term))*fuzzy))
		}
		if maxDistance != 0 && maxDistance == maxDistance {
			fuzzyMatches = s.index.fuzzyGet(term, int(maxDistance))
		}
	}

	for _, match := range prefixMatches {
		distance := len(match.term) - len(term)
		if distance == 0 {
			continue // the exact match
		}
		length := float64(len(match.term))
		weight := prefixWeight * length / (length + 0.3*float64(distance))
		s.termResults(term, match.term, weight, match.data, context, results)
	}
	for _, match := range fuzzyMatches {
		if match.distance == 0 {
			continue // the exact match
		}
		if query.prefix && len(match.term) > len(term) && hasPrefix(match.term, term) {
			// A term that is also a prefix match is scored as one only: MiniSearch deletes every
			// prefix match from the fuzzy matches, and those are exactly the terms that extend the
			// query term.
			continue
		}
		length := float64(len(match.term))
		weight := fuzzyWeight * length / (length + float64(match.distance))
		s.termResults(term, match.term, weight, match.data, context, results)
	}
	return results
}

func (s *MiniSearch) executeWildcardQuery(searchOptions *SearchOptions) *results {
	options := s.options.SearchOptions.over(searchOptions)
	results := newResults()
	for shortID, doc := range s.documents {
		if doc == nil {
			continue
		}
		score := 1.0
		if options.BoostDocument != nil {
			score = documentBoost(*options.BoostDocument, &doc.id)
		}
		results.add(&result{doc: shortID, score: score})
	}
	return results
}

// termResults adds the BM25+ scores of a term's postings in the scored fields to results.
// derivedTerm is the document term (the query term itself, a prefix match or a fuzzy match).
//
// A posting of a discarded document is skipped and removed (removeTerm), and lowers matchingFields
// for the postings after it in the list. MiniSearch removes it as it meets it, while iterating the
// list; here the removals are made after the term is scored. That is the same sequence of changes
// to the index, and nothing in between reads what they change: scoring reads this term's postings
// only, and removeTerm only deletes postings already passed or decrements their frequency.
func (s *MiniSearch) termResults(sourceTerm, derivedTerm []uint16, termWeight float64, data *termData, context *scoring, results *results) {
	if data == nil {
		return
	}
	type stale struct{ field, doc int }
	var discarded []*stale
	for _, field := range context.fields {
		fieldID, known := s.fieldIDs[field.name]
		if !known {
			continue
		}
		list := data.postings(fieldID)
		if list == nil {
			continue
		}
		matchingFields := len(list.postings)
		avgFieldLength := at(s.avgFieldLength, fieldID)
		for _, p := range list.postings {
			doc := s.documents[p.doc]
			if doc == nil {
				discarded = append(discarded, &stale{fieldID, p.doc})
				matchingFields--
				continue
			}
			docBoost := 1.0
			if context.boostDocument != nil {
				docBoost = documentBoost(*context.boostDocument, &doc.id)
			}
			if docBoost == 0 || docBoost != docBoost {
				continue
			}
			rawScore := calcBM25Score(float64(p.freq), float64(matchingFields), float64(s.documentCount), at(doc.fieldLength, fieldID), avgFieldLength, context.bm25)
			weightedScore := termWeight * context.termBoost * field.boost * docBoost * rawScore
			if r := results.byDoc[p.doc]; r != nil {
				r.score += weightedScore
				r.terms = assignUniqueTerm(r.terms, sourceTerm)
				if m := r.matchFor(derivedTerm); m != nil {
					m.fields = append(m.fields, field.name)
				} else {
					r.match = append(r.match, &matchEntry{derivedTerm, []string{field.name}})
				}
			} else {
				results.add(&result{
					doc:   p.doc,
					score: weightedScore,
					terms: [][]uint16{sourceTerm},
					match: []*matchEntry{{derivedTerm, []string{field.name}}},
				})
			}
		}
	}
	for _, posting := range discarded {
		s.removeTerm(posting.field, posting.doc, derivedTerm)
	}
}

// calcBM25Score is BM25+, operation for operation. Math.log is the host's: GopherJS's math.Log is
// Math.log (NOTES.md). The engines disagree in its last bit, so no other logarithm would match
// every engine's reference.
func calcBM25Score(termFreq, matchingCount, totalCount, fieldLength, avgFieldLength float64, params *BM25) float64 {
	invDocFreq := math.Log(1 + (totalCount-matchingCount+0.5)/(matchingCount+0.5))
	return invDocFreq * (params.D + termFreq*(params.K+1)/(termFreq+params.K*(1-params.B+params.B*fieldLength/avgFieldLength)))
}

// assignUniqueTerm adds term to target unless it is there (includes).
func assignUniqueTerm(target [][]uint16, term []uint16) [][]uint16 {
	for _, t := range target {
		if equal(t, term) {
			return target
		}
	}
	return append(target, term)
}

func assignUniqueTerms(target, source [][]uint16) [][]uint16 {
	for _, term := range source {
		target = assignUniqueTerm(target, term)
	}
	return target
}

// assignMatch is Object.assign(target, source) on two match objects: an existing key keeps its
// place and takes the new field list (it does not concatenate), a new key is added last.
func assignMatch(target, source []*matchEntry) []*matchEntry {
next:
	for _, m := range source {
		for i, t := range target {
			if equal(t.term, m.term) {
				target[i] = &matchEntry{t.term, m.fields}
				continue next
			}
		}
		target = append(target, m)
	}
	return target
}

// combineResults folds the results with the operator, looked up in lower case (combineWith nil
// is the parameter's default, OR). No results is an empty map whatever the operator, so an
// invalid operator is an error only when there is something to combine.
func combineResults(all []*results, combineWith *[]uint16) (*results, error) {
	if len(all) == 0 {
		return newResults(), nil
	}
	operator := encode("or")
	if combineWith != nil {
		operator = *combineWith
	}
	lower := toLowerCase(operator)
	if !equalASCII(lower, "or") && !equalASCII(lower, "and") && !equalASCII(lower, "and_not") {
		return nil, newError(encode("Invalid combination operator: "), operator)
	}
	combined := all[0]
	for _, rs := range all[1:] {
		switch {
		case equalASCII(lower, "or"):
			combined = combineOr(combined, rs)
		case equalASCII(lower, "and"):
			combined = combineAnd(combined, rs)
		default:
			combined = combineAndNot(combined, rs)
		}
	}
	return combined, nil
}

func combineOr(a, b *results) *results {
	for _, incoming := range b.order {
		existing := a.byDoc[incoming.doc]
		if existing == nil {
			a.add(incoming)
			continue
		}
		existing.score = existing.score + incoming.score
		existing.match = assignMatch(existing.match, incoming.match)
		existing.terms = assignUniqueTerms(existing.terms, incoming.terms)
	}
	return a
}

// combineAnd builds a new map in b's order, of the documents in both.
func combineAnd(a, b *results) *results {
	combined := newResults()
	for _, incoming := range b.order {
		existing := a.byDoc[incoming.doc]
		if existing == nil {
			continue
		}
		combined.add(&result{
			doc:   incoming.doc,
			score: existing.score + incoming.score,
			terms: assignUniqueTerms(existing.terms, incoming.terms),
			match: assignMatch(existing.match, incoming.match),
		})
	}
	return combined
}

func combineAndNot(a, b *results) *results {
	kept := newResults()
	for _, r := range a.order {
		if b.byDoc[r.doc] == nil {
			kept.add(r)
		}
	}
	return kept
}
