(* d3-hierarchy 3.1.3 layout algorithms. Links are arena indices. *)
type node = {
  mutable id : string;
  mutable kids : int array;
  mutable parent : int;
  mutable depth : int;
  mutable height : int;
  mutable number : int;
  mutable ancestor : int;
  mutable defaultAncestor : int;
  mutable thread : int;
  mutable value : float;
  mutable x : float;
  mutable y : float;
  mutable r : float;
  mutable x0 : float;
  mutable y0 : float;
  mutable x1 : float;
  mutable y1 : float;
  mutable z : float;
  mutable m : float;
  mutable c : float;
  mutable s : float;
}
let circle x y r = {id="";kids=[||];parent=(-1);depth=0;height=0;number=0;ancestor=0;defaultAncestor=(-1);thread=(-1);value=0.;x0=0.;y0=0.;x1=0.;y1=0.;z=0.;m=0.;c=0.;s=0.;x;y;r}
let ints n = Array.make n 0
let floats n = Array.make n 0.
let real = float_of_int
let integer = int_of_float
let sqrtf = sqrt
let minf a b = if Float.is_nan a || Float.is_nan b then nan else Float.min a b
let maxf a b = if Float.is_nan a || Float.is_nan b then nan else Float.max a b
let floorf = floor
let absf = abs_float
let roundf x = if x >= -0.5 && x < 0. then -0. else floor (x+.0.5)
exception Return of int
let invalidBasis () = failwith "invalid enclosing basis"

let separation (nodes : node array) (a : int) (b : int) : float =
  (if ((nodes).(a).parent = (nodes).(b).parent) then 1.0 else 2.0)

let contour (nodes : node array) (v : int) (left : bool) : int =
  let kids = (nodes).(v).kids in
  if ((Array.length kids) = 0) then begin
    (nodes).(v).thread
  end else begin
    (if left then (kids).(0) else (kids).(((Array.length kids) - 1)))
  end

let apportion (nodes : node array) (v : int) (previous : int) (fallback : int) : int =
  let ancestor = ref (fallback) in
  if (previous < 0) then begin
    !ancestor
  end else begin
    let vip = ref (v) in
    let vop = ref (v) in
    let vim = ref (previous) in
    let vom = ref (((nodes).((nodes).(v).parent).kids).(0)) in
    let sip = ref ((nodes).(!vip).m) in
    let sop = ref ((nodes).(!vop).m) in
    let sim = ref ((nodes).(!vim).m) in
    let som = ref ((nodes).(!vom).m) in
    vim := (contour (nodes) (!vim) (false));
    vip := (contour (nodes) (!vip) (true));
    while ((!vim >= 0) && (!vip >= 0)) do
      vom := (contour (nodes) (!vom) (true));
      vop := (contour (nodes) (!vop) (false));
      (nodes).(!vop).ancestor <- v;
      let shift = (((((nodes).(!vim).z +. !sim) -. (nodes).(!vip).z) -. !sip) +. (separation (nodes) (!vim) (!vip))) in
      if (shift > 0.0) then begin
        let anc = (nodes).(!vim).ancestor in
        let wm = (if ((nodes).(anc).parent = (nodes).(v).parent) then anc else !ancestor) in
        let change = (shift /. (real (((nodes).(v).number - (nodes).(wm).number)))) in
        (nodes).(v).c <- ((nodes).(v).c -. change);
        (nodes).(v).s <- ((nodes).(v).s +. shift);
        (nodes).(wm).c <- ((nodes).(wm).c +. change);
        (nodes).(v).z <- ((nodes).(v).z +. shift);
        (nodes).(v).m <- ((nodes).(v).m +. shift);
        sip := (!sip +. shift);
        sop := (!sop +. shift);
        ()
      end;
      sim := (!sim +. (nodes).(!vim).m);
      sip := (!sip +. (nodes).(!vip).m);
      som := (!som +. (nodes).(!vom).m);
      sop := (!sop +. (nodes).(!vop).m);
      vim := (contour (nodes) (!vim) (false));
      vip := (contour (nodes) (!vip) (true));
      ()
    done;
    if ((!vim >= 0) && ((contour (nodes) (!vop) (false)) < 0)) then begin
      (nodes).(!vop).thread <- !vim;
      (nodes).(!vop).m <- ((nodes).(!vop).m +. (!sim -. !sop));
      ()
    end;
    if ((!vip >= 0) && ((contour (nodes) (!vom) (true)) < 0)) then begin
      (nodes).(!vom).thread <- !vip;
      (nodes).(!vom).m <- ((nodes).(!vom).m +. (!sip -. !som));
      ancestor := v;
      ()
    end;
    !ancestor
  end

