// The port's one export (README, "Port convention"): one corpus script, `{steps}` (with
// `cachePrefix`, `event` and `request` in service-worker mode), to a promise of its output,
// `{results, events, handlers, caches}`.
let run: Scenario.input => promise<JSON.t>
