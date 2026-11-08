open Bindings

let getPlugins = () => {
  let rawPlugins = get(navigator(), "plugins")
  if isFalsy(rawPlugins) {
    jsUndefined
  } else {
    let plugins = []
    let rec loop = i =>
      if i >= arrayLength(asArray(rawPlugins)) {
        ()
      } else {
        let plugin = jsIndex(rawPlugins, i)
        if isTruthy(plugin) {
          let mimeTypes = []
          let rec mimeLoop = j =>
            if j >= arrayLength(asArray(plugin)) {
              ()
            } else {
              let mimeType = jsIndex(plugin, j)
              let item = emptyObject()
              set(item, "type", get(mimeType, "type"))
              set(item, "suffixes", get(mimeType, "suffixes"))
              arrayPush(mimeTypes, item)->ignore
              mimeLoop(j + 1)
            }
          mimeLoop(0)
          let item = emptyObject()
          set(item, "name", get(plugin, "name"))
          set(item, "description", get(plugin, "description"))
          set(item, "mimeTypes", mimeTypes)
          arrayPush(plugins, item)->ignore
        }
        loop(i + 1)
      }
    loop(0)
    identity(plugins)
  }
}
