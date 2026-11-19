// Index options, search options and their merging.
//
// MiniSearch merges option objects by spreading them ({ ...a, ...b }): a key b holds replaces a's
// whole value, so object options (boost, weights, bm25) are replaced, never merged key by key.
// Every search option is a pointer, nil meaning "the object has no such key", and
// SearchOptions.over is that spread. (Pointers, not values with a "set" flag: a merge then copies
// pointers, where GopherJS would copy every nested struct field by field.)

package main

// Prefix is the prefix option: a boolean, or autoSuggest's default function
// (term, i, terms) => i === terms.length - 1.
type Prefix struct {
	LastTerm bool // the function
	Value    bool
}

// Fuzzy is the fuzzy option: a boolean, or a number (a fraction of the term's length, or a
// distance).
type Fuzzy struct {
	IsNumber bool
	Flag     bool
	Number   float64
}

// truthy is the JavaScript truthiness MiniSearch tests (options.fuzzy || false, if (query.fuzzy)).
func (f *Fuzzy) truthy() bool {
	if f.IsNumber {
		return f.Number != 0 && f.Number == f.Number
	}
	return f.Flag
}

// Weights is the weights option; a missing weight comes from the library default.
type Weights struct {
	Fuzzy, Prefix *float64
}

// BM25 is the bm25 option, the BM25+ parameters.
type BM25 struct {
	K, B, D float64
}

// Filter is the declarative filter (reference.mjs): (result) => values.includes(result[field]).
type Filter struct {
	Field  string
	Values []*Value
}

// DocumentBoost is a pair of the declarative boostDocument:
// (id) => the factor of the first pair whose id === id, else 1.
type DocumentBoost struct {
	ID     Value
	Factor float64
}

// TermBoost is a pair of the declarative boostTerm:
// (term) => the factor of the first pair whose term === term, else 1.
type TermBoost struct {
	Term   []uint16
	Factor float64
}

// SearchOptions are the options of search and autoSuggest, as one object gives them.
type SearchOptions struct {
	Fields        *[]string
	Boost         map[string]float64 // nil: no boost key ({} is an empty, non-nil map)
	Prefix        *Prefix
	Fuzzy         *Fuzzy
	MaxFuzzy      *float64
	CombineWith   *[]uint16
	Weights       *Weights
	BM25          *BM25
	Filter        *Filter
	BoostDocument *[]*DocumentBoost
	BoostTerm     *[]*TermBoost
}

// over is { ...o, ...b }.
func (o *SearchOptions) over(b *SearchOptions) *SearchOptions {
	merged := *o
	if b.Fields != nil {
		merged.Fields = b.Fields
	}
	if b.Boost != nil {
		merged.Boost = b.Boost
	}
	if b.Prefix != nil {
		merged.Prefix = b.Prefix
	}
	if b.Fuzzy != nil {
		merged.Fuzzy = b.Fuzzy
	}
	if b.MaxFuzzy != nil {
		merged.MaxFuzzy = b.MaxFuzzy
	}
	if b.CombineWith != nil {
		merged.CombineWith = b.CombineWith
	}
	if b.Weights != nil {
		merged.Weights = b.Weights
	}
	if b.BM25 != nil {
		merged.BM25 = b.BM25
	}
	if b.Filter != nil {
		merged.Filter = b.Filter
	}
	if b.BoostDocument != nil {
		merged.BoostDocument = b.BoostDocument
	}
	if b.BoostTerm != nil {
		merged.BoostTerm = b.BoostTerm
	}
	return &merged
}

// fieldBoost is getOwnProperty(options.boost, field) || 1.
func (o *SearchOptions) fieldBoost(field string) float64 {
	return or(o.Boost[field], 1)
}

// defaultSearchOptions is MiniSearch's defaultSearchOptions.
func defaultSearchOptions() *SearchOptions {
	combineWith, maxFuzzy, fuzzyWeight, prefixWeight := encode("or"), 6.0, 0.45, 0.375
	return &SearchOptions{
		CombineWith: &combineWith,
		Prefix:      &Prefix{},
		Fuzzy:       &Fuzzy{},
		MaxFuzzy:    &maxFuzzy,
		Boost:       map[string]float64{},
		Weights:     &Weights{Fuzzy: &fuzzyWeight, Prefix: &prefixWeight},
		BM25:        &BM25{K: 1.2, B: 0.7, D: 0.5},
	}
}

// defaultAutoSuggestOptions is MiniSearch's defaultAutoSuggestOptions.
func defaultAutoSuggestOptions() *SearchOptions {
	and := encode("and")
	return &SearchOptions{CombineWith: &and, Prefix: &Prefix{LastTerm: true}}
}

// AutoVacuum is the autoVacuum option when it is not false. MiniSearch reads each value with
// `|| default` (batchSize 1000, minDirtCount 20, minDirtFactor 0.1), so a missing value is 0 here,
// which that `||` treats as it treats undefined.
type AutoVacuum struct {
	BatchSize, MinDirtCount, MinDirtFactor float64
}

// Options are the constructor's options.
type Options struct {
	Fields      []string
	IDField     string
	StoreFields []string
	// The library defaults under the constructor's searchOptions and autoSuggestOptions.
	SearchOptions      *SearchOptions
	AutoSuggestOptions *SearchOptions
	// nil for autoVacuum: false; true or absent are the default conditions.
	AutoVacuum *AutoVacuum
}

// or is a JavaScript `value || fallback` on numbers: 0 and NaN are falsy.
func or(value, fallback float64) float64 {
	if value == 0 || value != value {
		return fallback
	}
	return value
}
