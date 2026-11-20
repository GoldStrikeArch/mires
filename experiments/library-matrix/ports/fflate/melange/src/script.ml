(* Corpus runner / dispatcher matching experiments/library-matrix/libraries/fflate/reference.mjs. *)

open Host

let req_field obj name =
  match Value.field obj name with
  | Some v -> v
  | None -> invalid_arg ("fflate port: missing field " ^ name)

let opt_bool obj name =
  match Value.field obj name with
  | Some v -> Value.as_bool v
  | None -> false

let decode_payload spec =
  match Value.field spec "bytes" with
  | Some bytes_v -> Value.as_u8_from_int_array bytes_v
  | None ->
      let text = Value.as_text (req_field spec "text") in
      let latin1 = opt_bool spec "latin1" in
      Codec.str_to_u8 text latin1

let decode_compress_options input =
  match Value.field input "options" with
  | None -> Deflate.empty_compress_options
  | Some o ->
      let level = match Value.field o "level" with Some v -> Some (Value.as_int v) | None -> None in
      let mem = match Value.field o "mem" with Some v -> Some (Value.as_int v) | None -> None in
      let dictionary =
        match Value.field o "dictionary" with Some v -> Some (decode_payload v) | None -> None
      in
      let mtime =
        match Value.field o "mtime" with Some v -> Some (Value.as_float v) | None -> None
      in
      let filename =
        match Value.field o "filename" with Some v -> Some (Value.as_text v) | None -> None
      in
      { Deflate.level; mem; dictionary; mtime; filename }

let decode_decompress_options input =
  match Value.field input "options" with
  | None -> Inflate.empty_decompress_options
  | Some o ->
      let dictionary =
        match Value.field o "dictionary" with Some v -> Some (decode_payload v) | None -> None
      in
      let out =
        match Value.field o "outSize" with Some v -> Some (U8.create (Value.as_int v)) | None -> None
      in
      { Inflate.dictionary; out }

let run_compress_stream make_stream input =
  let opts = decode_compress_options input in
  let no_handler = opt_bool input "noHandler" in
  let records = ref [] in
  let handler =
    if no_handler then None
    else
      Some
        (fun chunk is_final ->
          records := { Stream.chunk = U8.copy_of chunk; is_final } :: !records)
  in
  let strm = make_stream opts handler in
  let steps = Value.as_array (req_field input "steps") in
  let n = Array.length steps in
  for i = 0 to n - 1 do
    let step = steps.(i) in
    match Value.as_string_name (req_field step "action") with
    | "push" ->
        let data = decode_payload (req_field step "data") in
        let is_final = opt_bool step "final" in
        Stream.push_compress strm data is_final
    | "flush" ->
        let sync = opt_bool step "sync" in
        Stream.flush_compress strm sync
    | other -> invalid_arg ("fflate port: unknown stream action " ^ other)
  done;
  Stream.frame_records !records

let run_decompress_stream make_stream push_fn input =
  let opts = decode_decompress_options input in
  let no_handler = opt_bool input "noHandler" in
  let records = ref [] in
  let handler =
    if no_handler then None
    else
      Some
        (fun chunk is_final ->
          records := { Stream.chunk = U8.copy_of chunk; is_final } :: !records)
  in
  let strm = make_stream opts handler in
  let steps = Value.as_array (req_field input "steps") in
  let n = Array.length steps in
  for i = 0 to n - 1 do
    let step = steps.(i) in
    match Value.as_string_name (req_field step "action") with
    | "push" ->
        let data = decode_payload (req_field step "data") in
        let is_final = opt_bool step "final" in
        push_fn strm data is_final
    | other -> invalid_arg ("fflate port: unknown stream action " ^ other)
  done;
  Stream.frame_records !records

let execute input =
  match Value.as_string_name (req_field input "op") with
  | "deflateSync" ->
      Deflate.deflate_sync (decode_payload (req_field input "data")) (decode_compress_options input)
  | "inflateSync" ->
      Inflate.inflate_sync
        (decode_payload (req_field input "data"))
        (decode_decompress_options input)
  | "gzipSync" ->
      Framing.gzip_sync (decode_payload (req_field input "data")) (decode_compress_options input)
  | "gunzipSync" ->
      Framing.gunzip_sync
        (decode_payload (req_field input "data"))
        (decode_decompress_options input)
  | "zlibSync" ->
      Framing.zlib_sync (decode_payload (req_field input "data")) (decode_compress_options input)
  | "unzlibSync" ->
      Framing.unzlib_sync
        (decode_payload (req_field input "data"))
        (decode_decompress_options input)
  | "decompressSync" ->
      Framing.decompress_sync
        (decode_payload (req_field input "data"))
        (decode_decompress_options input)
  | "strToU8" ->
      let text = Value.as_text (req_field input "text") in
      let latin1 = opt_bool input "latin1" in
      Codec.str_to_u8 text latin1
  | "strFromU8" ->
      let data = decode_payload (req_field input "data") in
      let latin1 = opt_bool input "latin1" in
      Codec.text_to_utf16le (Codec.str_from_u8 data latin1)
  | "Deflate" -> run_compress_stream Stream.create_deflate input
  | "Gzip" -> run_compress_stream Stream.create_gzip input
  | "Zlib" -> run_compress_stream Stream.create_zlib input
  | "Inflate" -> run_decompress_stream Stream.create_inflate Stream.push_inflate input
  | "Gunzip" -> run_decompress_stream Stream.create_gunzip Stream.push_gunzip input
  | "Unzlib" -> run_decompress_stream Stream.create_unzlib Stream.push_unzlib input
  | "Decompress" -> run_decompress_stream Stream.create_decompress Stream.push_decompress input
  | other -> invalid_arg ("fflate port: unknown operation " ^ other)

let run input =
  if not (opt_bool input "expectErrorCode") then execute input
  else
    let caught =
      try
        let _ = execute input in
        None
      with Tables.Flate_error (code, _) -> Some code
    in
    match caught with
    | Some code ->
        let out = U8.create 2 in
        U8.set out 0 0xff;
        U8.set out 1 code;
        out
    | None -> invalid_arg "fflate port: expected FlateError, but call succeeded"
