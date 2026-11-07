// Wires the page to the compiled program, and decides nothing.
//
// `bench_init` builds the application and answers it; every other entry point takes it back. A
// button handler passes it on and no more, and the one delegated handler passes the click's
// target: which row that is, and whether the click was a delete or a select, is worked out in
// Rust. This file exists because a compiled program cannot yet hand a closure to
// `addEventListener`, and it is as small as that gap.

const app = bench_init();

const on = (id, run) =>
  document.getElementById(id).addEventListener("click", (event) => {
    event.preventDefault();
    run(app);
  });

on("run", bench_run);
on("runlots", bench_runlots);
on("add", bench_add);
on("update", bench_update);
on("clear", bench_clear);
on("swaprows", bench_swaprows);

document.getElementById("tbody").addEventListener("click", (event) => {
  event.preventDefault();
  bench_click(app, event.target);
});