let tidy (nodes : node array) (pre : int array) (post : int array) (w : float) (h : float) (fixed : bool) : unit =
  let root = (pre).(0) in
  Array.iter (fun v ->
    (nodes).(v).ancestor <- v;
    (nodes).(v).thread <- (-1);
    (nodes).(v).defaultAncestor <- (-1);
    let kids = (nodes).(v).kids in
    for i = 0 to ((Array.length kids))-1 do
      (nodes).((kids).(i)).number <- i;
      ()
    done;
    ()
  ) (pre);
  Array.iter (fun v ->
    let n = (nodes).(v) in
    let p = n.parent in
    let prev = ref ((-1)) in
    if (n.number > 0) then begin
      prev := ((nodes).(p).kids).((n.number - 1));
      ()
    end;
    let kids = n.kids in
    if ((Array.length kids) > 0) then begin
      let shift = ref (0.0) in
      let change = ref (0.0) in
      for i = ((Array.length kids))-1 downto 0 do
        let child = (nodes).((kids).(i)) in
        child.z <- (child.z +. !shift);
        child.m <- (child.m +. !shift);
        change := (!change +. child.c);
        shift := (!shift +. (child.s +. !change));
        ()
      done;
      let mid = (((nodes).((kids).(0)).z +. (nodes).((kids).(((Array.length kids) - 1))).z) /. 2.0) in
      if (!prev < 0) then begin
        n.z <- mid;
        ()
      end else begin
        n.z <- ((nodes).(!prev).z +. (separation (nodes) (v) (!prev)));
        n.m <- (n.z -. mid);
        ()
      end;
      ()
    end else begin
      if (!prev >= 0) then begin
        n.z <- ((nodes).(!prev).z +. (separation (nodes) (v) (!prev)));
        ()
      end;
      ()
    end;
    if (p >= 0) then begin
      let ancestor = ref ((nodes).(p).defaultAncestor) in
      if (!ancestor < 0) then begin
        ancestor := ((nodes).(p).kids).(0);
        ()
      end;
      (nodes).(p).defaultAncestor <- (apportion (nodes) (v) (!prev) (!ancestor));
      ()
    end;
    ()
  ) (post);
  let offset = (-.(nodes).(root).z) in
  let left = ref (root) in
  let right = ref (root) in
  let depth = ref (0) in
  Array.iter (fun v ->
    let n = (nodes).(v) in
    let modifier = (if (v = root) then offset else (nodes).(n.parent).m) in
    n.x <- (n.z +. modifier);
    n.m <- (n.m +. modifier);
    if (n.x < (nodes).(!left).x) then begin
      left := v;
      ()
    end;
    if (n.x > (nodes).(!right).x) then begin
      right := v;
      ()
    end;
    if (n.depth > !depth) then begin
      depth := n.depth;
      ()
    end;
    ()
  ) (pre);
  let margin = (if (!left = !right) then 1.0 else ((separation (nodes) (!left) (!right)) /. 2.0)) in
  let tx = (margin -. (nodes).(!left).x) in
  let kx = (w /. (((nodes).(!right).x +. margin) +. tx)) in
  let ky = (h /. (real ((if (!depth = 0) then 1 else !depth)))) in
  Array.iter (fun v ->
    let n = (nodes).(v) in
    if fixed then begin
      n.x <- (n.x *. w);
      n.y <- ((real (n.depth)) *. h);
      ()
    end else begin
      n.x <- ((n.x +. tx) *. kx);
      n.y <- ((real (n.depth)) *. ky);
      ()
    end;
    ()
  ) (pre);
  ()

