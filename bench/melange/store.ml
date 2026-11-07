(* The krausest js-framework-benchmark non-keyed store, in OCaml, compiled to
   JavaScript by Melange v7.0.1.

   This is the OCaml half of the core-logic comparison. store.rs is the Rust
   half: the same data, the same operations, the same order of side effects,
   compiled by rustc_codegen_mires. Neither half touches the DOM -- what is being
   compared is the store, which is the part of a benchmark implementation that
   is written in the source language rather than delegated to a framework.

   IMPORT-FREE BY DESIGN
   ---------------------
   Melange's runtime lives in the `melange.js` opam package and is emitted as
   ordinary ES imports (`melange.js/caml_array.js` and friends). There is no
   opam here -- the compiler is the playground -- and those files are in no npm
   registry, so an output that imports one can neither be run under node nor
   honestly weighed: its real size would be its own bytes plus a runtime nobody
   present can produce.

   So every idiom below was chosen to emit no import at all, and
   `node extract.mjs` fails the build if one appears. What that ruled out, each
   verified against the v7.0.1 playground rather than assumed:

     * `a.(i)` and everything else in `Stdlib.Array`  ->  `melange.js/caml_array.js`
       (the bounds check is a runtime call). `Js.Array.unsafe_get/unsafe_set`
       are externals and compile to `a[i]`.
     * `Js.Math.random` and `Js.Math.round`  ->  `melange.js/js_math.js`. In
       v7 `Js.Math` is a module, not a wall of externals, so the two used here
       are declared locally with `[@@mel.scope "Math"]` and inline into
       `Math.random()` / `Math.round(x)` -- byte for byte the calls the
       reference implementation makes, and byte for byte what the Rust half's
       `#[js_extern]` emits.
     * integer `mod` by a NON-constant divisor  ->  `melange.js/caml_int32.js`
       (the divide-by-zero check). `mod_float` has no such check and compiles to
       `%`, so `random_below` does the remainder in float space and truncates
       once at the end -- see the note on that function.

   What survived unchanged is most of the language: records with mutable
   fields, `for`/`while`, `ref`, `^`, `int_of_float`, array literals. None of
   those costs an import.

   FAIRNESS NOTES (the ones a reader should be able to check)
   ----------------------------------------------------------
   * Both halves call the SAME `Math.random`. Neither seeds its own generator,
     so the harness can install one seeded function as `Math.random` and get
     bit-identical work -- and therefore bit-identical labels -- out of both.
   * The three `random_below` calls that build a label are bound with `let`
     rather than left as operands of `^`. OCaml does not specify the evaluation
     order of the arguments of an application and in practice evaluates them
     right to left, which would draw the noun's random number first and produce
     a different label than the reference for the same stream. The `let`s pin
     the order to the reference's left-to-right, and cost nothing: Melange
     inlines them straight back into the concatenation.
   * `Js.Array.push`, `Js.Array.concat` and `Js.Array.spliceInPlace` are
     externals over the real JS methods, which is what the reference calls. No
     OCaml list, no immutable copy: the container is a JavaScript array on both
     sides of the comparison. *)

(* ------------------------------------------------------------------ externals *)

(* `Math.random()` and `Math.round(x)`. Declared here rather than taken from
   `Js.Math` for the import reason above; the emission is identical. *)
external random : unit -> float = "random" [@@mel.scope "Math"]
external round : float -> float = "round" [@@mel.scope "Math"]

(* --------------------------------------------------------------------- data *)

type row = {
  id : int;
  mutable label : string;
}

type t = {
  (* A JavaScript array, mutated in place by add/delete/swap and replaced
     wholesale by run/runLots/clear, exactly as the reference store does. *)
  mutable rows : row array;
  (* Ids start at 1 and never reset, across every run in the process. *)
  mutable next_id : int;
  (* The index of the selected row, or -1. The reference uses `undefined`; an
     out-of-range int is the same information without an option allocation, and
     the Rust half spells it the same way. *)
  mutable selected : int;
}

