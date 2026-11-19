#!/usr/bin/env node
// Runs one timing cell (tools/lib/protocol.mjs `runCell`) in this fresh Node process, the Node
// counterpart of a fresh browser page load. tools/bench.mjs spawns it once per cell, writes the
// cell configuration to stdin and reads the JSON result from stdout; heartbeats go to stderr
// synchronously so the parent can stop a cell that hangs. Adapted from
// experiments/browser-workloads/tools/timing-cell.mjs.
import { writeSync } from 'node:fs'

import { runCell } from './lib/protocol.mjs'

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const result = await runCell(JSON.parse(Buffer.concat(chunks).toString('utf8')), {
  heartbeat: (label) => writeSync(2, `\u0001heartbeat ${label}\n`),
})
process.stdout.write(JSON.stringify(result))