let cluster (nodes : node array) (pre : int array) (post : int array) (w : float) (h : float) (fixed : bool) : unit =
  let previous = ref ((-1)) in
  let x = ref (0.0) in
  Array.iter (fun v ->
    let n = (nodes).(v) in
    if ((Array.length n.kids) = 0) then begin
      if (!previous >= 0) then begin
        x := (!x +. (separation (nodes) (v) (!previous)));
        ()
      end;
      n.x <- !x;
      previous := v;
      ()
    end else begin
      let height = ref (0.0) in
      let total = ref (0.0) in
      Array.iter (fun c ->
        total := (!total +. (nodes).(c).x);
        height := (maxf (!height) ((nodes).(c).y));
        ()
      ) (n.kids);
      n.x <- (!total /. (real ((Array.length n.kids))));
      n.y <- (1.0 +. !height);
      ()
    end;
    ()
  ) (post);
  let left = (post).(0) in
  let right = !previous in
  let x0 = ((nodes).(left).x -. ((separation (nodes) (left) (right)) /. 2.0)) in
  let x1 = ((nodes).(right).x +. ((separation (nodes) (right) (left)) /. 2.0)) in
  let rx = (nodes).((pre).(0)).x in
  let ry = (nodes).((pre).(0)).y in
  Array.iter (fun v ->
    let n = (nodes).(v) in
    if fixed then begin
      n.x <- ((n.x -. rx) *. w);
      n.y <- ((ry -. n.y) *. h);
      ()
    end else begin
      n.x <- (((n.x -. x0) /. (x1 -. x0)) *. w);
      n.y <- ((1.0 -. (if (ry <> 0.0) then (n.y /. ry) else 1.0)) *. h);
      ()
    end;
    ()
  ) (post);
  ()

let tileSlice (nodes : node array) (kids : int array) (start : int) (stop : int) (total : float) (x0 : float) (y0 : float) (x1 : float) (y1 : float) (dice : bool) : unit =
  let x = ref (x0) in
  let y = ref (y0) in
  let scale = (if (total = 0.0) then total else (if dice then ((x1 -. !x) /. total) else ((y1 -. !y) /. total))) in
  for i = start to (stop)-1 do
    let n = (nodes).((kids).(i)) in
    n.x0 <- !x;
    n.y0 <- !y;
    n.x1 <- x1;
    n.y1 <- y1;
    if dice then begin
      x := (!x +. (n.value *. scale));
      n.x1 <- !x;
      ()
    end else begin
      y := (!y +. (n.value *. scale));
      n.y1 <- !y;
      ()
    end;
    ()
  done;
  ()

let rec binary (nodes : node array) (kids : int array) (sums : float array) (start : int) (stop : int) (total : float) (x0 : float) (y0 : float) (x1 : float) (y1 : float) : unit =
  if (start >= (stop - 1)) then begin
    let n = (nodes).((kids).(start)) in
    n.x0 <- x0;
    n.y0 <- y0;
    n.x1 <- x1;
    n.y1 <- y1;
    ()
  end else begin
    let off = (sums).(start) in
    let target = ((total /. 2.0) +. off) in
    let k = ref ((start + 1)) in
    let hi = ref ((stop - 1)) in
    while (!k < !hi) do
      let mid = ((!k + !hi) / 2) in
      if ((sums).(mid) < target) then begin
        k := (mid + 1);
        ()
      end else begin
        hi := mid;
        ()
      end;
      ()
    done;
    if (((target -. (sums).((!k - 1))) < ((sums).(!k) -. target)) && ((start + 1) < !k)) then begin
      k := (!k - 1);
      ()
    end;
    let left = ((sums).(!k) -. off) in
    let right = (total -. left) in
    if ((x1 -. x0) > (y1 -. y0)) then begin
      let x = (if (total <> 0.0) then (((x0 *. right) +. (x1 *. left)) /. total) else x1) in
      (binary (nodes) (kids) (sums) (start) (!k) (left) (x0) (y0) (x) (y1));
      (binary (nodes) (kids) (sums) (!k) (stop) (right) (x) (y0) (x1) (y1));
      ()
    end else begin
      let y = (if (total <> 0.0) then (((y0 *. right) +. (y1 *. left)) /. total) else y1) in
      (binary (nodes) (kids) (sums) (start) (!k) (left) (x0) (y0) (x1) (y));
      (binary (nodes) (kids) (sums) (!k) (stop) (right) (x0) (y) (x1) (y1));
      ()
    end;
    ()
  end;
  ()

