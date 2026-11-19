// Word frequencies, FNV-1a and a fixed-decimal summary, as specified in
// libraries/pilot/CONTRACT.md. Go strings are UTF-8; `range` yields runes.

package main

import (
	"hash/fnv"
	"sort"
	"strconv"
	"strings"
)

type entry struct {
	word   string
	count  int
	length int
}

func isWordRune(r rune) bool {
	return r >= 0x80 || ('0' <= r && r <= '9') || ('A' <= r && r <= 'Z') || ('a' <= r && r <= 'z')
}

// fnv1a is FNV-1a 32 over the UTF-8 bytes of text, through the standard library.
func fnv1a(text string) uint32 {
	hash := fnv.New32a()
	hash.Write([]byte(text))
	return hash.Sum32()
}

func hex8(value uint32) string {
	digits := strconv.FormatUint(uint64(value), 16)
	return strings.Repeat("0", 8-len(digits)) + digits
}

func fixed3(value float64) string {
	return strconv.FormatFloat(value, 'f', 3, 64)
}

func analyse(text string, top int) string {
	var entries []*entry
	index := map[string]int{}
	tokens, codePoints, length := 0, 0, 0
	var word strings.Builder
	flush := func() {
		if length == 0 {
			return
		}
		tokens++
		codePoints += length
		key := word.String()
		if at, ok := index[key]; ok {
			entries[at].count++
		} else {
			index[key] = len(entries)
			entries = append(entries, &entry{word: key, count: 1, length: length})
		}
		word.Reset()
		length = 0
	}
	for _, r := range text {
		if !isWordRune(r) {
			flush()
			continue
		}
		// ASCII-only case folding.
		if 'A' <= r && r <= 'Z' {
			r += 'a' - 'A'
		}
		word.WriteRune(r)
		length++
	}
	flush()

	// SliceStable keeps first-occurrence order among equal keys.
	sort.SliceStable(entries, func(i, j int) bool {
		if entries[i].count != entries[j].count {
			return entries[i].count > entries[j].count
		}
		return entries[i].length > entries[j].length
	})
	mean := 0.0
	if tokens > 0 {
		mean = float64(codePoints) / float64(tokens)
	}
	lines := []string{
		"tokens " + strconv.Itoa(tokens),
		"distinct " + strconv.Itoa(len(entries)),
		"mean " + fixed3(mean),
		"fnv " + hex8(fnv1a(text)),
	}
	for i, e := range entries {
		if i >= top {
			break
		}
		share := float64(e.count) / float64(tokens)
		lines = append(lines, e.word+" "+strconv.Itoa(e.count)+" "+hex8(fnv1a(e.word))+" "+fixed3(share))
	}
	return strings.Join(lines, "\n")
}
