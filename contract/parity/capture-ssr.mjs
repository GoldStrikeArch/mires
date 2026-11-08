// Captures the SSR bytes each parity fixture runs against.
//
//   node capture-ssr.mjs            # rewrite fixtures/*/ssr.html
//   node capture-ssr.mjs --check    # fail if any of them would change
//
// WHY THE SERVER AND NOT A RENDERING CRATE
// ----------------------------------------
// The brief allowed a small Rust crate under contract/parity/ that renders the
// fixtures' server HTML to stdout. It is not here, on purpose.
//
// A rendering crate would need the `dom` facade feature, a path dep on
// topcoat-view, the `#[island]` macro, and its own copy of the counter's view
// body -- and that copy is the problem. The whole claim this harness makes is
// that ONE view body compiled twice agrees with itself. A second transcription
// of the view in a harness crate would be a third thing that has to be kept in
// step, and the day it drifted the harness would report parity between two
// copies that agree with each other and not with the app.
//
// demo-app already renders the real island through the real macro at the real
// route. So the SSR bytes come from there, and this script needs no build at
// all: it runs the binary demo-app's own build already produced. The capture is
// committed, so `run.mjs` needs neither cargo nor a server.
//
// The tradeoff is that ssr.html can go stale against a rebuilt emitter. That is
// covered, not hoped about: run.mjs asserts the client template string equals the
// server markup, so a captured file that no longer matches the compiled module
// fails the run rather than passing quietly.

import { spawn } from "node:child_process";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { BINARY, SPIKE, captureFreshness } from "./lib/delivery.mjs";

const ORIGIN = "http://127.0.0.1:3000";
const FIXTURES = join(import.meta.dirname, "fixtures");

/** Every fixture directory, in name order. */
export function fixtureNames() {
	return readdirSync(FIXTURES, { withFileTypes: true })
		.filter(entry => entry.isDirectory())
		.map(entry => entry.name)
		.sort();
}

/** A fixture's declaration. */
export function fixture(name) {
	return JSON.parse(readFileSync(join(FIXTURES, name, "fixture.json"), "utf8"));
}

/**
 * Run the already-built demo-app binary, hand it to `body`, and stop it.
 *
 * Deliberately does NOT build. demo-app is another agent's crate and rebuilding
 * it here would race a concurrent edit and could leave a half-built target dir;
 * worse, it would silently capture SSR bytes from a different emitter than the
 * one whose islands.js is on disk.
 */
async function withServer(body) {
	if (!existsSync(BINARY)) {
		throw new Error(`${BINARY} does not exist: build it first (cd demo-app && cargo build)`);
	}

	// REFUSE IF SOMETHING IS ALREADY THERE, because the failure is silent
	// otherwise and it has happened. The loop below spawns the binary and then
	// polls for a 200. If a foreign server already holds the port -- a concurrent
	// `topcoat dev`, a leftover run -- the spawned binary dies instantly with
	// EADDRINUSE while the poll succeeds against the STRANGER, and the capture
	// records its bytes. Observed once: a `topcoat dev` server answered, and its
	// pages carry an injected `<script src="http://127.0.0.1:<random>/dev.js">`
	// (crates/topcoat/src/dev.rs:58-67, gated on TOPCOAT_DEV_URL) whose port
	// changes every run, so `--check` reported eternal drift against a capture
	// that was never wrong. The exitCode guard inside the loop cannot cover it:
	// the guard runs before the fetch, and on the first pass the child has not
	// necessarily exited yet.
	try {
		const response = await fetch(`${ORIGIN}/`);
		throw new Error(
			`something is already serving ${ORIGIN} (answered ${response.status}). This capture would record ITS bytes,\n`
			+ "not demo-app's -- stop the other server and re-run. If it is a `topcoat dev` session, note that its pages\n"
			+ "carry an injected dev.js script tag on a random port, which is not something to capture.",
		);
	} catch (error) {
		// A refused connection is the good case: the port is ours.
		if (error instanceof Error && error.message.startsWith("something is already serving")) throw error;
	}

	const server = spawn(BINARY, { cwd: join(SPIKE, "demo-app"), stdio: ["ignore", "pipe", "pipe"] });
	const log = [];
	server.stdout.on("data", chunk => log.push(chunk));
	server.stderr.on("data", chunk => log.push(chunk));

	try {
		for (let attempt = 0; attempt < 60; attempt++) {
			if (server.exitCode !== null) {
				throw new Error(`the server exited with ${server.exitCode}:\n${Buffer.concat(log)}`);
			}
			try {
				const response = await fetch(`${ORIGIN}/`);
				if (response.ok) return await body();
			} catch {
				// not up yet
			}
			await new Promise(resolve => setTimeout(resolve, 100));
		}
		throw new Error(`the server never answered on ${ORIGIN}:\n${Buffer.concat(log)}`);
	} finally {
		server.kill("SIGTERM");
	}
}

async function main() {
	const check = process.argv.includes("--check");
	const names = fixtureNames();
	if (!names.length) throw new Error(`no fixtures under ${FIXTURES}`);

	// Deliberately a warning and not an error. A stale binary still serves a real
	// page and the capture is still worth having; what it is not worth is trusting
	// on the subject of the page's own delivery, which run.mjs handles by not
	// checking that part and saying so.
	const freshness = captureFreshness();
	if (!freshness.fresh && freshness.binary !== null) {
		console.log(`warning: ${BINARY} is older than ${freshness.stale.length} of its inputs, newest ${freshness.stale[0].file}`);
		console.log("         the capture will be of the OLD server. Rebuild demo-app to capture the current one.");
		console.log("");
	}

	const drift = [];
	await withServer(async () => {
		for (const name of names) {
			const declared = fixture(name);
			// A pending fixture's route does not exist yet, so fetching it would 404
			// and stop the capture of the fixtures that DO work. Skipping it here is
			// what makes a scaffold fixture landable before its island.
			if (declared.pending) {
				console.log(`skip    ${name}  ${declared.route}  PENDING: ${declared.pending.why.split(".")[0]}`);
				continue;
			}
			const response = await fetch(`${ORIGIN}${declared.route}`);
			if (!response.ok) throw new Error(`${declared.route} answered ${response.status}`);
			const html = await response.text();

			const path = join(FIXTURES, name, "ssr.html");
			const before = existsSync(path) ? readFileSync(path, "utf8") : null;
			if (before === html) {
				console.log(`ok      ${name}  ${declared.route}  ${html.length} bytes, unchanged`);
				continue;
			}
			if (check) {
				drift.push(name);
				console.log(`DRIFT   ${name}  ${declared.route}  captured ${before?.length ?? 0} bytes, server now sends ${html.length}`);
				continue;
			}
			writeFileSync(path, html);
			console.log(`written ${name}  ${declared.route}  ${html.length} bytes${before === null ? " (new)" : " (changed)"}`);
		}
	});

	if (drift.length) {
		console.log("");
		console.log(`${drift.length} fixture(s) drifted: re-run without --check to update, then re-run the parity check`);
		process.exit(1);
	}
}

if (import.meta.filename === process.argv[1]) {
	await main();
}