let squarify (nodes : node array) (kids : int array) (value : float) (left : float) (top : float) (x1 : float) (y1 : float) : unit =
  let total = ref (value) in
  let x = ref (left) in
  let y = ref (top) in
  let i = ref (0) in
  let j = ref (0) in
  let ratio = ((1.0 +. (sqrtf (5.0))) /. 2.0) in
  while (!i < (Array.length kids)) do
    let dx = (x1 -. !x) in
    let dy = (y1 -. !y) in
    let row = ref ((nodes).((kids).(!j)).value) in
    j := (!j + 1);
    while ((!row = 0.0) && (!j < (Array.length kids))) do
      row := (nodes).((kids).(!j)).value;
      j := (!j + 1);
      ()
    done;
    let low = ref (!row) in
    let high = ref (!row) in
    let alpha = ((maxf ((dy /. dx)) ((dx /. dy))) /. (!total *. ratio)) in
    let beta = ref (((!row *. !row) *. alpha)) in
    let best = ref ((maxf ((!high /. !beta)) ((!beta /. !low)))) in
    let finished = ref (false) in
    while ((!j < (Array.length kids)) && (not !finished)) do
      let v = (nodes).((kids).(!j)).value in
      row := (!row +. v);
      low := (minf (!low) (v));
      high := (maxf (!high) (v));
      beta := ((!row *. !row) *. alpha);
      let score = (maxf ((!high /. !beta)) ((!beta /. !low))) in
      if (score > !best) then begin
        row := (!row -. v);
        finished := true;
        ()
      end else begin
        best := score;
        j := (!j + 1);
        ()
      end;
      ()
    done;
    if (dx < dy) then begin
      let bottom = (if (!total <> 0.0) then (!y +. ((dy *. !row) /. !total)) else y1) in
      (tileSlice (nodes) (kids) (!i) (!j) (!row) (!x) (!y) (x1) (bottom) (true));
      if (!total <> 0.0) then begin
        y := bottom;
        ()
      end;
      ()
    end else begin
      let right = (if (!total <> 0.0) then (!x +. ((dx *. !row) /. !total)) else x1) in
      (tileSlice (nodes) (kids) (!i) (!j) (!row) (!x) (!y) (right) (y1) (false));
      if (!total <> 0.0) then begin
        x := right;
        ()
      end;
      ()
    end;
    total := (!total -. !row);
    i := !j;
    ()
  done;
  ()

