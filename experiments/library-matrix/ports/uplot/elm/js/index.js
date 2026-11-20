import Elm from 'elm-program'

let app
let response

function geometry(input) {
  if (!app) {
    app = Elm.Main.init()
    app.ports.response.subscribe(value => { response = value })
  }
  response = undefined
  app.ports.request.send(input)
  if (response === undefined) throw new Error('the Elm worker did not respond synchronously')
  if (response.error) throw new Error(response.error)
  return response.ok
}

function stroke(ctx, series, w, h) {
  const path = new Path2D()
  for (const [x, y] of series.geometry[0]) path.lineTo(x, y)
  ctx.save()
  try {
    const width = Math.round(devicePixelRatio * 1000) / 1000
    const offset = (width % 2) / 2
    ctx.lineWidth = width
    ctx.lineJoin = 'round'
    ctx.lineCap = 'butt'
    ctx.translate(offset, offset)
    const bounds = new Path2D()
    bounds.rect(-width / 2, -width / 2, w + width, h + width)
    ctx.clip(bounds)
    if (series.mask) {
      const mask = new Path2D()
      for (const rect of series.mask) mask.rect(...rect)
      ctx.clip(mask)
    }
    ctx.stroke(path)
  } finally {
    ctx.restore()
  }
  return series.geometry
}

export function run(input) {
  const canvas = document.createElement('canvas')
  document.body.appendChild(canvas)
  try {
    const ctx = canvas.getContext('2d')
    return geometry(input).map(({size: [w, h], bounds, series, probes}) => {
      canvas.width = Math.round(w)
      canvas.height = Math.round(h)
      ctx.strokeStyle = '#172839'
      const paths = series.map(s => stroke(ctx, s, ...bounds))
      return [canvas.width, canvas.height, paths, probes]
    })
  } finally {
    canvas.remove()
  }
}
