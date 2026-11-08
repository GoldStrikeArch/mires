open Bindings
open Data
let getDeviceMemory = () => replaceNaN(toFloat(get(navigator(), "deviceMemory")), jsUndefined)
