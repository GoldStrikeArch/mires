// matrix-floor 1.0.0: the identity program, a handwritten stand-in package. Its ports measure each
// language's runtime floor the same way for every family: the production artifact of a program
// whose only work is the boundary (a JS string in, the same string out). See ../CONTRACT.md.
export function identity(text) {
  return text
}
