import Big from '../../node_modules/big.js/big.mjs'

export function run(input) {
  const Decimal = Big()
  const outputs = []
  for (let i = 0; i < (input.repeat ?? 1); i++) {
    for (const c of input.cases) {
      Decimal.DP = c.dp ?? 20
      Decimal.RM = c.rm ?? 1
      try {
        const x = new Decimal(c.x)
        let result
        switch (c.op) {
          case 'toString': case 'sqrt': result = x[c.op](); break
          case 'plus': case 'minus': case 'times': case 'div': case 'cmp': result = x[c.op](c.y); break
          case 'pow': result = x.pow(c.n); break
          case 'round': case 'toFixed': case 'toExponential': case 'toPrecision': result = x[c.op](c.n, c.mode); break
          default: throw Error('unknown decimal operation')
        }
        outputs.push(String(result))
      } catch (error) {
        outputs.push(error.message)
      }
    }
  }
  return outputs.join('\n')
}
