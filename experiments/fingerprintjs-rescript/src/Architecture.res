open Bindings

let getArchitecture = () => {
  let f = newFloat32Array(1)
  let u8 = newUint8ArrayFromBuffer(arrayBuffer(f))
  jsSetIndex(f, 0, jsInfinity)
  jsSetIndex(f, 0, asFloat(jsIndex(f, 0)) -. asFloat(jsIndex(f, 0)))
  jsIndex(u8, 3)
}
