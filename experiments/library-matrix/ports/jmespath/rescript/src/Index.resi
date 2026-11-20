// The port's one export (README, "Port convention"): one corpus input, `{ data, queries }`, to
// its result array, one `{ value }` or `{ error }` per query.
type query
type input

let run: input => array<dict<Host.value>>
