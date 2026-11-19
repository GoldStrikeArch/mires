// Entry module of the ReScript port. The boundary: the corpus input is typed as a record whose
// runtime representation is the JS object itself (ReScript records compile to plain objects), so
// `text` is a JS string and `top` a number with no conversion; the result is a JS string.

type input = {text: string, top: int}

let run = (input: input): string => Pilot.analyse(input.text, input.top)
