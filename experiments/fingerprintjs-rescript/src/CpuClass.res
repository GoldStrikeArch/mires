open Bindings

let isSourceLoaded = emptyObject()
set(isSourceLoaded, "x", false)

let getCpuClass = () => {
  set(isSourceLoaded, "x", true)
  get(navigator(), "cpuClass")
}
