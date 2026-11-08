open Bindings

let getDateTimeLocale = (): any => {
  let intl = get(window(), "Intl")
  if isFalsy(intl) {
    identity(-1)
  } else {
    let dateTimeFormat = get(intl, "DateTimeFormat")
    if isFalsy(dateTimeFormat) {
      identity(-2)
    } else {
      let locale = get(resolvedOptions(construct0(dateTimeFormat)), "locale")
      if isFalsy(locale) && !strictEq(locale, identity("")) {
        identity(-3)
      } else {
        locale
      }
    }
  }
}
