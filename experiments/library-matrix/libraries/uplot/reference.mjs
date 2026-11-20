import uPlot from '../../node_modules/uplot/dist/uPlot.esm.js'

// Record the exact vertices passed to the browser and still let the real Path2D render them.
function geometry(chart, index, traces) {
  const NativePath = globalThis.Path2D
  class RecordedPath extends NativePath {
    vertices = []
    lineTo(x, y) {
      this.vertices.push([x, y])
      super.lineTo(x, y)
    }
  }
  try {
    globalThis.Path2D = RecordedPath
    const path = uPlot.paths.linear()(chart, index, 0, chart.data[0].length - 1)
    traces[index - 1] = [path.stroke.vertices, path.gaps]
    return path
  } finally {
    globalThis.Path2D = NativePath
  }
}

const precise = x => Math.round(x * 1e6) || 0

export function run(input) {
  let frame = input.frames[0]
  const host = document.createElement('div')
  document.body.appendChild(host)
  const traces = []
  let chart
  try {
    chart = new uPlot({
      width: frame.size[0] / uPlot.pxRatio, height: frame.size[1] / uPlot.pxRatio,
      padding: [0, 0, 0, 0], axes: [{show:false}, {show:false}],
      cursor: {show:false}, legend: {show:false}, select: {show:false},
      scales: {x: {time:false, range:() => frame.xRange}, y: {range:() => frame.yRange}},
      series: [{}, ...frame.data.slice(1).map(() => ({
        stroke: '#172839', width: 1, points: {show:false}, spanGaps: input.spanGaps,
        paths: (u, index) => geometry(u, index, traces)
      }))]
    }, frame.data, host)
    return input.frames.map(next => {
      frame = next
      traces.length = 0
      chart.batch(() => {
        chart.setSize({width:frame.size[0] / uPlot.pxRatio, height:frame.size[1] / uPlot.pxRatio})
        chart.setData(frame.data)
        for (const axis of ['x','y']) {
          const [min,max] = frame[axis+'Range']
          chart.setScale(axis, {min,max})
        }
      })
      const paths = frame.data.slice(1).map((_, i) => traces[i] ?? [[], input.spanGaps ? null : []])
      const probes = frame.probes.map(([x,y,px,py]) => [
        precise(chart.valToPos(x,'x',true)), precise(chart.valToPos(y,'y',true)),
        precise(chart.posToVal(px,'x',true)), precise(chart.posToVal(py,'y',true)),
        chart.valToIdx(x), chart.posToIdx(px,true)
      ])
      return [chart.ctx.canvas.width, chart.ctx.canvas.height, paths, probes]
    })
  } finally {
    chart?.destroy()
    host.remove()
  }
}
