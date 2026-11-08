// Check `fixtures/procedure-wire.json` against the source it claims to describe.
//
//   node --import ./register-loader.mjs check-procedure-wire.mjs
//
// WHY THIS EXISTS
// ---------------
// Every other fixture in this directory is REGENERATED from pinned upstream, so
// `run-all.mjs --check` catches drift by re-deriving it. `procedure-wire.json`
// cannot work that way: the procedure wire is Topcoat's own, there is no upstream
// to extract it from, and the framework crate that implements it is outside this
// harness's ownership. So the file is hand-written -- which would normally mean
// it is free to rot.
//
// This closes that hole from the other side. Instead of regenerating the file,
// re-read the SOURCE LINES it cites and assert the load-bearing values still say
// what the file says they say. A framework agent who changes the media type, the
// route prefix, the accepted method or the zero-argument spelling now breaks the
// contract's own drift gate, which is the point: the spec is the shared source
// for the server's HTTP tests and the client's fixtures, and a shared source that
// can silently disagree with the code is worse than no shared source.
//
// It deliberately does NOT compile or run anything. It reads text.

import { readFileSync, existsSync } from "node:fs";
import path from "node:path";

import { CONTRACT, FIXTURES } from "./paths.mjs";

/** The repository root: contract/ -> spike/rustc-codegen-js -> spike -> root. */
const ROOT = path.resolve(CONTRACT, "..", "..", "..");

const WIRE = path.join(FIXTURES, "procedure-wire.json");

/** Read a repo-relative source file, or fail loudly naming it. */
function source(rel) {
	const full = path.join(ROOT, rel);
	if (!existsSync(full)) throw new Error(`the wire spec cites ${rel}, which does not exist`);
	return readFileSync(full, "utf8");
}

const failures = [];
const checked = [];

function check(what, condition, detail) {
	checked.push(what);
	if (!condition) failures.push(detail ? `${what}\n    ${detail}` : what);
}

const spec = JSON.parse(readFileSync(WIRE, "utf8"));

// ---------- the values the two sides have to agree on ----------

const runtime = source("crates/topcoat-runtime/src/procedure.rs");
const grammar = source("crates/topcoat-runtime/grammar/src/procedure.rs");
const browser = source("crates/topcoat-runtime/browser/src/surrogate/procedure.ts");
const json = source("crates/topcoat-router/src/content/json.rs");
const badRequest = source("crates/topcoat-router/src/error/bad_request.rs");
const routerError = source("crates/topcoat-router/src/error.rs");

const prefix = runtime.match(/const PROCEDURE_ROUTE_PREFIX: &str = "([^"]+)"/)?.[1];
check(
	`the route prefix is ${JSON.stringify(spec.route.prefix)}`,
	prefix === spec.route.prefix,
	`source says ${JSON.stringify(prefix)}`,
);

const serdeType = runtime.match(/pub const SERDE_CONTENT_TYPE: &str = "([^"]+)"/)?.[1];
check(
	`the serde media type is ${JSON.stringify(spec.wires.serde.requestContentType)}`,
	serdeType === spec.wires.serde.requestContentType,
	`source says ${JSON.stringify(serdeType)}`,
);

check(
	"a procedure route accepts POST and nothing else",
	/Methods::Only\(&\[Method::POST\]\)/.test(runtime) && spec.route.method === "POST",
);

check(
	"the procedure id is a v4 uuid minted at macro-expansion time",
	/Uuid::new_v4\(\)/.test(grammar),
	"grammar/src/procedure.rs no longer mints the id that way, so the spec's note about ids not surviving a rebuild may be wrong",
);

check(
	"the serde wire's zero-argument form decodes an empty array, not a unit",
	/\[\(\); 0\]/.test(runtime) && spec.wires.serde.zeroArguments === "[]",
);

check(
	`the surrogate wire sends ${JSON.stringify(spec.wires.surrogate.zeroArguments)} for zero arguments`,
	/args\.length === 0\s*\?\s*null/.test(browser) && spec.wires.surrogate.zeroArguments === "null",
);

check(
	`the surrogate wire sends ${JSON.stringify(spec.wires.surrogate.requestContentType)}`,
	browser.includes(`"Content-Type": "${spec.wires.surrogate.requestContentType}"`),
);

check(
	"the surrogate wire posts to the route prefix the spec names",
	browser.includes(`\`${spec.route.prefix}/\${encodeURIComponent(this.id)}\``),
);

check(
	"the response media type is a static application/json set by Json's IntoResponse",
	/APPLICATION_JSON|application\/json/.test(json) && spec.wires.serde.responseContentType === "application/json",
);

check(
	"the Json extractor accepts any application/*+json, which is what lets the serde handler reuse it",
	/ends_with\("\+json"\)/.test(json) && !!spec.mediaTypeMatching.$jsonExtractorIsWider,
);

check(
	"a wrong media type is a 400 and not a 415",
	/StatusCode::BAD_REQUEST/.test(badRequest) && spec.errors.wrongMediaType.status === 400,
);

check(
	"no 415 constructor has appeared, which is the premise of deviation D2",
	!/UNSUPPORTED_MEDIA_TYPE/.test(routerError),
	"a 415 exists now, so the spec's D2 rationale needs revisiting",
);

check(
	"a bad request renders as `bad request: {description}` in plain text",
	badRequest.includes('write!(f, "bad request: {} (at `{path}`)"') && badRequest.includes('write!(f, "bad request: {}"'),
);

