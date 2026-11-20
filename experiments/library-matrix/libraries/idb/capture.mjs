// This runs on the test origin. All calls share the same browser and module instance.
export async function capture({moduleUrl,cases}) {
  const {run} = await import(moduleUrl)
  const databaseNames = async () => (await indexedDB.databases()).map(x=>x.name).sort()
  const before = await databaseNames()
  const pairs = await Promise.all(cases.map(async ({input}) => {
    const [a,b] = await Promise.all([run(structuredClone(input)),run(structuredClone(input))])
    if (JSON.stringify(a) !== JSON.stringify(b)) throw Error('concurrent calls differed')
    return a
  }))
  return {pairs,before,after:await databaseNames()}
}
