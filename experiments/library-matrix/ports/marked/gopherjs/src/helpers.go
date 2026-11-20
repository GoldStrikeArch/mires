// Helper functions mirroring marked 18.1.1 src/helpers.ts, without stdlib bloat or JS closures.
package main

import "github.com/gopherjs/gopherjs/js"

// intToString formats a non-negative integer as decimal ASCII without importing strconv.
func intToString(n int) string {
	if n == 0 {
		return "0"
	}
	var buf [12]byte
	i := len(buf)
	neg := n < 0
	if neg {
		n = -n
	}
	for n > 0 {
		i--
		buf[i] = byte('0' + n%10)
		n /= 10
	}
	if neg {
		i--
		buf[i] = '-'
	}
	return string(buf[i:])
}

func isTruthy(v jsStr) bool {
	return v != js.Undefined && v != nil && v.Length() > 0
}

func escapeCharEntity(ch int) jsStr {
	switch ch {
	case '&':
		return jsAmp
	case '<':
		return jsLt
	case '>':
		return jsGt
	case '"':
		return jsQuot
	case '\'':
		return jsApos
	}
	return jsEmpty
}

// escapeHtmlEntities mirrors helpers.ts escapeHtmlEntities.
func escapeHtmlEntities(html jsStr, encode bool) jsStr {
	var re *js.Object
	if encode {
		if !otherGrammar.escapeTest.Call("test", html).Bool() {
			return html
		}
		re = otherGrammar.escapeReplace
	} else {
		if !otherGrammar.escapeTestNoEncode.Call("test", html).Bool() {
			return html
		}
		re = otherGrammar.escapeReplaceNoEncode
	}

	re.Set("lastIndex", 0)
	out := jsEmpty
	lastEnd := 0
	for {
		m := re.Call("exec", html)
		if m == nil {
			break
		}
		idx := m.Get("index").Int()
		if idx > lastEnd {
			out = out.Call("concat", html.Call("slice", lastEnd, idx))
		}
		ch := m.Index(0).Call("charCodeAt", 0).Int()
		out = out.Call("concat", escapeCharEntity(ch))
		lastEnd = idx + 1
	}
	if lastEnd < html.Length() {
		out = out.Call("concat", html.Call("slice", lastEnd))
	}
	return out
}

func parseDecJS(s jsStr) int {
	n := s.Length()
	v := 0
	for i := 0; i < n; i++ {
		v = v*10 + (s.Call("charCodeAt", i).Int() - '0')
	}
	return v
}

func parseHexJS(s jsStr) int {
	n := s.Length()
	v := 0
	for i := 0; i < n; i++ {
		c := s.Call("charCodeAt", i).Int()
		var d int
		if c >= '0' && c <= '9' {
			d = c - '0'
		} else if c >= 'a' && c <= 'f' {
			d = c - 'a' + 10
		} else {
			d = c - 'A' + 10
		}
		v = (v << 4) | d
	}
	return v
}

// decodeNumericCharacterReferences mirrors helpers.ts decodeNumericCharacterReferences.
func decodeNumericCharacterReferences(text jsStr) jsStr {
	if !otherGrammar.numericCharacterReference.Call("test", text).Bool() {
		return text
	}
	re := otherGrammar.numericCharacterReferenceOrAmp
	re.Set("lastIndex", 0)
	out := jsEmpty
	lastEnd := 0
	for {
		m := re.Call("exec", text)
		if m == nil {
			break
		}
		idx := m.Get("index").Int()
		if idx > lastEnd {
			out = out.Call("concat", text.Call("slice", lastEnd, idx))
		}
		dec := m.Index(1)
		hex := m.Index(2)
		if dec == js.Undefined && hex == js.Undefined {
			out = out.Call("concat", jsAmp)
		} else {
			var code int
			if dec != js.Undefined {
				code = parseDecJS(dec)
			} else {
				code = parseHexJS(hex)
			}
			if code == 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff) {
				out = out.Call("concat", jsReplacementChar)
			} else if code == 38 {
				out = out.Call("concat", jsAmp)
			} else {
				out = out.Call("concat", jsFromCodePoint.Invoke(code))
			}
		}
		lastEnd = idx + m.Index(0).Length()
	}
	if lastEnd < text.Length() {
		out = out.Call("concat", text.Call("slice", lastEnd))
	}
	return out
}

