(* noble's _md.ts: the Merkle–Damgård base of SHA-256 and SHA-512, and its Chi and Maj. *)

(* Bits of [b] where [a] is set, else of [c]. *)
let chi a b c = U32.((a land b) lxor (lnot a land c))

(* The majority of each bit of [a], [b] and [c]. *)
let maj a b c = U32.((a land b) lxor (a land c) lxor (b land c))

(* noble's HashMD. [process] compresses the block at an offset; [get] returns the state words,
   the high half before the low one for 64-bit words. [length] counts bytes, a JS number in
   noble. *)
type t = {
  block_len : int;
  output_len : int;
  pad_offset : int;
  buffer : int array;
  mutable pos : int;
  mutable length : float;
  process : int array -> int -> unit;
  get : unit -> int32 array;
}

let make ~block_len ~output_len ~pad_offset ~process ~get =
  { block_len; output_len; pad_offset; buffer = Array.make block_len 0; pos = 0; length = 0.; process; get }

let update h data =
  let len = Array.length data in
  let pos = ref 0 in
  while !pos < len do
    let take = min (h.block_len - h.pos) (len - !pos) in
    if take = h.block_len then
      (* With nothing buffered, whole blocks are compressed straight from the input. *)
      while h.block_len <= len - !pos do
        h.process data !pos;
        pos := !pos + h.block_len
      done
    else begin
      Array.blit data !pos h.buffer h.pos take;
      h.pos <- h.pos + take;
      pos := !pos + take;
      if h.pos = h.block_len then begin
        h.process h.buffer 0;
        h.pos <- 0
      end
    end
  done;
  h.length <- h.length +. float_of_int len

let digest h =
  let { buffer; block_len; _ } = h in
  buffer.(h.pos) <- 0x80;
  let pos = h.pos + 1 in
  Array.fill buffer pos (block_len - pos) 0;
  (* No room for the length field: pad this block and put the length in the next. *)
  if h.pad_offset > block_len - pos then begin
    h.process buffer 0;
    Array.fill buffer 0 block_len 0
  end;
  (* noble's setU64FromNum: only the low 64 bits of the length field; the rest stays zero. *)
  let bits = h.length *. 8. in
  Hash.set_uint32_be buffer (block_len - 8) (U64.from_num_h bits);
  Hash.set_uint32_be buffer (block_len - 4) (U64.from_num_l bits);
  h.process buffer 0;
  let state = h.get () in
  let out = Array.make h.output_len 0 in
  for i = 0 to (h.output_len / 4) - 1 do
    Hash.set_uint32_be out (4 * i) state.(i)
  done;
  out

let to_hash h : Hash.t =
  { Hash.block_len = h.block_len; output_len = h.output_len; update = update h; digest = (fun () -> digest h) }
