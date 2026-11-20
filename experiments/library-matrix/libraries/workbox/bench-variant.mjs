export function variant(input,k) {
  const suffix = k.toString(36).padStart(11,'0')
  return {steps:input.steps.map(step => step.path?.startsWith('first/') ? {...step,path:step.path+'-'+suffix} : step)}
}
