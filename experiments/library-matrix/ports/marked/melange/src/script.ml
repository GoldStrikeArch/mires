(* Corpus entry point for marked@18.1.1: decodes { op, markdown, options? } and runs
   Marked#parse or Marked#parseInline synchronously, returning the rendered HTML JS string. *)

open Host
open Tokens

let read_options (input : Value.t) : options =
  match Value.field input "options" with
  | None -> { gfm = true; breaks = false; pedantic = false }
  | Some obj ->
      let gfm = match Value.field obj "gfm" with Some v -> Value.as_bool v | None -> true in
      let breaks = match Value.field obj "breaks" with Some v -> Value.as_bool v | None -> false in
      let pedantic =
        match Value.field obj "pedantic" with Some v -> Value.as_bool v | None -> false
      in
      { gfm; breaks; pedantic }

let run (input : Value.t) : Jstr.t =
  let op =
    match Value.field input "op" with
    | Some v -> Value.as_string_name v
    | None -> invalid_arg "marked port: missing op"
  in
  let markdown =
    match Value.field input "markdown" with
    | Some v -> Value.as_jstr v
    | None -> invalid_arg "marked port: missing markdown"
  in
  let options = read_options input in
  match op with
  | "parse" -> Parser.parse (Lexer.lex markdown options)
  | "parseInline" -> Parser.parse_inline (Lexer.lex_inline markdown options)
  | other -> invalid_arg ("unknown operation " ^ other)
