// The first point sets the width. An absent ordinate is pushed as JS undefined.
let flatten = rings => {
  if Array.length(rings) == 0 || Array.length(Array.getUnsafe(rings, 0)) == 0 {
    JsError.TypeError.make("missing first vertex")->JsError.throw
  }
  let dimensions = rings->Array.getUnsafe(0)->Array.getUnsafe(0)->Array.length
  let vertices = []
  let holes = []
  let prevLen = ref(0)
  let holeIndex = ref(0)
  rings->Array.forEach(ring => {
    ring->Array.forEach(point => {
      for d in 0 to dimensions - 1 {
        vertices->Array.push(point[d])->ignore
      }
    })
    if prevLen.contents != 0 {
      holeIndex := holeIndex.contents + prevLen.contents
      holes->Array.push(holeIndex.contents)->ignore
    }
    prevLen := Array.length(ring)
  })
  (vertices, holes, dimensions)
}
