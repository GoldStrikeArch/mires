// This adapter uses the published ESM entry so all engines run the same release.
import {
  applyPatch, createTwoFilesPatch, diffArrays, diffChars, diffJson, diffLines,
  diffWords, parsePatch, reversePatch, structuredPatch,
} from '../../node_modules/diff/libesm/index.js'

export function run(input) {
  const { op, before, after, patch, oldFileName, newFileName, oldHeader, newHeader } = input
  const options = { ...input.options }
  switch (op) {
    case 'diffChars': return diffChars(before, after, options)
    case 'diffWords': return diffWords(before, after, options)
    case 'diffLines': return diffLines(before, after, options)
    case 'diffJson': return diffJson(before, after, options)
    case 'diffArrays': return diffArrays(before, after, options)
    case 'structuredPatch': return structuredPatch(oldFileName, newFileName, before, after, oldHeader, newHeader, options)
    case 'createTwoFilesPatch': return createTwoFilesPatch(oldFileName, newFileName, before, after, oldHeader, newHeader, options)
    case 'parsePatch': return parsePatch(patch)
    case 'applyPatch': return applyPatch(before, patch, options)
    case 'reversePatch': return reversePatch(patch)
    default: throw new TypeError('unknown diff operation')
  }
}
