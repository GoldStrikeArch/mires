export function wrapElm(source) {
  const patched = source.replace(
    'var init = _Process_sleep(0);',
    'var init = _Scheduler_succeed(null);',
  )
  if (patched === source) {
    throw new Error('could not patch Elm outgoing-port sleep')
  }
  const rewritten = patched.replace(/}\s*\(\s*this\s*\)\s*\)\s*;?\s*$/, '}(scope));')
  if (rewritten === patched) {
    throw new Error('could not rewrite Elm IIFE this-binding')
  }
  return `const scope = Object.create(null);\n${rewritten}\nexport default scope.Elm;\n`
}
