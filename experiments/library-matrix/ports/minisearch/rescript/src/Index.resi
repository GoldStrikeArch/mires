// The port's one export (README, "Port convention"): one corpus input, `{ options, operations }`,
// to its result, `{ results }`.
type input

let run: input => JSON.t