check(
	"the decode error prefixes the spec asserts are the ones the source builds",
	json.includes('format!("invalid JSON value: {}"') && json.includes('format!("invalid JSON syntax: {}"'),
);

const guard = runtime.match(/expected request with `Content-Type: \{SERDE_CONTENT_TYPE\}`/);
check(
	"the wrong-media-type message is the one the spec quotes exactly",
	!!guard && spec.errors.wrongMediaType.bodyExactly === `bad request: expected request with \`Content-Type: ${serdeType}\``,
);

// ---------- the file's own internal consistency ----------

const ids = spec.vectors.map(vector => vector.id);
check("every vector id is unique", new Set(ids).size === ids.length);

for (const vector of spec.vectors) {
	if (typeof vector.body === "string" && vector.bodyIsValidJson !== false) {
		let ok = true;
		try {
			JSON.parse(vector.body);
		} catch {
			ok = false;
		}
		check(`vector ${vector.id}'s body is valid JSON`, ok, JSON.stringify(vector.body));
	}
	if (vector.expect && !("status" in vector.expect)) {
		check(`vector ${vector.id} expects a status`, false);
	}
}

// The media-type rule, reimplemented from the Rust so the accept/reject lists in
// the spec are checked rather than asserted.
const accepts = value => {
	if (value === null || value === undefined) return false;
	return value.split(";")[0].trim().toLowerCase() === serdeType.toLowerCase();
};
for (const value of spec.mediaTypeMatching.serdeAccepts) {
	check(`the serde guard accepts ${JSON.stringify(value)}`, accepts(value));
}
for (const value of spec.mediaTypeMatching.serdeRejects) {
	check(`the serde guard rejects ${JSON.stringify(value)}`, !accepts(value));
}

// Every request vector's content type must land on the side its expectation says.
for (const vector of spec.vectors.filter(entry => entry.direction === "request" && entry.wire === "serde")) {
	if (!("contentType" in vector)) continue;
	const rejected = vector.expect.bodyExactly === spec.errors.wrongMediaType.bodyExactly;
	check(
		`vector ${vector.id}'s content type is on the side its expectation claims`,
		accepts(vector.contentType) === !rejected,
	);
}

// ---------- agreement with the copy the framework crate tests against ----------
//
// The framework crate distils these vectors into its own fixture and runs real HTTP
// tests against them. That makes it the only consumer that can find out the wire does
// something this file merely predicted, so corrections flow FROM there TO here. Two
// things have to hold, and the second is the one with teeth: the two files must agree
// vector for vector, and no correction the downstream copy reports may be left
// unfolded. Without that second check a copy could carry an observed value for a
// release while this file kept telling wave 3's client fixtures the wrong thing, and
// both would be green.
const DISTILLED = "crates/topcoat-runtime/macro/tests/fixtures/procedure_wire.json";
if (!existsSync(path.join(ROOT, DISTILLED))) {
	// Not a failure: the framework crate may legitimately not have distilled it yet.
	console.log(`  note: ${DISTILLED} does not exist yet, so the agreement checks are skipped`);
} else {
	const distilled = JSON.parse(source(DISTILLED));
	const ours = new Map(spec.vectors.map(vector => [vector.id, vector]));
	const theirs = new Map(distilled.vectors.map(vector => [vector.id, vector]));

	check(
		"the distilled copy carries the same vector ids",
		ours.size === theirs.size && [...ours.keys()].every(id => theirs.has(id)),
		`ours ${ours.size}, theirs ${theirs.size}`,
	);

	for (const [id, vector] of theirs) {
		const mine = ours.get(id);
		if (!mine) continue;
		check(
			`vector ${id}'s expectation is the same in both files`,
			JSON.stringify(mine.expect) === JSON.stringify(vector.expect),
			`ours ${JSON.stringify(mine.expect)}\n    theirs ${JSON.stringify(vector.expect)}`,
		);
	}

	// A correction is folded in when this file no longer says what was `claimed`.
	for (const correction of distilled.$corrections ?? []) {
		const claimed = correction.claimed ?? "";
		const stillClaimed = claimed && JSON.stringify(spec.vectors.find(v => v.id === correction.vector) ?? {}).includes(
			// "expect.bodyEndsWith = \"(at `1`)\"" -> the quoted value
			claimed.match(/"([^"]*)"\s*$/)?.[1] ?? " ",
		);
		check(
			`the correction reported for ${correction.vector} has been folded in`,
			!stillClaimed,
			`the downstream copy observed ${correction.observed}, and this file still says ${claimed}`,
		);
	}
}

// Every cited file exists (the citation map is the reason this file is trustworthy).
for (const [key, citation] of Object.entries(spec.$citations)) {
	const rel = citation.split(/[\s:(]/)[0];
	if (!rel.includes("/")) continue;
	check(`the citation for ${key} names a file that exists`, existsSync(path.join(ROOT, rel)), rel);
}

// ---------- report ----------

console.log(`procedure wire: ${checked.length} checks over ${spec.vectors.length} vectors`);
const derived = spec.vectors.filter(vector => vector.derived).length;
console.log(`  ${spec.vectors.length - derived} vector(s) pinned by an existing test, ${derived} derived from the cited code`);
if (failures.length) {
	console.log("");
	for (const failure of failures) console.log(`  FAIL  ${failure}`);
	console.log("");
	console.log(`${failures.length} check(s) failed: the wire spec and the source disagree`);
	process.exit(1);
}
console.log("  the spec agrees with every source value it cites");
