open Bindings

let getPrivateClickMeasurement = () => {
  let link = createElement(document(), "a")
  let sourceId = coalesce(get(link, "attributionSourceId"), get(link, "attributionsourceid"))
  if isUndefined(sourceId) {
    jsUndefined
  } else {
    identity(jsString(sourceId))
  }
}
