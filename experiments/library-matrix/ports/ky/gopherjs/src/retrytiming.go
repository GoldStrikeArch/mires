// Ky 2.2.0 core/retry-timing.js: which header gives a server-provided retry delay, and that delay.
// Ky matches the three HTTP-date forms with anchored regular expressions made of ASCII literals
// and \d; scanner matches the same strings, piece by piece.

package main

import (
	"math"
	"strings"
	"time"
)

type timingHeader struct {
	value          string
	allowTimestamp bool
}

const (
	// A reset value is only read as an epoch from this second onwards (2001-09-09).
	timestampThresholdSeconds = 1e9
	msPerDay                  = 86400000.0
)

var (
	months       = []string{"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"}
	weekdays     = []string{"Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"}
	weekdayNames = []string{"Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"}
	// Retry-After is authoritative when present. Only missing Retry-After falls through to the
	// rate-limit headers: RateLimit-Reset, Symfony's X-RateLimit-Retry-After, then GitHub's and
	// Twitter's reset headers.
	timingHeaders = []timingHeader{
		{"Retry-After", false},
		{"RateLimit-Reset", true},
		{"X-RateLimit-Retry-After", false},
		{"X-RateLimit-Reset", true},
		{"X-Rate-Limit-Reset", true},
	}
)

func getRetryTimingHeader(headers Headers) (timingHeader, bool) {
	for _, name := range timingHeaders {
		if value, ok := headers.Get(name.value); ok {
			return timingHeader{value, name.allowTimestamp}, true
		}
	}
	return timingHeader{}, false
}

// isDelay is /^\d+$/: one or more ASCII digits and nothing else.
func isDelay(value string) bool {
	if value == "" {
		return false
	}
	for i := 0; i < len(value); i++ {
		if value[i] < '0' || value[i] > '9' {
			return false
		}
	}
	return true
}

// scanner consumes the pieces of an anchored pattern from the front of the value; ok turns false
// at the first piece that does not match.
type scanner struct {
	rest string
	ok   bool
}

func (s *scanner) literal(expected string) {
	if s.ok && strings.HasPrefix(s.rest, expected) {
		s.rest = s.rest[len(expected):]
	} else {
		s.ok = false
	}
}

// word is an alternation of words: the index of the one that matches.
func (s *scanner) word(words []string) int {
	if s.ok {
		for index, word := range words {
			if strings.HasPrefix(s.rest, word) {
				s.rest = s.rest[len(word):]
				return index
			}
		}
	}
	s.ok = false
	return -1
}

// digits is \d{count}, as its value.
func (s *scanner) digits(count int) int {
	if !s.ok || len(s.rest) < count || !isDelay(s.rest[:count]) {
		s.ok = false
		return 0
	}
	value := 0
	for i := 0; i < count; i++ {
		value = value*10 + int(s.rest[i]-'0')
	}
	s.rest = s.rest[count:]
	return value
}

// time is "08:49:37".
func (s *scanner) time() (hours, minutes, seconds int) {
	hours = s.digits(2)
	s.literal(":")
	minutes = s.digits(2)
	s.literal(":")
	seconds = s.digits(2)
	return
}

func (s *scanner) end() bool { return s.ok && s.rest == "" }

func isLeapYear(year int) bool { return (year%4 == 0 && year%100 != 0) || year%400 == 0 }

func daysInMonth(year, month int) int {
	switch month {
	case 1:
		if isLeapYear(year) {
			return 29
		}
		return 28
	case 3, 5, 8, 10:
		return 30
	}
	return 31
}

// daysFromCivil counts the days from 1970-01-01 to a proleptic Gregorian date, as ECMAScript's
// MakeDay does. A day past the month's end carries into the next month, as in MakeDay.
func daysFromCivil(year, monthIndex, day int) int {
	month := monthIndex + 1
	if month <= 2 {
		year--
	}
	era := year / 400
	if year < 0 {
		era = (year - 399) / 400
	}
	yearOfEra := year - era*400
	dayOfYear := (153*((month+9)%12)+2)/5 + day - 1
	dayOfEra := yearOfEra*365 + yearOfEra/4 - yearOfEra/100 + dayOfYear
	return era*146097 + dayOfEra - 719468
}

// civilFromDays is the year, month index and day of a day count, as getUTCFullYear and its
// siblings give them.
func civilFromDays(dayCount int) (year, month, day int) {
	days := dayCount + 719468
	era := days / 146097
	if days < 0 {
		era = (days - 146096) / 146097
	}
	dayOfEra := days - era*146097
	yearOfEra := (dayOfEra - dayOfEra/1460 + dayOfEra/36524 - dayOfEra/146096) / 365
	dayOfYear := dayOfEra - (365*yearOfEra + yearOfEra/4 - yearOfEra/100)
	shiftedMonth := (5*dayOfYear + 2) / 153
	day = dayOfYear - (153*shiftedMonth+2)/5 + 1
	month = shiftedMonth + 2
	if shiftedMonth >= 10 {
		month = shiftedMonth - 10
	}
	year = yearOfEra + era*400
	if month <= 1 {
		year++
	}
	return
}

