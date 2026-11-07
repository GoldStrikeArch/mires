#!/usr/bin/env node
//
// Checks a source map the backend wrote, without a browser.
//
//   node scripts/check-sourcemap.js <out.js> [expected-source-substring]
//
// Stepping through Rust in DevTools cannot be automated here, so this asserts
// everything a debugger would need before it could work:
//
//   * the .js ends with a sourceMappingURL comment naming a file that exists;
//   * the .map parses as JSON and has the version 3 shape;
//   * every VLQ segment decodes, and every field it points at is in range;
//   * the generated positions are inside the .js, line by line;
//   * the original positions are inside the source they name, whenever the map
//     carries that source's text;
//   * every generated line is mapped, which is what makes a debugger's
//     "step to the next line" land somewhere;
//   * the sources named in ignoreList are only ever the generated glue.
//
// Prints a summary and exits non-zero on the first thing that is wrong.

'use strict';

const fs = require('fs');
const path = require('path');

const BASE64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function fail(message) {
	console.error(`check-sourcemap: ${message}`);
	process.exit(1);
}

function decodeLine(text, where) {
	const fields = [];
	let value = 0;
	let shift = 0;
	let started = false;
	for (const ch of text) {
		const digit = BASE64.indexOf(ch);
		if (digit < 0) fail(`${where}: '${ch}' is not a base 64 digit`);
		value |= (digit & 31) << shift;
		shift += 5;
		started = true;
		if ((digit & 32) === 0) {
			const negative = value & 1;
			const magnitude = value >>> 1;
			fields.push(negative ? -magnitude : magnitude);
			value = 0;
			shift = 0;
			started = false;
		}
	}
	if (started) fail(`${where}: a VLQ value was left unterminated`);
	return fields;
}

const jsPath = process.argv[2];
const wanted = process.argv[3];
if (!jsPath) fail('usage: check-sourcemap.js <out.js> [expected-source-substring]');

const js = fs.readFileSync(jsPath, 'utf8');
const jsLines = js.split('\n');
if (js.endsWith('\n')) jsLines.pop();

const urlMatch = js.match(/\/\/# sourceMappingURL=(.*)\s*$/);
if (!urlMatch) fail(`${jsPath} has no sourceMappingURL comment`);
const mapPath = path.join(path.dirname(jsPath), urlMatch[1].trim());
if (!fs.existsSync(mapPath)) fail(`${mapPath} does not exist`);

const map = JSON.parse(fs.readFileSync(mapPath, 'utf8'));
if (map.version !== 3) fail(`${mapPath}: version is ${map.version}, not 3`);
for (const field of ['sources', 'names', 'mappings']) {
	if (!(field in map)) fail(`${mapPath}: no \`${field}\``);
}
if (map.sourcesContent && map.sourcesContent.length !== map.sources.length) {
	fail(`${mapPath}: sourcesContent has ${map.sourcesContent.length} entries for ${map.sources.length} sources`);
}

// The line count of every source whose text the map carries, for the range check.
const sourceLines = (map.sources || []).map((_, i) => {
	const content = map.sourcesContent ? map.sourcesContent[i] : null;
	return typeof content === 'string' ? content.split('\n').length : null;
});

let source = 0;
let srcLine = 0;
let srcCol = 0;
let name = 0;
let total = 0;
let named = 0;
const perSource = new Map();
const unmapped = [];

const lines = map.mappings.split(';');
if (lines.length > jsLines.length) {
	fail(`${mapPath}: ${lines.length} mapped lines for a ${jsLines.length} line file`);
}

lines.forEach((line, genLine) => {
	if (line === '') {
		unmapped.push(genLine + 1);
		return;
	}
	let genCol = 0;
	for (const segment of line.split(',')) {
		const fields = decodeLine(segment, `${mapPath}:${genLine + 1}`);
		if (![1, 4, 5].includes(fields.length)) {
			fail(`${mapPath}:${genLine + 1}: a segment has ${fields.length} fields`);
		}
		genCol += fields[0];
		if (genCol < 0) fail(`${mapPath}:${genLine + 1}: negative generated column`);
		if (genCol > jsLines[genLine].length) {
			fail(`${mapPath}:${genLine + 1}: column ${genCol} is past the end of a ${jsLines[genLine].length} character line`);
		}
		if (fields.length === 1) continue;
		source += fields[1];
		srcLine += fields[2];
		srcCol += fields[3];
		if (source < 0 || source >= map.sources.length) {
			fail(`${mapPath}:${genLine + 1}: source index ${source} is out of range`);
		}
		if (srcLine < 0 || srcCol < 0) {
			fail(`${mapPath}:${genLine + 1}: negative original position`);
		}
		const limit = sourceLines[source];
		if (limit !== null && srcLine >= limit) {
			fail(`${mapPath}:${genLine + 1}: ${map.sources[source]}:${srcLine + 1} is past the end of a ${limit} line file`);
		}
		if (fields.length === 5) {
			name += fields[4];
			if (name < 0 || name >= map.names.length) {
				fail(`${mapPath}:${genLine + 1}: name index ${name} is out of range`);
			}
			named++;
		}
		total++;
		perSource.set(map.sources[source], (perSource.get(map.sources[source]) || 0) + 1);
	}
});

if (unmapped.length > 0) {
	fail(`${mapPath}: ${unmapped.length} generated lines have no mapping (first: ${unmapped[0]})`);
}

for (const index of map.ignoreList || []) {
	if (!map.sources[index] || !map.sources[index].includes('generated')) {
		fail(`${mapPath}: ignoreList names ${map.sources[index]}, which is not generated glue`);
	}
}

if (wanted) {
	const hit = [...perSource.entries()].find(([s]) => s.includes(wanted));
	if (!hit) fail(`${mapPath}: nothing maps to a source matching \`${wanted}\` (sources: ${map.sources.join(', ')})`);
	if (hit[1] < 5) fail(`${mapPath}: only ${hit[1]} mappings reach \`${hit[0]}\``);
}

const rust = [...perSource.entries()].filter(([s]) => s.endsWith('.rs'));
console.log(`${path.basename(mapPath)}: ${total} mappings, ${named} named, ${map.sources.length} sources, ${map.names.length} names`);
for (const [file, count] of rust.sort((a, b) => b[1] - a[1])) {
	const index = map.sources.indexOf(file);
	const embedded = map.sourcesContent && typeof map.sourcesContent[index] === 'string';
	console.log(`  ${count.toString().padStart(5)}  ${file}${embedded ? '' : '  (no sourcesContent)'}`);
}
