// TextRenderer for marked@18.1.1 (ported faithfully from `src/TextRenderer.ts`).
// Flattens inline tokens to plain text (used for image `alt` attributes).

open Tokens

let renderInline = (tokens: array<inlineToken>): string => {
  let out = ref("")
  let len = tokens->Array.length
  for i in 0 to len - 1 {
    let chunk = switch tokens->Array.getUnsafe(i) {
    | Strong({text})
    | Em({text})
    | Codespan({text})
    | Del({text})
    | Html({text})
    | Text({text})
    | Escape({text})
    | Link({text})
    | Image({text}) => text
    | Br(_) => ""
    | Checkbox({raw}) => raw
    }
    out := out.contents ++ chunk
  }
  out.contents
}