// createTimestamp is Ky's: Date.UTC of the parts, with the year set again by setUTCFullYear, and
// undefined unless every part round-trips. Date.UTC reads a year from 0 to 99 as 1900 + year, so
// the day has to exist in that year too. Go's time.Date does neither, so the dates are counted
// here.
func createTimestamp(year, month, day, hours, minutes, seconds int) (float64, bool) {
	if hours > 23 || minutes > 59 || seconds > 60 {
		return 0, false
	}
	normalizedSeconds := seconds
	if normalizedSeconds > 59 {
		normalizedSeconds = 59
	}
	utcYear := year
	if year >= 0 && year <= 99 {
		utcYear += 1900
	}
	if day < 1 || day > daysInMonth(utcYear, month) || day > daysInMonth(year, month) {
		return 0, false
	}
	timestamp := float64(daysFromCivil(year, month, day))*msPerDay +
		float64(hours*3600000+minutes*60000+normalizedSeconds*1000)
	// A leap second is represented by parsing 60 as 59 and adding one second.
	if seconds == 60 {
		timestamp += 1000
	}
	return timestamp, true
}

// parseDate is Ky's: the first of the three forms whose pattern matches decides.
func parseDate(value string) (float64, bool) {
	// IMF-fixdate, "Sun, 06 Nov 1994 08:49:37 GMT".
	imf := scanner{rest: value, ok: true}
	imf.word(weekdays)
	imf.literal(", ")
	day := imf.digits(2)
	imf.literal(" ")
	month := imf.word(months)
	imf.literal(" ")
	year := imf.digits(4)
	imf.literal(" ")
	hours, minutes, seconds := imf.time()
	imf.literal(" GMT")
	if imf.end() {
		return createTimestamp(year, month, day, hours, minutes, seconds)
	}

	// RFC 850, "Sunday, 06-Nov-94 08:49:37 GMT". RFC 9110 maps two-digit years to the latest
	// matching century that is not more than 50 years in the future.
	rfc850 := scanner{rest: value, ok: true}
	rfc850.word(weekdayNames)
	rfc850.literal(", ")
	day = rfc850.digits(2)
	rfc850.literal("-")
	month = rfc850.word(months)
	rfc850.literal("-")
	twoDigitYear := rfc850.digits(2)
	rfc850.literal(" ")
	hours, minutes, seconds = rfc850.time()
	rfc850.literal(" GMT")
	if rfc850.end() {
		now := float64(time.Now().UnixMilli())
		today := int(math.Floor(now / msPerDay))
		currentYear, currentMonth, currentDay := civilFromDays(today)
		currentCenturyYear := currentYear/100*100 + twoDigitYear
		fiftyYearsFromNow := float64(daysFromCivil(currentYear+50, currentMonth, currentDay))*msPerDay +
			(now - float64(today)*msPerDay)
		timestamp, found := 0.0, false
		for _, candidateYear := range []int{currentCenturyYear - 100, currentCenturyYear, currentCenturyYear + 100} {
			if candidate, ok := createTimestamp(candidateYear, month, day, hours, minutes, seconds); ok && candidate <= fiftyYearsFromNow {
				timestamp, found = candidate, true
			}
		}
		return timestamp, found
	}

	// asctime, "Sun Nov  6 08:49:37 1994": the day is two digits, or a space and one digit.
	asctime := scanner{rest: value, ok: true}
	asctime.word(weekdays)
	asctime.literal(" ")
	month = asctime.word(months)
	asctime.literal(" ")
	if asctime.ok && strings.HasPrefix(asctime.rest, " ") {
		asctime.literal(" ")
		day = asctime.digits(1)
	} else {
		day = asctime.digits(2)
	}
	asctime.literal(" ")
	hours, minutes, seconds = asctime.time()
	asctime.literal(" ")
	year = asctime.digits(4)
	if asctime.end() {
		return createTimestamp(year, month, day, hours, minutes, seconds)
	}
	return 0, false
}

func calculateRetryTimingDelay(header timingHeader) (float64, bool) {
	now := func() float64 { return float64(time.Now().UnixMilli()) }
	if isDelay(header.value) {
		seconds := toNumber(header.value)
		// Standard Retry-After numbers are delay seconds. Timestamp compatibility only applies to
		// reset headers.
		delay := seconds * 1000
		if header.allowTimestamp && seconds >= timestampThresholdSeconds {
			delay -= now()
		}
		return math.Max(0, delay), true
	}
	// createTimestamp only returns valid dates, so the delay is always finite.
	timestamp, ok := parseDate(header.value)
	if !ok {
		return 0, false
	}
	return math.Max(0, timestamp-now()), true
}