// cleanUrl mirrors helpers.ts cleanUrl, checking for lone surrogates upfront to avoid URIError.
func cleanUrl(href jsStr) jsStr {
	if otherGrammar.loneSurrogate.Call("test", href).Bool() {
		return nil
	}
	return jsEncodeURI.Invoke(href).Call("replace", otherGrammar.percentDecode, jsPercent)
}

// splitCells mirrors helpers.ts splitCells.
func splitCells(tableRow jsStr, count int) []jsStr {
	// Insert a space before every unescaped '|' (even number of preceding '\').
	row := jsEmpty
	lastEnd := 0
	pos := 0
	for {
		idx := tableRow.Call("indexOf", jsPipe, pos).Int()
		if idx == -1 {
			break
		}
		escaped := false
		curr := idx
		for {
			curr--
			if curr < 0 || tableRow.Call("charCodeAt", curr).Int() != '\\' {
				break
			}
			escaped = !escaped
		}
		if !escaped {
			if idx > lastEnd {
				row = row.Call("concat", tableRow.Call("slice", lastEnd, idx))
			}
			row = row.Call("concat", jsSpacePipe)
			lastEnd = idx + 1
		}
		pos = idx + 1
	}
	if lastEnd == 0 {
		row = tableRow
	} else if lastEnd < tableRow.Length() {
		row = row.Call("concat", tableRow.Call("slice", lastEnd))
	}

	rawCells := row.Call("split", otherGrammar.splitPipe)
	n := rawCells.Length()
	cells := make([]jsStr, n)
	for i := 0; i < n; i++ {
		cells[i] = rawCells.Index(i)
	}

	if len(cells) > 0 && cells[0].Call("trim").Length() == 0 {
		cells = cells[1:]
	}
	if len(cells) > 0 && cells[len(cells)-1].Call("trim").Length() == 0 {
		cells = cells[:len(cells)-1]
	}

	if count > 0 {
		if len(cells) > count {
			cells = cells[:count]
		} else {
			for len(cells) < count {
				cells = append(cells, jsEmpty)
			}
		}
	}

	for i := 0; i < len(cells); i++ {
		cells[i] = cells[i].Call("trim").Call("replace", otherGrammar.slashPipe, jsPipe)
	}
	return cells
}

// rtrim mirrors helpers.ts rtrim for a single ASCII code unit c.
func rtrim(str jsStr, c int, invert bool) jsStr {
	l := str.Length()
	if l == 0 {
		return jsEmpty
	}
	suffLen := 0
	for suffLen < l {
		currChar := str.Call("charCodeAt", l-suffLen-1).Int()
		if currChar == c && !invert {
			suffLen++
		} else if currChar != c && invert {
			suffLen++
		} else {
			break
		}
	}
	if suffLen == 0 {
		return str
	}
	return str.Call("slice", 0, l-suffLen)
}

// trimTrailingBlankLines mirrors helpers.ts trimTrailingBlankLines.
func trimTrailingBlankLines(str jsStr) jsStr {
	lines := str.Call("split", jsNewline)
	n := lines.Length()
	end := n - 1
	for end >= 0 && otherGrammar.blankLine.Call("test", lines.Index(end)).Bool() {
		end--
	}
	if n-end <= 2 {
		return str
	}
	return lines.Call("slice", 0, end+1).Call("join", jsNewline)
}

// normalizeLabel mirrors helpers.ts normalizeLabel (full Unicode case fold).
func normalizeLabel(label jsStr) jsStr {
	return label.Call("trim").Call("toLowerCase").Call("toUpperCase").Call("toLowerCase")
}

// findClosingBracket mirrors helpers.ts findClosingBracket(str, "()").
func findClosingBracket(str jsStr) int {
	if str.Call("indexOf", jsLParen).Int() == -1 && str.Call("indexOf", jsRParen).Int() == -1 {
		return -1
	}
	level := 0
	n := str.Length()
	for i := 0; i < n; i++ {
		ch := str.Call("charCodeAt", i).Int()
		if ch == '\\' {
			i++
		} else if ch == '(' {
			level++
		} else if ch == ')' {
			level--
			if level < 0 {
				return i
			}
		}
	}
	if level > 0 {
		return -2
	}
	return -1
}

