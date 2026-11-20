export async function capture({moduleUrl, oracleUrl, inputsJSON}) {
  const inputs = JSON.parse(inputsJSON)
  const target = await import(moduleUrl)
  const oracle = await import(oracleUrl)
  const databases = async () => (await indexedDB.databases()).map(db=>db.name).sort()
  const before = {databases:await databases(), elements:document.body.childElementCount}
  const expected = []
  for (const input of inputs) expected.push(await oracle.run(structuredClone(input)))
  // All calls share an origin, including their keys, and each uses different document bodies.
  const results = await Promise.all(inputs.map(input=>target.run(structuredClone(input))))
  const after = {databases:await databases(), elements:document.body.childElementCount}
  return {before, after, expected, results}
}
