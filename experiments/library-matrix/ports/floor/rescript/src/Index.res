// Floor port (ReScript): the record type is the JS object, so the string passes through unchanged.
type input = {text: string}

let run = (input: input): string => input.text
