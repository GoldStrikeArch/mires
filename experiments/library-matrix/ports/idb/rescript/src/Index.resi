// The port's one export (README, "Port convention"): one corpus input, `{ steps }`, to a promise of
// `{ log, databases }`.
type output = {log: array<JSON.t>, databases: array<JSON.t>}

let run: JSON.t => promise<output>
