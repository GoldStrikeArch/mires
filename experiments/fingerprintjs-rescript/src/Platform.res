open Bindings
open Browser

let getPlatform = (): any => {
  let platform = get(navigator(), "platform")
  if asString(platform) == "MacIntel" {
    if isWebKit() && !isDesktopWebKit() {
      if isIPad() {
        identity("iPad")
      } else {
        identity("iPhone")
      }
    } else {
      platform
    }
  } else {
    platform
  }
}
