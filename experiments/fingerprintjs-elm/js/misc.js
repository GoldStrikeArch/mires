export function errorToObject(error) {
  return {
    name: error.name,
    message: error.message,
    stack: error.stack?.split('\n'),
    ...error,
  }
}

export function isFunctionNative(func) {
  return /^function\s.*?\{\s*\[native code]\s*}$/.test(String(func))
}
