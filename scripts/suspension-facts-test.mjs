import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const [file, fixture] = process.argv.slice(2);
const report = JSON.parse(readFileSync(file));
assert.equal(report.async.schema, 8);
assert.equal(report.async.mode, 'report-only');
const expected = fixture === 'initialization' ? {initialization: 1} : fixture === 'local'
  ? {immediate: 0, one_await: 1, two_awaits: 2, holds_a_struct: 1}
  : fixture === 'liveness' ? {borrowed: 1, moved_guard: 1, held_guard: 1, two_states: 2}
  : {relay: 1, repeated: 1, immediate: 0};
const found = {};
for (const graph of report.async.graphs) {
  assert.equal(graph.selected, false);
  assert.equal(graph.await_relationships_proven, false);
  assert.equal(graph.wake_sources_proven, false);
  const name = Object.keys(expected).find(name => graph.item.includes(`$${name}$`));
  if (!name) continue;
  const source = graph.source_suspensions;
  assert.equal(source.status, 'captured', graph.item);
  assert.equal(graph.runtime_frame_matches_type, true, graph.item);
  assert.ok(source.runtime_frame);
  assert.equal(source.runtime_frame.states.length, graph.state_count);
  assert.equal(source.runtime_frame.saved_local_sources.length, graph.saved_locals);
  assert.deepEqual(source.runtime_frame.states.slice(0, 3), [[], [], []]);
  for (const locals of source.runtime_frame.saved_local_sources) {
    assert.deepEqual(locals, [...new Set(locals)].sort((a, b) => a - b));
    assert.ok(locals.every(local => local >= 0 && local < source.local_count && local !== 1));
  }
  assert.equal(source.producer, 'rustc-mir-promoted-before-runtime-lowering');
  assert.equal(source.yield_count, expected[name], graph.item);
  assert.equal(source.yields.length, expected[name]);
  assert.equal(source.blocks.length, source.block_count);
  assert.ok(source.initialization);
  assert.equal(source.initialization.points.length, source.block_count);
  for (const point of source.initialization.points) {
    assert.equal(point.before === null, point.after === null);
    for (const state of [point.before, point.after].filter(Boolean)) {
      for (const paths of Object.values(state)) {
        assert.deepEqual(paths, [...new Set(paths)].sort((a, b) => a - b));
        assert.ok(paths.every(path => path >= 0 && path < source.initialization.paths.length));
      }
    }
  }
  const yieldBlocks = source.blocks.flatMap((block, index) => block.kind === 'Yield' ? [index] : []);
  assert.deepEqual(yieldBlocks, source.yields.map(point => point.block));
  const entries = [...new Set([0, ...source.yields.map(point => point.resume)])].sort((a, b) => a - b);
  assert.deepEqual(source.segments.map(segment => segment.entry), entries);
  for (const block of source.blocks) {
    for (const edge of block.successors) {
      assert.ok(['Normal', 'Unwind', 'Resume', 'Cancel', 'Imaginary'].includes(edge.kind));
      assert.ok(Number.isInteger(edge.target) && edge.target >= 0 && edge.target < source.block_count);
    }
  }
  // Independent fixed-point closure, including cycles, rather than the linker's worklist walk.
  for (const segment of source.segments) {
    const reached = new Set([segment.entry]);
    let changed;
    do {
      changed = false;
      source.blocks.forEach((block, index) => {
        if (!reached.has(index) || block.kind === 'Yield') return;
        for (const edge of block.successors) {
          if (edge.kind === 'Normal' && !reached.has(edge.target)) {
            reached.add(edge.target);
            changed = true;
          }
        }
      });
    } while (changed);
    const blocks = [...reached].sort((a, b) => a - b);
    assert.deepEqual(segment.blocks, blocks);
    assert.deepEqual(segment.yield_blocks, blocks.filter(index => source.blocks[index].kind === 'Yield'));
    assert.deepEqual(segment.terminal_blocks, blocks.filter(index => {
      const block = source.blocks[index];
      return block.kind !== 'Yield' && !block.successors.some(edge => edge.kind === 'Normal');
    }));
    assert.deepEqual(segment.unwind_edges, blocks.flatMap(index => source.blocks[index].successors
      .filter(edge => edge.kind === 'Unwind').map(edge => [index, edge.target])));
  }
  if (name === 'repeated') {
    const point = source.yields[0];
    const resumed = source.segments.find(segment => segment.entry === point.resume);
    assert.ok(resumed.yield_blocks.includes(point.block), 'the loop can reach its own yield again');
    assert.ok(resumed.terminal_blocks.length > 0, 'the loop can also complete');
  }
  let previous = -1;
  for (const point of source.yields) {
    assert.ok(point.liveness, 'source liveness must be captured');
    for (const locals of Object.values(point.liveness)) {
      assert.deepEqual(locals,[...new Set(locals)].sort((a,b)=>a-b));
      assert.ok(locals.every(local=>local >= 0 && local < source.local_count));
    }
    assert.ok(!point.liveness.live.includes(1));
    assert.ok(point.liveness.borrowed.every(local=>point.liveness.live.includes(local)));
    assert.ok(point.block > previous && point.block < source.block_count);
    previous = point.block;
    assert.ok(point.resume < source.block_count);
    assert.deepEqual(source.blocks[point.block].successors, [
      {kind: 'Resume', target: point.resume},
      ...(point.drop === null ? [] : [{kind: 'Cancel', target: point.drop}]),
    ]);
    assert.ok(point.drop === null || point.drop < source.block_count);
    assert.ok(point.resume_local === null || point.resume_local < source.local_count);
    assert.ok(point.source.file.endsWith(fixture === 'local' ? '26_async.rs' : fixture === 'liveness' ? '111_async_source_liveness.rs' : fixture === 'initialization' ? '112_async_initialization.rs' : 'suspension_provider.rs'));
    const sourceFile = new URL(`../${point.source.file}`, import.meta.url);
    const line = readFileSync(sourceFile, 'utf8').split('\n')[point.source.line - 1];
    assert.ok(line.includes('.await'), JSON.stringify(point));
  }
  (found[name] ??= []).push(source);
}
assert.deepEqual(Object.keys(found).sort(), Object.keys(expected).sort());
if (fixture === 'generic') {
  assert.equal(found.relay.length, 2, 'both generic instantiations must retain the same definition facts');
  assert.deepEqual(found.relay[0], found.relay[1]);
}
const fallback = report.items.filter(item => item.lane === 'fallback');
assert.equal(fallback.length, 0, JSON.stringify(fallback));
console.log(JSON.stringify({fixture, found, fallback: fallback.length}, null, 2));
