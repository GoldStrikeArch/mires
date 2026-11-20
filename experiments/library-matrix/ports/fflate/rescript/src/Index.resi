// The port's one export (README, "Port convention"): one corpus input to its raw `Uint8Array` result.
@tag("op")
type input

let run: input => Uint8Array.t
