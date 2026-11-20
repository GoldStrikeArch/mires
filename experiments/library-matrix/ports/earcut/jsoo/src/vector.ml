(* Per-call append-only storage for triangles, block indices and Morton sort buffers. *)
type 'a t = { mutable items : 'a array; mutable size : int }
let create seed = { items = Array.make 16 seed; size = 0 }
let push v x =
  if v.size = Array.length v.items then begin
    let bigger = Array.make (v.size * 2) x in
    Array.blit v.items 0 bigger 0 v.size;
    v.items <- bigger
  end;
  v.items.(v.size) <- x;
  v.size <- v.size + 1
let finish v = Array.sub v.items 0 v.size