let rectangles (nodes : node array) (pre : int array) (w : float) (h : float) (p : float) (partition : bool) (tile : string) (rounded : bool) : unit =
  let root = (nodes).((pre).(0)) in
  let levels = (real ((root.height + 1))) in
  root.x1 <- w;
  root.y1 <- h;
  if partition then begin
    root.x0 <- p;
    root.y0 <- p;
    root.y1 <- (h /. levels);
    ()
  end;
  Array.iter (fun v ->
    let n = (nodes).(v) in
    let kids = n.kids in
    let inset = (if (n.parent >= 0) then (p /. 2.0) else 0.0) in
    if partition then begin
      if ((Array.length kids) > 0) then begin
        (tileSlice (nodes) (kids) (0) ((Array.length kids)) (n.value) (n.x0) (((h *. (real ((n.depth + 1)))) /. levels)) (n.x1) (((h *. (real ((n.depth + 2)))) /. levels)) (true));
        ()
      end;
      n.x1 <- (n.x1 -. p);
      n.y1 <- (n.y1 -. p);
      ()
    end else begin
      n.x0 <- (n.x0 +. inset);
      n.y0 <- (n.y0 +. inset);
      n.x1 <- (n.x1 -. inset);
      n.y1 <- (n.y1 -. inset);
      ()
    end;
    if (n.x1 < n.x0) then begin
      n.x0 <- ((n.x0 +. n.x1) /. 2.0);
      n.x1 <- n.x0;
      ()
    end;
    if (n.y1 < n.y0) then begin
      n.y0 <- ((n.y0 +. n.y1) /. 2.0);
      n.y1 <- n.y0;
      ()
    end;
    if ((not partition) && ((Array.length kids) > 0)) then begin
      let gap = (p -. (p /. 2.0)) in
      let x = ref ((n.x0 +. gap)) in
      let y = ref ((n.y0 +. gap)) in
      let xx = ref ((n.x1 -. gap)) in
      let yy = ref ((n.y1 -. gap)) in
      if (!xx < !x) then begin
        x := ((!x +. !xx) /. 2.0);
        xx := !x;
        ()
      end;
      if (!yy < !y) then begin
        y := ((!y +. !yy) /. 2.0);
        yy := !y;
        ()
      end;
      if (tile = "binary") then begin
        let sums = (floats (((Array.length kids) + 1))) in
        for j = 0 to ((Array.length kids))-1 do
          (sums).((j + 1)) <- ((sums).(j) +. (nodes).((kids).(j)).value);
          ()
        done;
        (binary (nodes) (kids) (sums) (0) ((Array.length kids)) (n.value) (!x) (!y) (!xx) (!yy));
        ()
      end else begin
        if (tile = "sliceDice") then begin
          (tileSlice (nodes) (kids) (0) ((Array.length kids)) (n.value) (!x) (!y) (!xx) (!yy) (((n.depth mod 2) = 0)));
          ()
        end else begin
          (squarify (nodes) (kids) (n.value) (!x) (!y) (!xx) (!yy));
          ()
        end;
        ()
      end;
      ()
    end;
    ()
  ) (pre);
  if rounded then begin
    Array.iter (fun v ->
      let n = (nodes).(v) in
      n.x0 <- (roundf (n.x0));
      n.x1 <- (roundf (n.x1));
      n.y0 <- (roundf (n.y0));
      n.y1 <- (roundf (n.y1));
      ()
    ) (pre);
    ()
  end;
  ()

let weak (a : node) (b : node) : bool =
  let dr = ((a.r -. b.r) +. ((maxf ((maxf (a.r) (b.r))) (1.0)) *. 1e-09)) in
  let dx = (b.x -. a.x) in
  let dy = (b.y -. a.y) in
  ((dr > 0.0) && ((dr *. dr) > ((dx *. dx) +. (dy *. dy))))

let outside (a : node) (b : node) : bool =
  let dr = (a.r -. b.r) in
  let dx = (b.x -. a.x) in
  let dy = (b.y -. a.y) in
  ((dr < 0.0) || ((dr *. dr) < ((dx *. dx) +. (dy *. dy))))

let pair (a : node) (b : node) : node =
  let dx = (b.x -. a.x) in
  let dy = (b.y -. a.y) in
  let dr = (b.r -. a.r) in
  let distance = (sqrtf (((dx *. dx) +. (dy *. dy)))) in
  (circle ((((a.x +. b.x) +. ((dx /. distance) *. dr)) /. 2.0)) ((((a.y +. b.y) +. ((dy /. distance) *. dr)) /. 2.0)) ((((distance +. a.r) +. b.r) /. 2.0)))

