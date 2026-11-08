import { elmCall } from '../elm_bridge.js'

export function getMathFingerprint() {
  const M = Math
  const fallbackFn = () => 0
  const acos = M.acos || fallbackFn
  const acosh = M.acosh || fallbackFn
  const asin = M.asin || fallbackFn
  const asinh = M.asinh || fallbackFn
  const atanh = M.atanh || fallbackFn
  const atan = M.atan || fallbackFn
  const sin = M.sin || fallbackFn
  const sinh = M.sinh || fallbackFn
  const cos = M.cos || fallbackFn
  const cosh = M.cosh || fallbackFn
  const tan = M.tan || fallbackFn
  const tanh = M.tanh || fallbackFn
  const exp = M.exp || fallbackFn
  const expm1 = M.expm1 || fallbackFn
  const log1p = M.log1p || fallbackFn

  return elmCall({
    op: 'math',
    acos: acos(0.123124234234234242),
    acosh: acosh(1e308),
    asin: asin(0.123124234234234242),
    asinh: asinh(1),
    atanh: atanh(0.5),
    atan: atan(0.5),
    sin: sin(-1e300),
    sinh: sinh(1),
    cos: cos(10.000000000123),
    cosh: cosh(1),
    tan: tan(-1e300),
    tanh: tanh(1),
    exp: exp(1),
    expm1: expm1(1),
    log1p: log1p(10),
    exp1: M.exp(1),
    exp2: M.exp(2),
  })
}
