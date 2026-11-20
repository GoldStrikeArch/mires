export function variant(input,k) {
  const token = k.toString(36).padStart(11,'0')
  return {cases:input.cases.map(c=>({...c,url:c.url+'?v='+token}))}
}