let triple (a : node) (b : node) (c : node) : node =
  let a2 = (a.x -. b.x) in
  let a3 = (a.x -. c.x) in
  let b2 = (a.y -. b.y) in
  let b3 = (a.y -. c.y) in
  let c2 = (b.r -. a.r) in
  let c3 = (c.r -. a.r) in
  let d1 = (((a.x *. a.x) +. (a.y *. a.y)) -. (a.r *. a.r)) in
  let d2 = (((d1 -. (b.x *. b.x)) -. (b.y *. b.y)) +. (b.r *. b.r)) in
  let d3 = (((d1 -. (c.x *. c.x)) -. (c.y *. c.y)) +. (c.r *. c.r)) in
  let ab = ((a3 *. b2) -. (a2 *. b3)) in
  let xa = ((((b2 *. d3) -. (b3 *. d2)) /. (ab *. 2.0)) -. a.x) in
  let xb = (((b3 *. c2) -. (b2 *. c3)) /. ab) in
  let ya = ((((a3 *. d2) -. (a2 *. d3)) /. (ab *. 2.0)) -. a.y) in
  let yb = (((a2 *. c3) -. (a3 *. c2)) /. ab) in
  let qa = (((xb *. xb) +. (yb *. yb)) -. 1.0) in
  let qb = (2.0 *. ((a.r +. (xa *. xb)) +. (ya *. yb))) in
  let qc = (((xa *. xa) +. (ya *. ya)) -. (a.r *. a.r)) in
  let r = (-.(if ((absf (qa)) > 1e-06) then ((qb +. (sqrtf (((qb *. qb) -. ((4.0 *. qa) *. qc))))) /. (2.0 *. qa)) else (qc /. qb))) in
  (circle (((a.x +. xa) +. (xb *. r))) (((a.y +. ya) +. (yb *. r))) (r))

let covers (nodes : node array) (basis : int array) (count : int) (a : node) : bool =
  let i = ref (0) in
  while ((!i < count) && (weak (a) ((nodes).((basis).(!i))))) do
    i := (!i + 1);
    ()
  done;
  (!i = count)

let extend (nodes : node array) (basis : int array) (count : int) (p : int) : int =
  try
    if (covers (nodes) (basis) (count) ((nodes).(p))) then begin
      (basis).(0) <- p;
      raise (Return (1))
    end;
    for i = 0 to (count)-1 do
      let x = (basis).(i) in
      if ((outside ((nodes).(p)) ((nodes).(x))) && (covers (nodes) (basis) (count) ((pair ((nodes).(x)) ((nodes).(p)))))) then begin
        (basis).(0) <- x;
        (basis).(1) <- p;
        raise (Return (2))
      end;
      ()
    done;
    for i = 0 to (count)-1 do
      let x = (basis).(i) in
      for j = (i + 1) to (count)-1 do
        let y = (basis).(j) in
        if ((outside ((pair ((nodes).(x)) ((nodes).(y)))) ((nodes).(p))) && (outside ((pair ((nodes).(x)) ((nodes).(p)))) ((nodes).(y))) && (outside ((pair ((nodes).(y)) ((nodes).(p)))) ((nodes).(x))) && (covers (nodes) (basis) (count) ((triple ((nodes).(x)) ((nodes).(y)) ((nodes).(p)))))) then begin
          (basis).(0) <- x;
          (basis).(1) <- y;
          (basis).(2) <- p;
          raise (Return (3))
        end;
        ()
      done;
      ()
    done;
    (invalidBasis ())
  with Return result -> result

let enclose (nodes : node array) (front : int array) (n : int) (seed : float array) : node =
  for j = (n)-1 downto 0 do
    let step = ((1664525.0 *. (seed).(0)) +. 1013904223.0) in
    (seed).(0) <- (step -. ((floorf ((step /. 4294967296.0))) *. 4294967296.0));
    let k = (integer ((((seed).(0) /. 4294967296.0) *. (real ((j + 1)))))) in
    let save = (front).(j) in
    (front).(j) <- (front).(k);
    (front).(k) <- save;
    ()
  done;
  let basis = (ints (3)) in
  let count = ref (0) in
  let i = ref (0) in
  let result = ref ((circle (0.0) (0.0) (0.0))) in
  while (!i < n) do
    let p = (front).(!i) in
    if ((!count > 0) && (weak (!result) ((nodes).(p)))) then begin
      i := (!i + 1);
      ()
    end else begin
      count := (extend (nodes) (basis) (!count) (p));
      if (!count = 1) then begin
        result := (circle ((nodes).(p).x) ((nodes).(p).y) ((nodes).(p).r));
        ()
      end else begin
        if (!count = 2) then begin
          result := (pair ((nodes).((basis).(0))) ((nodes).((basis).(1))));
          ()
        end else begin
          result := (triple ((nodes).((basis).(0))) ((nodes).((basis).(1))) ((nodes).((basis).(2))));
          ()
        end;
        ()
      end;
      i := 0;
      ()
    end;
    ()
  done;
  !result

