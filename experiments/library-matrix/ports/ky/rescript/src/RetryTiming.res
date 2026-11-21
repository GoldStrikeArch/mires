// Ky 2.2.0 core/retry-timing.js: which header gives a server-provided retry delay, and that delay.

type header = {value: string, allowTimestamp: bool}

// A reset value is only read as an epoch from this second onwards (2001-09-09).
let timestampThresholdSeconds = 1_000_000_000.
let delayPattern = /^\d+$/
let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
// HTTP-date parsing is limited to IMF-fixdate and the two obsolete HTTP-date formats.
let imfDatePattern = /^(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun), (\d{2}) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{4}) (\d{2}):(\d{2}):(\d{2}) GMT$/
let rfc850DatePattern = /^(?:Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday), (\d{2})-(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)-(\d{2}) (\d{2}):(\d{2}):(\d{2}) GMT$/
let asctimeDatePattern = /^(Mon|Tue|Wed|Thu|Fri|Sat|Sun) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{2}| \d) (\d{2}:\d{2}:\d{2}) (\d{4})$/

let getRetryTimingHeader = (headers: Host.Headers.t): option<header> => {
  let get = name => headers->Host.Headers.get(name)
  // Retry-After is authoritative when present. Only missing Retry-After falls through to rate-limit headers.
  switch get("Retry-After") {
  | Some(value) => Some({value, allowTimestamp: false})
  | None =>
    switch get("RateLimit-Reset") {
    | Some(value) => Some({value, allowTimestamp: true})
    | None =>
      switch get("X-RateLimit-Retry-After") {
      | Some(value) => Some({value, allowTimestamp: false})
      | None =>
        switch get("X-RateLimit-Reset") {
        | Some(value) => Some({value, allowTimestamp: true})
        | None => get("X-Rate-Limit-Reset")->Option.map(value => {value, allowTimestamp: true})
        }
      }
    }
  }
}

let createTimestamp = (~year, ~month, ~day, ~hours, ~minutes, ~seconds) => {
  let monthIndex = months->Array.indexOf(month)
  let dayNumber = Host.toNumber(day)
  let hoursNumber = Host.toNumber(hours)
  let minutesNumber = Host.toNumber(minutes)
  let secondsNumber = Host.toNumber(seconds)
  if monthIndex == -1 || hoursNumber > 23. || minutesNumber > 59. || secondsNumber > 60. {
    None
  } else {
    let normalizedSeconds = Math.min(secondsNumber, 59.)
    let date = Date.fromTime(
      Host.utc(year, Int.toFloat(monthIndex), dayNumber, hoursNumber, minutesNumber, normalizedSeconds),
    )
    date->Date.setUTCFullYear(Float.toInt(year))
    let timestamp = date->Date.getTime
    // Date.UTC normalizes overflow, so round-trip the parts to reject invalid dates.
    if (
      Int.toFloat(date->Date.getUTCFullYear) != year ||
      date->Date.getUTCMonth != monthIndex ||
      Int.toFloat(date->Date.getUTCDate) != dayNumber ||
      Int.toFloat(date->Date.getUTCHours) != hoursNumber ||
      Int.toFloat(date->Date.getUTCMinutes) != minutesNumber ||
      Int.toFloat(date->Date.getUTCSeconds) != normalizedSeconds
    ) {
      None
    } else {
      // A leap second is represented by parsing 60 as 59 and adding one second.
      Some(secondsNumber == 60. ? timestamp +. 1000. : timestamp)
    }
  }
}

let capture = (result, index) =>
  result->RegExp.Result.matches->Array.getUnsafe(index - 1)->Option.getOr("")

let parseDate = value =>
  switch imfDatePattern->RegExp.exec(value) {
  | Some(imfDate) =>
    createTimestamp(
      ~year=Host.toNumber(capture(imfDate, 3)),
      ~month=capture(imfDate, 2),
      ~day=capture(imfDate, 1),
      ~hours=capture(imfDate, 4),
      ~minutes=capture(imfDate, 5),
      ~seconds=capture(imfDate, 6),
    )
  | None =>
    switch rfc850DatePattern->RegExp.exec(value) {
    | Some(rfc850Date) =>
      let now = Date.make()
      let twoDigitYear = Host.toNumber(capture(rfc850Date, 3))
      let currentYear = Int.toFloat(now->Date.getUTCFullYear)
      let currentCenturyYear = Math.floor(currentYear /. 100.) *. 100. +. twoDigitYear
      let fiftyYearsFromNow = Host.utcMilliseconds(
        currentYear +. 50.,
        Int.toFloat(now->Date.getUTCMonth),
        Int.toFloat(now->Date.getUTCDate),
        Int.toFloat(now->Date.getUTCHours),
        Int.toFloat(now->Date.getUTCMinutes),
        Int.toFloat(now->Date.getUTCSeconds),
        Int.toFloat(now->Date.getUTCMilliseconds),
      )
      // RFC 9110 maps two-digit years to the latest matching century that is not more than 50 years in the future.
      [currentCenturyYear -. 100., currentCenturyYear, currentCenturyYear +. 100.]->Array.reduce(
        None,
        (timestamp, year) =>
          switch createTimestamp(
            ~year,
            ~month=capture(rfc850Date, 2),
            ~day=capture(rfc850Date, 1),
            ~hours=capture(rfc850Date, 4),
            ~minutes=capture(rfc850Date, 5),
            ~seconds=capture(rfc850Date, 6),
          ) {
          | Some(candidate) if candidate <= fiftyYearsFromNow => Some(candidate)
          | _ => timestamp
          },
      )
    | None =>
      asctimeDatePattern
      ->RegExp.exec(value)
      ->Option.flatMap(asctimeDate => {
        let time = capture(asctimeDate, 4)->String.split(":")
        createTimestamp(
          ~year=Host.toNumber(capture(asctimeDate, 5)),
          ~month=capture(asctimeDate, 2),
          ~day=capture(asctimeDate, 3)->String.trim,
          ~hours=time->Array.getUnsafe(0),
          ~minutes=time->Array.getUnsafe(1),
          ~seconds=time->Array.getUnsafe(2),
        )
      })
    }
  }

let calculateRetryTimingDelay = ({value, allowTimestamp}) =>
  if delayPattern->RegExp.test(value) {
    let delay = Host.toNumber(value) *. 1000.
    // Standard Retry-After numbers are delay seconds. Timestamp compatibility only applies to reset headers.
    let delay =
      allowTimestamp && Host.toNumber(value) >= timestampThresholdSeconds ? delay -. Date.now() : delay
    Some(Math.max(0., delay))
  } else {
    parseDate(value)->Option.map(timestamp => Math.max(0., timestamp -. Date.now()))
  }
