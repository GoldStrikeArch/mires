open Bindings
open Data

let currentTimezoneOffset = () => {
  let currentYear = getYear(newDate())
  mathMax(
    toFloat(Bindings.getTimezoneOffset(newDateYMD(currentYear, 0, 1))),
    toFloat(Bindings.getTimezoneOffset(newDateYMD(currentYear, 6, 1))),
  )
}

let getTimezone = (): any => {
  let dateTimeFormat = get(get(window(), "Intl"), "DateTimeFormat")
  if isTruthy(dateTimeFormat) {
    let timezone = get(resolvedOptions(construct0(dateTimeFormat)), "timeZone")
    if isTruthy(timezone) {
      timezone
    } else {
      let offset = -.currentTimezoneOffset()
      identity(
        "UTC" ++ if offset >= 0. {
          "+"
        } else {
          ""
        } ++ jsString(offset),
      )
    }
  } else {
    let offset = -.currentTimezoneOffset()
    identity(
      "UTC" ++ if offset >= 0. {
        "+"
      } else {
        ""
      } ++ jsString(offset),
    )
  }
}
