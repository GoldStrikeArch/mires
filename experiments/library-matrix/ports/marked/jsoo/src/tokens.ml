(* Token, option, and lexer-context definitions for marked@18.1.1. *)

open Host

type options = {
  gfm : bool;
  breaks : bool;
  pedantic : bool;
}

type link_def = {
  link_href : Jstr.t;
  link_title : Jstr.t option;
}

type align =
  | Align_none
  | Align_left
  | Align_center
  | Align_right

type text_token = {
  mutable t_raw : Jstr.t;
  mutable t_text : Jstr.t;
  mutable t_escaped : bool;
  mutable t_is_paragraph : bool;
  t_tokens : token Vec.t option;
}

and table_cell = {
  cell_text : Jstr.t;
  cell_tokens : token Vec.t;
  cell_header : bool;
  cell_align : align;
}

and list_item = {
  mutable item_raw : Jstr.t;
  mutable item_task : bool;
  mutable item_checked : bool;
  mutable item_loose : bool;
  mutable item_text : Jstr.t;
  mutable item_tokens : token Vec.t;
}

and token =
  | Space of { mutable raw : Jstr.t }
  | Hr of { mutable raw : Jstr.t }
  | Heading of {
      mutable raw : Jstr.t;
      depth : int;
      text : Jstr.t;
      tokens : token Vec.t;
    }
  | Code of {
      mutable raw : Jstr.t;
      lang : Jstr.t option;
      text : Jstr.t;
      escaped : bool;
    }
  | Table of {
      mutable raw : Jstr.t;
      header : table_cell Vec.t;
      align : align Vec.t;
      rows : table_cell Vec.t Vec.t;
    }
  | Blockquote of {
      mutable raw : Jstr.t;
      text : Jstr.t;
      tokens : token Vec.t;
    }
  | List of {
      mutable raw : Jstr.t;
      ordered : bool;
      start : int;
      mutable loose : bool;
      items : list_item Vec.t;
    }
  | Checkbox of {
      mutable raw : Jstr.t;
      checked : bool;
    }
  | Html of {
      mutable raw : Jstr.t;
      block : bool;
      text : Jstr.t;
    }
  | Def of {
      mutable raw : Jstr.t;
      tag : Jstr.t;
      href : Jstr.t;
      title : Jstr.t option;
    }
  | Text of text_token
  | Escape of {
      raw : Jstr.t;
      text : Jstr.t;
    }
  | Link of {
      raw : Jstr.t;
      href : Jstr.t;
      title : Jstr.t option;
      text : Jstr.t;
      autolink : bool;
      tokens : token Vec.t;
    }
  | Image of {
      raw : Jstr.t;
      href : Jstr.t;
      title : Jstr.t option;
      text : Jstr.t;
      tokens : token Vec.t option;
    }
  | Strong of {
      raw : Jstr.t;
      text : Jstr.t;
      tokens : token Vec.t;
    }
  | Em of {
      raw : Jstr.t;
      text : Jstr.t;
      tokens : token Vec.t;
    }
  | Codespan of {
      raw : Jstr.t;
      text : Jstr.t;
    }
  | Br of {
      raw : Jstr.t;
    }
  | Del of {
      raw : Jstr.t;
      text : Jstr.t;
      tokens : token Vec.t;
    }

type inline_queue_entry = {
  mutable iq_src : Jstr.t;
  iq_tokens : token Vec.t;
}

type lexer_state = {
  mutable in_link : bool;
  mutable in_raw_block : bool;
  mutable link_emitted : bool;
  mutable link_paren_possible : bool;
  mutable top : bool;
}

type lexer = {
  options : options;
  rules : Rules.rules;
  state : lexer_state;
  tokens : token Vec.t;
  links : link_def StrMap.t;
  inline_queue : inline_queue_entry Vec.t;
  mutable block_tokens_fn : Jstr.t -> token Vec.t -> bool -> token Vec.t;
  mutable inline_tokens_fn : Jstr.t -> token Vec.t -> token Vec.t;
}

let s_nl = Jstr.of_string "\n"

let token_raw (tok : token) : Jstr.t =
  match tok with
  | Space r -> r.raw
  | Hr r -> r.raw
  | Heading r -> r.raw
  | Code r -> r.raw
  | Table r -> r.raw
  | Blockquote r -> r.raw
  | List r -> r.raw
  | Checkbox r -> r.raw
  | Html r -> r.raw
  | Def r -> r.raw
  | Text t -> t.t_raw
  | Escape r -> r.raw
  | Link r -> r.raw
  | Image r -> r.raw
  | Strong r -> r.raw
  | Em r -> r.raw
  | Codespan r -> r.raw
  | Br r -> r.raw
  | Del r -> r.raw

let append_raw_nl (tok : token) : unit =
  match tok with
  | Space r -> r.raw <- Jstr.concat r.raw s_nl
  | Hr r -> r.raw <- Jstr.concat r.raw s_nl
  | Heading r -> r.raw <- Jstr.concat r.raw s_nl
  | Code r -> r.raw <- Jstr.concat r.raw s_nl
  | Table r -> r.raw <- Jstr.concat r.raw s_nl
  | Blockquote r -> r.raw <- Jstr.concat r.raw s_nl
  | List r -> r.raw <- Jstr.concat r.raw s_nl
  | Checkbox r -> r.raw <- Jstr.concat r.raw s_nl
  | Html r -> r.raw <- Jstr.concat r.raw s_nl
  | Def r -> r.raw <- Jstr.concat r.raw s_nl
  | Text t -> t.t_raw <- Jstr.concat t.t_raw s_nl
  | Escape _ | Link _ | Image _ | Strong _ | Em _ | Codespan _ | Br _ | Del _ -> ()

let enqueue_inline (lex : lexer) (src : Jstr.t) : token Vec.t =
  let tokens = Vec.create () in
  Vec.push lex.inline_queue { iq_src = src; iq_tokens = tokens };
  tokens