let place (b : node) (a : node) (c : node) : unit =
  let dx = (b.x -. a.x) in
  let dy = (b.y -. a.y) in
  let d2 = ((dx *. dx) +. (dy *. dy)) in
  if (d2 <> 0.0) then begin
    let a2 = ((a.r +. c.r) *. (a.r +. c.r)) in
    let b2 = ((b.r +. c.r) *. (b.r +. c.r)) in
    if (a2 > b2) then begin
      let x = (((d2 +. b2) -. a2) /. (2.0 *. d2)) in
      let y = (sqrtf ((maxf (0.0) (((b2 /. d2) -. (x *. x)))))) in
      c.x <- ((b.x -. (x *. dx)) -. (y *. dy));
      c.y <- ((b.y -. (x *. dy)) +. (y *. dx));
      ()
    end else begin
      let x = (((d2 +. a2) -. b2) /. (2.0 *. d2)) in
      let y = (sqrtf ((maxf (0.0) (((a2 /. d2) -. (x *. x)))))) in
      c.x <- ((a.x +. (x *. dx)) -. (y *. dy));
      c.y <- ((a.y +. (x *. dy)) +. (y *. dx));
      ()
    end;
    ()
  end else begin
    c.x <- (a.x +. c.r);
    c.y <- a.y;
    ()
  end;
  ()

let intersects (a : node) (b : node) : bool =
  let dr = ((a.r +. b.r) -. 1e-06) in
  let dx = (b.x -. a.x) in
  let dy = (b.y -. a.y) in
  ((dr > 0.0) && ((dr *. dr) > ((dx *. dx) +. (dy *. dy))))

let score (a : node) (b : node) : float =
  let ab = (a.r +. b.r) in
  let dx = (((a.x *. b.r) +. (b.x *. a.r)) /. ab) in
  let dy = (((a.y *. b.r) +. (b.y *. a.r)) /. ab) in
  ((dx *. dx) +. (dy *. dy))

