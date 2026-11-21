// This runs on the test origin. All calls share the same browser and module instance.
// Cases arrive as JSON text (WebDriver arguments cannot nest deeply). Each case runs twice at once;
// a call that rejects is recorded by its error name.
export async function capture({moduleUrl,casesJSON}) {
  const {run} = await import(moduleUrl)
  const cases = JSON.parse(casesJSON)
  const databaseNames = async () => (await indexedDB.databases()).map(x=>x.name).sort()
  const settle = input => run(structuredClone(input)).then(value=>({value}),error=>({threw:String(error?.name)}))
  const before = await databaseNames()
  const pairs = await Promise.all(cases.map(async ({id,input}) => {
    const [a,b] = (await Promise.all([settle(input),settle(input)])).map(outcome=>JSON.stringify(outcome))
    if (a !== b) throw Error(id+': concurrent calls differed')
    return a
  }))
  return {pairs,before,after:await databaseNames()}
}
