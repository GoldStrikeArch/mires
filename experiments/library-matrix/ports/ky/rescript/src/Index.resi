// The port's one export (README, "Port convention"): one corpus input, `{ cases }`, to a promise of
// its result array, one `[trace, outcome]` per request case.
type input

let run: input => promise<array<JSON.t>>
