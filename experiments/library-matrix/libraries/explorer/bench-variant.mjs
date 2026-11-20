import {variant as varyChart} from '../uplot/bench-variant.mjs'

export function variant(input, k) {
  return {
    ...input,
    chart: varyChart(input.chart, k),
    records: input.records.map((doc, i) => ({
      ...doc,
      id: doc.id + '-' + k,
      body: doc.body + '\n\nRevision ' + (i + k)
    }))
  }
}
