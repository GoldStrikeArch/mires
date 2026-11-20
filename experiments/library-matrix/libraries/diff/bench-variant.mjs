export function variant(input, k) {
  const suffix = ' /* rev ' + k.toString(36).padStart(11, '0') + ' */\n'
  return { ...input, before: input.before.replaceAll('\n', suffix), after: input.after.replaceAll('\n', suffix) }
}
