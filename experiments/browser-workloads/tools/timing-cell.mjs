#!/usr/bin/env node
// Runs one timing cell (tools/timing-protocol.mjs `runCell`) in this fresh Node process, the Node
// counterpart of a fresh browser page, and prints the JSON result on stdout. tools/run.mjs spawns
// it once per cell and writes the cell configuration (arm module URLs, input, warmups, samples,
// seed) to stdin.
import { runCell } from './timing-protocol.mjs'

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const result = await runCell(JSON.parse(Buffer.concat(chunks).toString('utf8')))
process.stdout.write(JSON.stringify(result))
