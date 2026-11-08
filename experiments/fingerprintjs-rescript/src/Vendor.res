open Bindings
let getVendor = () => orElse(get(navigator(), "vendor"), identity(""))
