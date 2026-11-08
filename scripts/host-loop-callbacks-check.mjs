// Checks examples/host_loop_callbacks.rs: closures created inside `for` loop bodies and handed to
// the host keep their own turn's captures (CONTRACT.md, "A thunk built inside a loop snapshots its
// environment").
//
//   node scripts/host-loop-callbacks-check.mjs <program.mjs> [--snapshot]
//
// Two halves, and both must pass.
//
// BEHAVIOUR. Every exported function builds its callbacks in a loop and returns them only after the
// loop has ended. Each one is called afterwards, out of creation order and more than once, and must
// answer its own turn's values. A thunk that closed over the loop's reused binding would answer the
// last turn's value for every row instead.
//
// SHAPE. The emitted JavaScript is parsed and every function an exported loop function contains is
// classified. A function created on a loop that escapes (is not called on the spot) and reads a
// binding of the enclosing function is exactly the defect: the binding is declared once in the
// prelude and reassigned on every turn. The contract's cure is an immediately invoked wrapper that
// takes the environment as its parameter,
//
//   (($c) => ($a0) => name($c, $a0))(env)
//
// so an escaping function on a loop is accepted only as the direct body of such a wrapper. Under
// `--snapshot` the program must also contain that wrapper, spelled as the contract spells it, at
// every loop site: the build passed here has the callback marker in the loop body itself (MIR
// inlining puts it there), so a missing wrapper is a missing snapshot, not a different shape.
// Without the flag the marker may sit in a callee's own frame, which snapshots by being a call, and
// only the "no escaping function reads a reused binding" half applies.

import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import * as acorn from 'acorn';

const program = resolve(process.argv[2]);
const snapshot = process.argv.includes('--snapshot');
const loopFunctions = ['by_value', 'by_reference', 'nested', 'counting', 'running'];
let checks = 0;
const ok = (value, message) => {
  assert.ok(value, message);
  checks += 1;
};
const equal = (actual, expected, message) => {
  assert.deepEqual(actual, expected, message);
  checks += 1;
};

// ------------------------------------------------------------------------------- behaviour

const api = await import(pathToFileURL(program));
const reversed = (length) => Array.from({ length }, (_, index) => length - 1 - index);
const callbacks = (value, length, arity, label) => {
  ok(Array.isArray(value), `${label}: an array of callbacks`);
  equal(value.length, length, `${label}: one callback per turn`);
  for (const callback of value) {
    equal(typeof callback, 'function', `${label}: each element is a function`);
    equal(callback.length, arity, `${label}: callback arity`);
  }
  return value;
};

{
  const made = callbacks(api.by_value(5), 5, 0, 'by_value');
  for (const order of [reversed(5), [0, 1, 2, 3, 4], [2, 2, 0]]) {
    for (const index of order) equal(made[index](), index, `by_value[${index}]`);
  }
  callbacks(api.by_value(0), 0, 0, 'by_value(0)');
}
{
  const rows = [10, 20, 30, 40];
  const made = callbacks(api.by_reference(), 4, 1, 'by_reference');
  for (const index of reversed(4)) equal(made[index](7), rows[index] + 7, `by_reference[${index}](7)`);
  for (const index of [0, 3]) equal(made[index](0), rows[index], `by_reference[${index}](0)`);
}
{
  const made = callbacks(api.nested(3, 4), 12, 0, 'nested');
  for (const slot of reversed(12)) {
    const row = Math.floor(slot / 4), column = slot % 4;
    equal(made[slot](), row * 100 + column, `nested[${row}][${column}]`);
  }
}
{
  const counter = { count: 0 };
  const made = callbacks(api.counting(counter, 6), 6, 0, 'counting');
  equal(counter.count, 0, 'counting: creating the callbacks runs none of them');
  for (const index of reversed(6)) equal(made[index](), index, `counting[${index}] answers its turn`);
  equal(counter.count, 0 + 1 + 2 + 3 + 4 + 5, 'counting: every turn added its own index once');
  equal(made[5](), 5, 'counting[5] again');
  equal(made[0](), 0, 'counting[0] again');
  equal(counter.count, 20, 'counting: the counter is the caller\'s object, mutated in place');
}
{
  const made = callbacks(api.running(5), 5, 0, 'running');
  equal(made.map((callback) => callback()).reverse(), [15, 10, 6, 3, 1], 'running totals, last first');
  equal(made.map((callback) => callback()), [1, 3, 6, 10, 15], 'running totals, in turn order');
}

// ----------------------------------------------------------------------------------- shape

const ast = acorn.parse(readFileSync(program, 'utf8'), { ecmaVersion: 'latest', sourceType: 'module' });
const isFunction = (node) => node?.type === 'ArrowFunctionExpression' || node?.type === 'FunctionExpression';
const isLoop = (node) => /^(For|ForIn|ForOf|While|DoWhile)Statement$/.test(node.type);

