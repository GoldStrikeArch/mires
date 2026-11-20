let flatten ~missing rings =
  if Array.length rings = 0 || Array.length rings.(0) = 0 then invalid_arg "missing first vertex";
  let dimensions = Array.length rings.(0).(0) in
  let vertices = Vector.create missing and holes = Vector.create 0 in
  let prev_len = ref 0 and hole_index = ref 0 in
  Array.iter (fun ring ->
    Array.iter (fun p ->
      for d = 0 to dimensions - 1 do
        Vector.push vertices (if d < Array.length p then p.(d) else missing)
      done) ring;
    if !prev_len <> 0 then begin
      hole_index := !hole_index + !prev_len; Vector.push holes !hole_index
    end;
    prev_len := Array.length ring) rings;
  Vector.finish vertices, Vector.finish holes, dimensions