let siblings (nodes : node array) (kids : int array) (seed : float array) : float =
  let n = (Array.length kids) in
  let a = ref (0) in
  let b = ref (1) in
  (nodes).((kids).(0)).x <- 0.0;
  (nodes).((kids).(0)).y <- 0.0;
  if (n = 1) then begin
    (nodes).((kids).(0)).r
  end else begin
    (nodes).((kids).(0)).x <- (-.(nodes).((kids).(1)).r);
    (nodes).((kids).(1)).x <- (nodes).((kids).(0)).r;
    (nodes).((kids).(1)).y <- 0.0;
    if (n = 2) then begin
      ((nodes).((kids).(0)).r +. (nodes).((kids).(1)).r)
    end else begin
      (place ((nodes).((kids).(1))) ((nodes).((kids).(0))) ((nodes).((kids).(2))));
      let after = (ints (n)) in
      let before = (ints (n)) in
      (after).(0) <- 1;
      (before).(2) <- 1;
      (after).(1) <- 2;
      (before).(0) <- 2;
      (after).(2) <- 0;
      (before).(1) <- 0;
      let i = ref (3) in
      while (!i < n) do
        (place ((nodes).((kids).(!a))) ((nodes).((kids).(!b))) ((nodes).((kids).(!i))));
        let j = ref ((after).(!b)) in
        let k = ref ((before).(!a)) in
        let sj = ref ((nodes).((kids).(!b)).r) in
        let sk = ref ((nodes).((kids).(!a)).r) in
        let retry = ref (false) in
        let more = ref (true) in
        while (!more && (not !retry)) do
          if (!sj <= !sk) then begin
            if (intersects ((nodes).((kids).(!j))) ((nodes).((kids).(!i)))) then begin
              b := !j;
              (after).(!a) <- !b;
              (before).(!b) <- !a;
              retry := true;
              ()
            end else begin
              sj := (!sj +. (nodes).((kids).(!j)).r);
              j := (after).(!j);
              ()
            end;
            ()
          end else begin
            if (intersects ((nodes).((kids).(!k))) ((nodes).((kids).(!i)))) then begin
              a := !k;
              (after).(!a) <- !b;
              (before).(!b) <- !a;
              retry := true;
              ()
            end else begin
              sk := (!sk +. (nodes).((kids).(!k)).r);
              k := (before).(!k);
              ()
            end;
            ()
          end;
          more := (!j <> (after).(!k));
          ()
        done;
        if (not !retry) then begin
          (before).(!i) <- !a;
          (after).(!i) <- !b;
          (after).(!a) <- !i;
          (before).(!b) <- !i;
          b := !i;
          let best = ref ((score ((nodes).((kids).(!a))) ((nodes).((kids).((after).(!a)))))) in
          j := (after).(!i);
          while (!j <> !b) do
            let candidateScore = (score ((nodes).((kids).(!j))) ((nodes).((kids).((after).(!j))))) in
            if (candidateScore < !best) then begin
              best := candidateScore;
              a := !j;
              ()
            end;
            j := (after).(!j);
            ()
          done;
          b := (after).(!a);
          i := (!i + 1);
          ()
        end;
        ()
      done;
      let front = (ints (n)) in
      (front).(0) <- (kids).(!b);
      let count = ref (1) in
      let cursor = ref ((after).(!b)) in
      while (!cursor <> !b) do
        (front).(!count) <- (kids).(!cursor);
        count := (!count + 1);
        cursor := (after).(!cursor);
        ()
      done;
      let enclosure = (enclose (nodes) (front) (!count) (seed)) in
      Array.iter (fun id ->
        (nodes).(id).x <- ((nodes).(id).x -. enclosure.x);
        (nodes).(id).y <- ((nodes).(id).y -. enclosure.y);
        ()
      ) (kids);
      enclosure.r
    end
  end

let packChildren (nodes : node array) (post : int array) (padding : float) (seed : float array) : unit =
  let pad = (if (padding <> padding) then 0.0 else padding) in
  Array.iter (fun i ->
    let kids = (nodes).(i).kids in
    if ((Array.length kids) > 0) then begin
      if (pad <> 0.0) then begin
        Array.iter (fun k ->
          (nodes).(k).r <- ((nodes).(k).r +. pad);
          ()
        ) (kids);
        ()
      end;
      let radius = (siblings (nodes) (kids) (seed)) in
      if (pad <> 0.0) then begin
        Array.iter (fun k ->
          (nodes).(k).r <- ((nodes).(k).r -. pad);
          ()
        ) (kids);
        ()
      end;
      (nodes).(i).r <- (radius +. pad);
      ()
    end;
    ()
  ) (post);
  ()

let pack (nodes : node array) (pre : int array) (post : int array) (w : float) (h : float) (p : float) : unit =
  let root = (nodes).((pre).(0)) in
  root.x <- (w /. 2.0);
  root.y <- (h /. 2.0);
  Array.iter (fun i ->
    let n = (nodes).(i) in
    if ((Array.length n.kids) = 0) then begin
      let raw = (sqrtf (n.value)) in
      n.r <- (if (raw = raw) then (maxf (0.0) (raw)) else 0.0);
      ()
    end;
    ()
  ) (pre);
  let seed = (floats (1)) in
  (seed).(0) <- 1.0;
  (packChildren (nodes) (post) (0.0) (seed));
  (packChildren (nodes) (post) ((p *. (root.r /. (minf (w) (h))))) (seed));
  let k = ((minf (w) (h)) /. (2.0 *. root.r)) in
  Array.iter (fun i ->
    let n = (nodes).(i) in
    n.r <- (n.r *. k);
    if (n.parent >= 0) then begin
      n.x <- ((nodes).(n.parent).x +. (k *. n.x));
      n.y <- ((nodes).(n.parent).y +. (k *. n.y));
      ()
    end;
    ()
  ) (pre);
  ()
