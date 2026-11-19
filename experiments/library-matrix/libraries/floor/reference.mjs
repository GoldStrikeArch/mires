// Reference adapter for the floor library (see ../pilot/reference.mjs for the adapter pattern).
import { identity } from './package/index.mjs'

export function run(input) {
  return identity(input.text)
}
