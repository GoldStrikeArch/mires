// Ky 2.2.0 core/retry-timing.js: which header gives a server-provided retry delay, and that delay.
// The patterns are Ky's own, run as JS regular expressions (`js.RegExp`): Scala's own regex follows
// java.util.regex, whose classes differ from JS's.

package ky

import scala.scalajs.js

final case class TimingHeader(value: String, allowTimestamp: Boolean)

object RetryTiming:
  /** A reset value is only read as an epoch from this second onwards (2001-09-09). */
  private val TimestampThresholdSeconds = 1_000_000_000.0
  private val MsPerDay = 86_400_000.0
  private val delayPattern = js.RegExp("""^\d+$""")
  private val months = Vector("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")

  // HTTP-date parsing is limited to IMF-fixdate and the two obsolete HTTP-date formats.
  private val imfDatePattern = js.RegExp(
    """^(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun), (\d{2}) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{4}) (\d{2}):(\d{2}):(\d{2}) GMT$"""
  )
  private val rfc850DatePattern = js.RegExp(
    """^(?:Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday), (\d{2})-(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)-(\d{2}) (\d{2}):(\d{2}):(\d{2}) GMT$"""
  )
  private val asctimeDatePattern = js.RegExp(
    """^(Mon|Tue|Wed|Thu|Fri|Sat|Sun) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{2}| \d) (\d{2}:\d{2}:\d{2}) (\d{4})$"""
  )

  /** Retry-After is authoritative when present. Only missing Retry-After falls through to the
    * rate-limit headers: RateLimit-Reset, Symfony's X-RateLimit-Retry-After, then GitHub's and
    * Twitter's reset headers.
    */
  private val timingHeaders = List(
    "Retry-After" -> false,
    "RateLimit-Reset" -> true,
    "X-RateLimit-Retry-After" -> false,
    "X-RateLimit-Reset" -> true,
    "X-Rate-Limit-Reset" -> true
  )

  def getRetryTimingHeader(headers: Headers): Option[TimingHeader] =
    timingHeaders.iterator
      .flatMap((name, allowTimestamp) => Option(headers.get(name)).map(TimingHeader(_, allowTimestamp)))
      .nextOption()

  private def isLeapYear(year: Int) = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0

  private def daysInMonth(year: Int, month: Int) = month match
    case 1 => if isLeapYear(year) then 29 else 28
    case 3 | 5 | 8 | 10 => 30
    case _ => 31

  /** Days from 1970-01-01 to a proleptic Gregorian date, as ECMAScript's MakeDay counts them. A day
    * past the month's end carries into the next month, as MakeDay does.
    */
  private def daysFromCivil(year: Int, monthIndex: Int, day: Int): Int =
    val month = monthIndex + 1
    val y = if month <= 2 then year - 1 else year
    val era = Math.floorDiv(y, 400)
    val yearOfEra = y - era * 400
    val dayOfYear = (153 * ((month + 9) % 12) + 2) / 5 + day - 1
    val dayOfEra = yearOfEra * 365 + yearOfEra / 4 - yearOfEra / 100 + dayOfYear
    era * 146_097 + dayOfEra - 719_468

  /** The year, month index and day of a day count, as getUTCFullYear and its siblings give them. */
  private def civilFromDays(dayCount: Int): (Int, Int, Int) =
    val days = dayCount + 719_468
    val era = Math.floorDiv(days, 146_097)
    val dayOfEra = days - era * 146_097
    val yearOfEra = (dayOfEra - dayOfEra / 1460 + dayOfEra / 36_524 - dayOfEra / 146_096) / 365
    val dayOfYear = dayOfEra - (365 * yearOfEra + yearOfEra / 4 - yearOfEra / 100)
    val shiftedMonth = (5 * dayOfYear + 2) / 153
    val day = dayOfYear - (153 * shiftedMonth + 2) / 5 + 1
    val month = if shiftedMonth < 10 then shiftedMonth + 2 else shiftedMonth - 10
    (yearOfEra + era * 400 + (if month <= 1 then 1 else 0), month, day)

  /** Ky's createTimestamp: Date.UTC of the parts, with the year set again by setUTCFullYear, and
    * undefined unless every part round-trips. Date.UTC reads a year from 0 to 99 as 1900 + year, so
    * the day has to exist in that year too.
    */
  private def createTimestamp(
      year: Int,
      month: String,
      day: String,
      hours: String,
      minutes: String,
      seconds: String
  ): Option[Double] =
    val monthIndex = months.indexOf(month)
    val (dayNumber, hoursNumber, minutesNumber, secondsNumber) = (day.toInt, hours.toInt, minutes.toInt, seconds.toInt)
    if monthIndex == -1 || hoursNumber > 23 || minutesNumber > 59 || secondsNumber > 60 then None
    else
      val normalizedSeconds = secondsNumber.min(59)
      val utcYear = if year >= 0 && year <= 99 then year + 1900 else year
      if dayNumber < 1 || dayNumber > daysInMonth(utcYear, monthIndex) || dayNumber > daysInMonth(year, monthIndex) then None
      else
        val timestamp = daysFromCivil(year, monthIndex, dayNumber) * MsPerDay +
          (hoursNumber * 3_600_000 + minutesNumber * 60_000 + normalizedSeconds * 1000)
        // A leap second is represented by parsing 60 as 59 and adding one second.
        Some(if secondsNumber == 60 then timestamp + 1000 else timestamp)

  /** The pattern's groups, when it matches. */
  private def exec(pattern: js.RegExp, value: String): Option[IndexedSeq[String]] =
    Option(pattern.exec(value)).map(_.toIndexedSeq.map(_.getOrElse("")))

  private def parseDate(value: String): Option[Double] =
    exec(imfDatePattern, value) match
      case Some(date) => createTimestamp(date(3).toInt, date(2), date(1), date(4), date(5), date(6))
      case None =>
        exec(rfc850DatePattern, value) match
          case Some(date) =>
            val now = js.Date.now()
            val today = Math.floor(now / MsPerDay).toInt
            val (currentYear, currentMonth, currentDay) = civilFromDays(today)
            val currentCenturyYear = Math.floorDiv(currentYear, 100) * 100 + date(3).toInt
            val fiftyYearsFromNow =
              daysFromCivil(currentYear + 50, currentMonth, currentDay) * MsPerDay + (now - today * MsPerDay)
            // RFC 9110 maps two-digit years to the latest matching century that is not more than 50
            // years in the future.
            List(currentCenturyYear - 100, currentCenturyYear, currentCenturyYear + 100).foldLeft(Option.empty[Double]) {
              (timestamp, year) =>
                createTimestamp(year, date(2), date(1), date(4), date(5), date(6))
                  .filter(_ <= fiftyYearsFromNow)
                  .orElse(timestamp)
            }
          case None =>
            exec(asctimeDatePattern, value).flatMap { date =>
              val Array(hours, minutes, seconds) = date(4).split(":")
              createTimestamp(date(5).toInt, date(2), date(3).trim, hours, minutes, seconds)
            }

  def calculateRetryTimingDelay(header: TimingHeader): Option[Double] =
    if delayPattern.test(header.value) then
      val seconds = header.value.toDouble
      // Standard Retry-After numbers are delay seconds. Timestamp compatibility only applies to
      // reset headers.
      val delay =
        if header.allowTimestamp && seconds >= TimestampThresholdSeconds then seconds * 1000 - js.Date.now()
        else seconds * 1000
      Some(js.Math.max(0, delay))
    else
      // createTimestamp only returns valid dates, so the delay is always finite.
      parseDate(header.value).map(timestamp => js.Math.max(0, timestamp - js.Date.now()))