let adjectives =
  [|
    "pretty"; "large"; "big"; "small"; "tall"; "short"; "long"; "handsome";
    "plain"; "quaint"; "clean"; "elegant"; "easy"; "angry"; "crazy"; "helpful";
    "mushy"; "odd"; "unsightly"; "adorable"; "important"; "inexpensive";
    "cheap"; "expensive"; "fancy";
  |]

(* "brown" twice, in the reference and therefore here. *)
let colours =
  [|
    "red"; "yellow"; "blue"; "green"; "pink"; "brown"; "purple"; "brown";
    "white"; "black"; "orange";
  |]

let nouns =
  [|
    "table"; "chair"; "house"; "bbq"; "desk"; "car"; "pony"; "cookie";
    "sandwich"; "burger"; "pizza"; "mouse"; "keyboard";
  |]

(* ------------------------------------------------------------------- random *)

(* The reference's `_random`:

     _random(max) { return Math.round(Math.random() * 1000) % max; }

   The remainder is taken in float space because integer `mod` with a variable
   divisor pulls `caml_int32` in for its divide-by-zero check, and `mod_float`
   does not. `Math.round` already returns a whole number, so the float
   remainder and the integer one agree for every value that can occur, and the
   single `int_of_float` at the end is the `| 0` the reference's implicit int
   conversion performs anyway. Emitted:

     Math.round(Math.random() * 1000) % max | 0 *)
let random_below max =
  int_of_float (Stdlib.mod_float (round (random () *. 1000.)) (float_of_int max))

(* ---------------------------------------------------------------- the store *)

let create () = { rows = [||]; next_id = 1; selected = -1 }

(* `count` fresh rows, taking ids from the store's counter. The rows are built
   into a new array rather than into `t.rows`, because `add` concatenates and
   `run` replaces. *)
let build_data t count =
  let data = [||] in
  for _ = 1 to count do
    (* Left to right, and see the fairness note in the header for why these are
       `let` bound rather than written inline. *)
    let adjective = Js.Array.unsafe_get adjectives (random_below (Js.Array.length adjectives)) in
    let colour = Js.Array.unsafe_get colours (random_below (Js.Array.length colours)) in
    let noun = Js.Array.unsafe_get nouns (random_below (Js.Array.length nouns)) in
    let row = { id = t.next_id; label = adjective ^ " " ^ colour ^ " " ^ noun } in
    t.next_id <- t.next_id + 1;
    ignore (Js.Array.push data ~value:row)
  done;
  data

let run t =
  t.rows <- build_data t 1000;
  t.selected <- -1

let run_lots t =
  t.rows <- build_data t 10000;
  t.selected <- -1

let add t = t.rows <- Js.Array.concat t.rows ~other:(build_data t 1000)

(* Every tenth row from index 0, appending " !!!" to the label in place. *)
let update t =
  let length = Js.Array.length t.rows in
  let index = ref 0 in
  while !index < length do
    let row = Js.Array.unsafe_get t.rows !index in
    row.label <- row.label ^ " !!!";
    index := !index + 10
  done

let select t index = t.selected <- index

let delete t index =
  ignore (Js.Array.spliceInPlace t.rows ~start:index ~remove:1 ~add:[||])

(* The reference swaps 1 and 998, and only when there are enough rows to have
   both. *)
let swap_rows t =
  if Js.Array.length t.rows > 998 then begin
    let held = Js.Array.unsafe_get t.rows 1 in
    Js.Array.unsafe_set t.rows 1 (Js.Array.unsafe_get t.rows 998);
    Js.Array.unsafe_set t.rows 998 held
  end

let clear t =
  t.rows <- [||];
  t.selected <- -1

(* ------------------------------------------------------------- observations *)

(* Read-only accessors, so the harness can compare the two stores element by
   element without knowing either representation. The Rust half exports the
   same four. *)

let length t = Js.Array.length t.rows

let row_id t index = (Js.Array.unsafe_get t.rows index).id

let row_label t index = (Js.Array.unsafe_get t.rows index).label

let selected t = t.selected
