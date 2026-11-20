(* noble's hmac.ts: _HMAC over the hash that [create] makes. *)

open Hash

let make (create : unit -> Hash.t) key : Hash.t =
  let i_hash = create () in
  let block_len = i_hash.block_len in
  let pad = Array.make block_len 0 in
  (* A key longer than a block is replaced by its digest. *)
  let key =
    if Array.length key > block_len then begin
      let long = create () in
      long.update key;
      long.digest ()
    end
    else key
  in
  Array.blit key 0 pad 0 (Array.length key);
  let xor_pad mask = Array.iteri (fun i b -> pad.(i) <- b lxor mask) pad in
  xor_pad 0x36;
  i_hash.update pad;
  let o_hash = create () in
  (* Undo the inner XOR and apply the outer one. *)
  xor_pad (0x36 lxor 0x5c);
  o_hash.update pad;
  {
    block_len;
    output_len = i_hash.output_len;
    update = i_hash.update;
    digest =
      (fun () ->
        o_hash.update (i_hash.digest ());
        o_hash.digest ());
  }