// The names a function declares: its parameters and every `let`/`const`/`var` in its own body.
const declared = (fn) => {
  const names = new Set();
  const pattern = (node) => {
    if (!node) return;
    if (node.type === 'Identifier') names.add(node.name);
    else if (node.type === 'AssignmentPattern') pattern(node.left);
    else if (node.type === 'RestElement') pattern(node.argument);
    else if (node.type === 'ArrayPattern') node.elements.forEach(pattern);
    else if (node.type === 'ObjectPattern') node.properties.forEach((p) => pattern(p.value ?? p.argument));
  };
  fn.params.forEach(pattern);
  walk(fn.body, (node) => {
    if (node !== fn.body && (isFunction(node) || node.type === 'FunctionDeclaration')) return false;
    if (node.type === 'VariableDeclarator') pattern(node.id);
    if (node.type === 'CatchClause') pattern(node.param);
    return true;
  });
  return names;
};

// The identifiers an expression reads, not counting property names.
const reads = (root) => {
  const names = new Set();
  walk(root, (node, parent) => {
    if (node.type !== 'Identifier') return true;
    if (parent?.type === 'MemberExpression' && parent.property === node && !parent.computed) return true;
    if (parent?.type === 'Property' && parent.key === node && !parent.computed && !parent.shorthand) return true;
    names.add(node.name);
    return true;
  });
  return names;
};

function walk(node, visit, parent = null) {
  if (!node || typeof node.type !== 'string') return;
  if (visit(node, parent) === false) return;
  for (const key of Object.keys(node)) {
    if (key === 'type' || key === 'start' || key === 'end') continue;
    const child = node[key];
    if (Array.isArray(child)) child.forEach((item) => walk(item, visit, node));
    else if (child && typeof child.type === 'string') walk(child, visit, node);
  }
}

const exported = new Map();
for (const node of ast.body) {
  if (node.type === 'FunctionDeclaration' && loopFunctions.includes(node.id.name)) exported.set(node.id.name, node);
}
equal([...exported.keys()].sort(), [...loopFunctions].sort(), 'every loop function is emitted under its own name');

// The contract's spelling: `(($c) => ($a0, ..) => name($c, $a0, ..))(env)`.
const contractSpelling = (wrapper) => {
  const thunk = wrapper.body;
  if (wrapper.params.length !== 1 || wrapper.params[0].name !== '$c') return false;
  if (thunk.type !== 'ArrowFunctionExpression' || thunk.body.type !== 'CallExpression') return false;
  const forwarded = thunk.params.map((param, index) => param.type === 'Identifier' && param.name === `$a${index}`);
  const args = thunk.body.arguments.map((arg) => arg.type === 'Identifier' ? arg.name : null);
  return forwarded.every(Boolean) && args[0] === '$c' &&
    args.slice(1).join() === thunk.params.map((param) => param.name).join();
};

const summary = {};
for (const [name, fn] of exported) {
  const locals = declared(fn);
  const wrappers = [], bare = [];
  const visit = (node, parent, depth) => {
    if (!node || typeof node.type !== 'string') return;
    const inLoop = depth > 0;
    if (isFunction(node)) {
      const invoked = parent?.type === 'CallExpression' && parent.callee === node;
      const wrapped = isFunction(parent) && parent.body === node && parent.__invoked;
      if (inLoop && invoked && isFunction(node.body)) wrappers.push(node);
      if (inLoop && !invoked && !wrapped) {
        const own = declared(node);
        const free = [...reads(node.body)].filter((id) => locals.has(id) && !own.has(id));
        if (free.length > 0) bare.push({ at: node.start, free });
      }
      node.__invoked = invoked;
    }
    const nested = depth + (isLoop(node) ? 1 : 0);
    for (const key of Object.keys(node)) {
      if (key === 'type' || key === 'start' || key === 'end' || key === '__invoked') continue;
      const child = node[key];
      if (Array.isArray(child)) child.forEach((item) => visit(item, node, nested));
      else if (child && typeof child.type === 'string') visit(child, node, nested);
    }
  };
  visit(fn.body, fn, 0);
  equal(bare, [], `${name}: no function created on a loop escapes over a reused binding`);
  if (snapshot) {
    ok(wrappers.length >= 1, `${name}: the loop's callback takes its environment through an immediately invoked wrapper`);
    ok(wrappers.every(contractSpelling), `${name}: the wrapper is spelled (($c) => ($a0..) => body($c, $a0..))(env)`);
  }
  summary[name] = wrappers.length;
}

writeFileSync(`${program}.check.json`, JSON.stringify({ passed: true, checks, snapshot, wrappers: summary }) + '\n');
console.log(`host loop callbacks: ${checks} checks passed (${snapshot ? 'snapshot wrappers required' : 'shape: no escaping loop thunk'}; wrappers ${JSON.stringify(summary)})`);
