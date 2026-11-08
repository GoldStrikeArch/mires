open Bindings
open Browser

let getAudioContextBaseLatency = (): any => {
  let isAllowedPlatform = isAndroid() || isWebKit()
  if !isAllowedPlatform {
    identity(-2)
  } else if isFalsy(get(window(), "AudioContext")) {
    identity(-1)
  } else {
    let latency = get(construct0(get(window(), "AudioContext")), "baseLatency")
    if isNullish(latency) {
      identity(-1)
    } else if !isFinite(latency) {
      identity(-3)
    } else {
      latency
    }
  }
}
