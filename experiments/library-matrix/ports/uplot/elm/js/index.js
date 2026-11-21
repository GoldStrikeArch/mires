import Elm from 'elm-program'

let app
let response

// The chart, computed by Elm: each frame's canvas size, stroke settings, series geometry and clip
// rectangles, and probes.
function chart(request) {
  if (!app) {
    app = Elm.Main.init()
    app.ports.response.subscribe(value => { response = value })
  }
  response = undefined
  app.ports.request.send(request)
  if (response === undefined) throw new Error('the Elm worker did not respond synchronously')
  if (response.error) throw new Error(response.error)
  return response.ok
}

// Applies Elm's settings for one series: the canvas calls of uPlot's drawPath.
function paint(ctx, settings, series) {
  const path = new Path2D()
  for (const [x, y] of series.geometry[0]) path.lineTo(x, y)
  const bounds = new Path2D()
  bounds.rect(...settings.bounds)
  if (settings.translate !== null) ctx.translate(settings.translate, settings.translate)
  ctx.strokeStyle = settings.strokeStyle
  ctx.lineWidth = settings.lineWidth
  ctx.lineJoin = settings.lineJoin
  ctx.lineCap = settings.lineCap
  ctx.save()
  ctx.clip(bounds)
  if (series.clip !== null) {
    const clip = new Path2D()
    for (const rect of series.clip) clip.rect(...rect)
    ctx.clip(clip)
  }
  if (settings.stroke) ctx.stroke(path)
  ctx.restore()
  if (settings.translate !== null) ctx.translate(-settings.translate, -settings.translate)
}

export function run(input) {
  const canvas = document.createElement('canvas')
  document.body.appendChild(canvas)
  try {
    const ctx = canvas.getContext('2d')
    return chart({ input, pxRatio: devicePixelRatio }).map(frame => {
      [canvas.width, canvas.height] = frame.canvas
      if (frame.paint !== null) for (const series of frame.series) paint(ctx, frame.paint, series)
      return [canvas.width, canvas.height, frame.series.map(series => series.geometry), frame.probes]
    })
  } finally {
    canvas.remove()
  }
}
