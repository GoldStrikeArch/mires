open Bindings
let getOpenDatabase = () => isTruthy(get(window(), "openDatabase"))
