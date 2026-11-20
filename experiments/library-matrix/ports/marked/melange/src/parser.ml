(* Exact port of marked@18.1.1 src/Parser.ts. *)

open Host
open Tokens

let parse_inline_text (tokens : token Vec.t) : Jstr.t =
  let n = Vec.length tokens in
  if n = 0 then Jstr.empty
  else if n = 1 then Renderer.Text_renderer.render_token (Vec.get tokens 0)
  else
    let out = Vec.create () in
    for i = 0 to n - 1 do
      let chunk = Renderer.Text_renderer.render_token (Vec.get tokens i) in
      if Jstr.length chunk > 0 then Vec.push out chunk
    done;
    Jstr.join out Jstr.empty

let rec render_block (tok : token) : Jstr.t =
  match tok with
  | Space _ -> Renderer.space ()
  | Hr _ -> Renderer.hr ()
  | Heading h -> Renderer.heading ~parse_inline ~depth:h.depth h.tokens
  | Code c -> Renderer.code ~text:c.text ~lang:c.lang ~escaped:c.escaped
  | Table t -> Renderer.table ~parse_inline ~header:t.header ~rows:t.rows
  | Blockquote b -> Renderer.blockquote ~parse b.tokens
  | List l -> Renderer.list ~parse ~ordered:l.ordered ~start:l.start l.items
  | Checkbox c -> Renderer.checkbox c.checked
  | Html h -> Renderer.html h.text
  | Def _ -> Renderer.def ()
  | Text t when t.t_is_paragraph -> (
      match t.t_tokens with
      | Some ts -> Renderer.paragraph ~parse_inline ts
      | None -> Renderer.paragraph ~parse_inline (Vec.create ()))
  | Text t -> Renderer.text ~parse_inline t
  | _ -> invalid_arg "marked port: unexpected inline token in block parse"

and parse (tokens : token Vec.t) : Jstr.t =
  let n = Vec.length tokens in
  if n = 0 then Jstr.empty
  else if n = 1 then render_block (Vec.get tokens 0)
  else
    let out = Vec.create () in
    for i = 0 to n - 1 do
      let chunk = render_block (Vec.get tokens i) in
      if Jstr.length chunk > 0 then Vec.push out chunk
    done;
    Jstr.join out Jstr.empty

and render_inline (tok : token) : Jstr.t =
  match tok with
  | Escape e -> Renderer.escape_text e.text
  | Html h -> Renderer.html h.text
  | Link l ->
      Renderer.link ~parse_inline ~href:l.href ~title:l.title ~text:l.text ~tokens:l.tokens
        ~autolink:l.autolink
  | Image img ->
      Renderer.image ~parse_inline_text ~href:img.href ~title:img.title ~text:img.text
        ~tokens:img.tokens
  | Checkbox c -> Renderer.checkbox c.checked
  | Strong s -> Renderer.strong ~parse_inline s.tokens
  | Em e -> Renderer.em ~parse_inline e.tokens
  | Codespan c -> Renderer.codespan c.text
  | Br _ -> Renderer.br ()
  | Del d -> Renderer.del ~parse_inline d.tokens
  | Text t -> Renderer.text ~parse_inline t
  | _ -> invalid_arg "marked port: unexpected block token in parseInline"

and parse_inline (tokens : token Vec.t) : Jstr.t =
  let n = Vec.length tokens in
  if n = 0 then Jstr.empty
  else if n = 1 then render_inline (Vec.get tokens 0)
  else
    let out = Vec.create () in
    for i = 0 to n - 1 do
      let chunk = render_inline (Vec.get tokens i) in
      if Jstr.length chunk > 0 then Vec.push out chunk
    done;
    Jstr.join out Jstr.empty
