export async function capture({ moduleUrl, cases }) {
  const { run } = await import(moduleUrl)
  const before = (await caches.keys()).sort()
  const results = {}
  for (const entry of cases) {
    try { results[entry.id] = await run(entry.input) }
    catch (e) { results[entry.id] = {error:String(e?.message ?? e)} }
  }
  return {results, before, after:(await caches.keys()).sort()}
}
