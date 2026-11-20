(* String <-> Uint8Array codecs (strToU8, strFromU8) and UTF-16LE adapter encoding. *)

open Host

let str_to_u8 text latin1 =
  if latin1 then begin
    let len = Text.length text in
    let out = U8.create len in
    for i = 0 to len - 1 do
      U8.set out i (Text.unit_at text i land 255)
    done;
    out
  end
  else Text.encode_utf8 text

let str_from_u8 dat latin1 =
  if latin1 then begin
    let len = U8.length dat in
    if len = 0 then Text.empty
    else
      let acc = ref Text.empty in
      let i = ref 0 in
      while !i < len do
        let stop =
          let next = !i + 16384 in
          if next < len then next else len
        in
        acc := Text.append !acc (Text.from_u8_chunk (U8.subarray dat !i stop));
        i := stop
      done;
      !acc
  end
  else Text.decode_utf8 dat

let text_to_utf16le text =
  let len = Text.length text in
  let out = U8.create (len lsl 1) in
  for i = 0 to len - 1 do
    let u = Text.unit_at text i in
    let j = i lsl 1 in
    U8.set out j (u land 255);
    U8.set out (j + 1) (u lsr 8)
  done;
  out