// expandTabs mirrors helpers.ts expandTabs, counting Unicode scalar values for column tracking.
func expandTabs(line jsStr, indent int) jsStr {
	if line.Call("indexOf", jsTab).Int() == -1 {
		return line
	}
	n := line.Length()
	col := indent
	out := jsEmpty
	lastEnd := 0
	for i := 0; i < n; i++ {
		ch := line.Call("charCodeAt", i).Int()
		if ch == '\t' {
			if i > lastEnd {
				out = out.Call("concat", line.Call("slice", lastEnd, i))
			}
			added := 4 - (col % 4)
			out = out.Call("concat", jsSpaces[added])
			col += added
			lastEnd = i + 1
		} else {
			if ch >= 0xD800 && ch <= 0xDBFF && i+1 < n {
				lo := line.Call("charCodeAt", i+1).Int()
				if lo >= 0xDC00 && lo <= 0xDFFF {
					i++
				}
			}
			col++
		}
	}
	if lastEnd < n {
		out = out.Call("concat", line.Call("slice", lastEnd))
	}
	return out
}

// expandLeadingTabs replaces /^[ \t]+/ with expandTabs(whitespace, indent) without a JS callback.
func expandLeadingTabs(line jsStr, indent int) jsStr {
	n := line.Length()
	end := 0
	hasTab := false
	for end < n {
		ch := line.Call("charCodeAt", end).Int()
		if ch == ' ' {
			end++
		} else if ch == '\t' {
			hasTab = true
			end++
		} else {
			break
		}
	}
	if !hasTab {
		return line
	}
	return expandTabs(line.Call("slice", 0, end), indent).Call("concat", line.Call("slice", end))
}

// firstLineOf returns the prefix before the first '\n', equivalent to s.split('\n', 1)[0].
func firstLineOf(s jsStr) jsStr {
	idx := s.Call("indexOf", jsNewline).Int()
	if idx == -1 {
		return s
	}
	return s.Call("slice", 0, idx)
}

// searchNonSpace returns the index of the first char != ' ', equivalent to s.search(/[^ ]/).
func searchNonSpace(s jsStr) int {
	n := s.Length()
	for i := 0; i < n; i++ {
		if s.Call("charCodeAt", i).Int() != ' ' {
			return i
		}
	}
	return -1
}

// codePointLen counts Unicode code points in s, matching [...s].length.
func codePointLen(s jsStr) int {
	n := s.Length()
	count := 0
	for i := 0; i < n; i++ {
		ch := s.Call("charCodeAt", i).Int()
		if ch >= 0xD800 && ch <= 0xDBFF && i+1 < n {
			lo := s.Call("charCodeAt", i+1).Int()
			if lo >= 0xDC00 && lo <= 0xDFFF {
				i++
			}
		}
		count++
	}
	return count
}

// firstCodePointLen returns the UTF-16 code-unit length of the first code point of s,
// matching [...s][0].length.
func firstCodePointLen(s jsStr) int {
	if s.Length() >= 2 {
		hi := s.Call("charCodeAt", 0).Int()
		if hi >= 0xD800 && hi <= 0xDBFF {
			lo := s.Call("charCodeAt", 1).Int()
			if lo >= 0xDC00 && lo <= 0xDFFF {
				return 2
			}
		}
	}
	return 1
}

func splitToSlice(s jsStr) []jsStr {
	arr := s.Call("split", jsNewline)
	n := arr.Length()
	out := make([]jsStr, n)
	for i := 0; i < n; i++ {
		out[i] = arr.Index(i)
	}
	return out
}

func joinLines(lines []jsStr) jsStr {
	n := len(lines)
	if n == 0 {
		return jsEmpty
	}
	out := lines[0]
	for i := 1; i < n; i++ {
		out = out.Call("concat", jsNewline, lines[i])
	}
	return out
}
