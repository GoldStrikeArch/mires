// Ky 2.2.0 core/retry-timing.js: which header gives a server-provided retry delay, and that delay.
// The patterns are Ky's own; Kotlin/JS runs them as JS regular expressions.

package ky

import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

internal class TimingHeader(val value: String, val allowTimestamp: Boolean)

/** A reset value is only read as an epoch from this second onwards (2001-09-09). */
private const val TIMESTAMP_THRESHOLD_SECONDS = 1_000_000_000.0
private const val MS_PER_DAY = 86_400_000.0
private val delayPattern = Regex("""^\d+$""")
private val months = listOf("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")

// HTTP-date parsing is limited to IMF-fixdate and the two obsolete HTTP-date formats.
private val imfDatePattern =
    Regex("""^(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun), (\d{2}) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{4}) (\d{2}):(\d{2}):(\d{2}) GMT$""")
private val rfc850DatePattern =
    Regex("""^(?:Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday), (\d{2})-(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)-(\d{2}) (\d{2}):(\d{2}):(\d{2}) GMT$""")
private val asctimeDatePattern =
    Regex("""^(Mon|Tue|Wed|Thu|Fri|Sat|Sun) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{2}| \d) (\d{2}:\d{2}:\d{2}) (\d{4})$""")

internal fun getRetryTimingHeader(headers: Headers): TimingHeader? {
    // Retry-After is authoritative when present. Only missing Retry-After falls through to
    // rate-limit headers.
    headers.get("Retry-After")?.let { return TimingHeader(it, allowTimestamp = false) }
    headers.get("RateLimit-Reset")?.let { return TimingHeader(it, allowTimestamp = true) }
    // Symfony-based services
    headers.get("X-RateLimit-Retry-After")?.let { return TimingHeader(it, allowTimestamp = false) }
    // GitHub, then Twitter
    return (headers.get("X-RateLimit-Reset") ?: headers.get("X-Rate-Limit-Reset"))
        ?.let { TimingHeader(it, allowTimestamp = true) }
}

private fun isLeapYear(year: Int) = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0

private fun daysInMonth(year: Int, month: Int) = when (month) {
    1 -> if (isLeapYear(year)) 29 else 28
    3, 5, 8, 10 -> 30
    else -> 31
}

/** Days from 1970-01-01 to a proleptic Gregorian date, as ECMAScript's MakeDay counts them. A day
 *  past the month's end carries into the next month, as MakeDay does. */
private fun daysFromCivil(year: Int, monthIndex: Int, day: Int): Int {
    val month = monthIndex + 1
    val y = if (month <= 2) year - 1 else year
    val era = y.floorDiv(400)
    val yearOfEra = y - era * 400
    val dayOfYear = (153 * ((month + 9) % 12) + 2) / 5 + day - 1
    val dayOfEra = yearOfEra * 365 + yearOfEra / 4 - yearOfEra / 100 + dayOfYear
    return era * 146_097 + dayOfEra - 719_468
}

/** The year, month index and day of a day count, as getUTCFullYear and its siblings give them. */
private fun civilFromDays(dayCount: Int): Triple<Int, Int, Int> {
    val days = dayCount + 719_468
    val era = days.floorDiv(146_097)
    val dayOfEra = days - era * 146_097
    val yearOfEra = (dayOfEra - dayOfEra / 1460 + dayOfEra / 36_524 - dayOfEra / 146_096) / 365
    val dayOfYear = dayOfEra - (365 * yearOfEra + yearOfEra / 4 - yearOfEra / 100)
    val shiftedMonth = (5 * dayOfYear + 2) / 153
    val day = dayOfYear - (153 * shiftedMonth + 2) / 5 + 1
    val month = if (shiftedMonth < 10) shiftedMonth + 2 else shiftedMonth - 10
    return Triple(yearOfEra + era * 400 + if (month <= 1) 1 else 0, month, day)
}

/** Ky's createTimestamp: Date.UTC of the parts, with the year set again by setUTCFullYear, and
 *  undefined unless every part round-trips. Date.UTC reads a year from 0 to 99 as 1900 + year, so
 *  the day has to exist in that year too. */
private fun createTimestamp(year: Int, month: String, day: String, hours: String, minutes: String, seconds: String): Double? {
    val monthIndex = months.indexOf(month)
    val dayNumber = day.toInt()
    val hoursNumber = hours.toInt()
    val minutesNumber = minutes.toInt()
    val secondsNumber = seconds.toInt()
    if (monthIndex == -1 || hoursNumber > 23 || minutesNumber > 59 || secondsNumber > 60) return null
    val normalizedSeconds = min(secondsNumber, 59)
    val utcYear = if (year in 0..99) year + 1900 else year
    if (dayNumber < 1 || dayNumber > daysInMonth(utcYear, monthIndex) || dayNumber > daysInMonth(year, monthIndex)) {
        return null
    }
    val timestamp = daysFromCivil(year, monthIndex, dayNumber) * MS_PER_DAY +
        (hoursNumber * 3_600_000 + minutesNumber * 60_000 + normalizedSeconds * 1000)
    // A leap second is represented by parsing 60 as 59 and adding one second.
    return if (secondsNumber == 60) timestamp + 1000 else timestamp
}

private fun parseDate(value: String): Double? {
    imfDatePattern.find(value)?.groupValues?.let { date ->
        return createTimestamp(date[3].toInt(), date[2], date[1], date[4], date[5], date[6])
    }
    rfc850DatePattern.find(value)?.groupValues?.let { date ->
        val now = JsDate.now()
        val today = floor(now / MS_PER_DAY).toInt()
        val (currentYear, currentMonth, currentDay) = civilFromDays(today)
        val currentCenturyYear = currentYear.floorDiv(100) * 100 + date[3].toInt()
        val fiftyYearsFromNow =
            daysFromCivil(currentYear + 50, currentMonth, currentDay) * MS_PER_DAY + (now - today * MS_PER_DAY)
        // RFC 9110 maps two-digit years to the latest matching century that is not more than 50
        // years in the future.
        var timestamp: Double? = null
        for (year in listOf(currentCenturyYear - 100, currentCenturyYear, currentCenturyYear + 100)) {
            val candidate = createTimestamp(year, date[2], date[1], date[4], date[5], date[6])
            if (candidate != null && candidate <= fiftyYearsFromNow) timestamp = candidate
        }
        return timestamp
    }
    return asctimeDatePattern.find(value)?.groupValues?.let { date ->
        val (hours, minutes, seconds) = date[4].split(":")
        createTimestamp(date[5].toInt(), date[2], date[3].trim(), hours, minutes, seconds)
    }
}

internal fun calculateRetryTimingDelay(header: TimingHeader): Double? {
    if (delayPattern.containsMatchIn(header.value)) {
        val seconds = header.value.toDouble()
        // Standard Retry-After numbers are delay seconds. Timestamp compatibility only applies to
        // reset headers.
        val delay = if (header.allowTimestamp && seconds >= TIMESTAMP_THRESHOLD_SECONDS) {
            seconds * 1000 - JsDate.now()
        } else {
            seconds * 1000
        }
        return max(0.0, delay)
    }
    // createTimestamp only returns valid dates, so the delay is always finite.
    return parseDate(header.value)?.let { max(0.0, it - JsDate.now()) }
}
