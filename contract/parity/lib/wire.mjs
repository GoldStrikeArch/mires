// The procedure wire, checked against the vectors that specify it.
//
// A fixture whose island calls a procedure has to stub the call, and a stub is a
// second statement of the wire. `contract/fixtures/procedure-wire.json` is the
// first one, and the wave-2 notes record what happens when two statements of one
// wire drift: the argument-index path in the error message was written `1` here
// and rendered `[1]` by the server, and only the framework agent's real HTTP test
// could tell. The lesson written down then was that a citation is only as strong as
// what the cited test actually asserts.
//
// So the fixture does not restate the wire. It NAMES the vectors it is honouring,
// and this file checks that the values it plans to send and expect are the ones
// those vectors carry. If a vector moves -- as one already has -- the fixture fails
// with the name of the vector rather than sending a stale body into a stub that
// happily accepts it.

import { readFileSync } from "node:fs";
import { join } from "node:path";

import { SPIKE } from "./delivery.mjs";

/** The wire spec. */
export function wireSpec() {
	return JSON.parse(readFileSync(join(SPIKE, "contract", "fixtures", "procedure-wire.json"), "utf8"));
}

/**
 * Check a fixture's `wire` block against the vectors it names.
 *
 * @param {object} declared the fixture's `wire` block
 * @returns {{differences: string[], vectors: Record<string, object>, notes: string[]}}
 */
export function wireParity(declared) {
	const spec = wireSpec();
	const differences = [];
	const notes = [];
	const byId = new Map(spec.vectors.map(vector => [vector.id, vector]));

	// Every named vector has to exist. A fixture naming a vector that was renamed
	// or removed is the most likely way this goes stale, and it is silent unless
	// asked.
	const vectors = {};
	for (const [role, id] of Object.entries(declared.vectors)) {
		const vector = byId.get(id);
		if (!vector) {
			differences.push(`the fixture's ${role} names vector ${JSON.stringify(id)}, which procedure-wire.json does not have`);
			continue;
		}
		vectors[role] = vector;
	}

	const wire = spec.wires[declared.wire];
	if (!wire) {
		differences.push(`the fixture declares wire ${JSON.stringify(declared.wire)}, which procedure-wire.json does not describe`);
		return { differences, vectors, notes };
	}

	// The three values a client can get wrong on its own, each read from the spec
	// rather than repeated here.
	if (declared.requestContentType !== wire.requestContentType) {
		differences.push(`request content type: fixture says ${JSON.stringify(declared.requestContentType)}, the ${declared.wire} wire is ${JSON.stringify(wire.requestContentType)}`);
	}
	if (declared.responseContentType !== wire.responseContentType) {
		differences.push(`response content type: fixture says ${JSON.stringify(declared.responseContentType)}, the ${declared.wire} wire is ${JSON.stringify(wire.responseContentType)}`);
	}
	if (declared.zeroArguments !== wire.zeroArguments) {
		differences.push(`zero-argument body: fixture says ${JSON.stringify(declared.zeroArguments)}, the ${declared.wire} wire is ${JSON.stringify(wire.zeroArguments)}`);
	}

	// The route, which the fixture asserts the island's call against.
	if (declared.routePrefix !== spec.route.prefix) {
		differences.push(`route prefix: fixture says ${JSON.stringify(declared.routePrefix)}, procedure-wire.json says ${JSON.stringify(spec.route.prefix)}`);
	}
	if (declared.method !== spec.route.method) {
		differences.push(`method: fixture says ${JSON.stringify(declared.method)}, procedure-wire.json says ${JSON.stringify(spec.route.method)}`);
	}

	// The one argument the island actually sends must be the shape the one-argument
	// vector pins: an ARRAY even for a single value, because the tuple is `(T,)`.
	// This is the single most likely thing for a hand-written client to get wrong.
	if (vectors.oneArgument) {
		const body = vectors.oneArgument.body;
		if (!body.startsWith("[") || !body.endsWith("]")) {
			differences.push(`the oneArgument vector's body ${JSON.stringify(body)} is not an array, so the fixture's assumption that one argument is still wrapped is wrong`);
		}
	}

	// The error path, whose exact spelling is the thing that has already been wrong
	// once.
	//
	// VALIDATE THE RULE, THEN APPLY IT -- rather than copying the vector's literal
	// suffix, which would be wrong here for a reason worth writing down. The vector
	// pins a TWO-argument procedure whose second argument is bad, so its suffix is
	// ``(at `[1]`)``. The search island's procedure takes ONE argument, so a bad
	// argument is index 0 and the real server answers ``(at `[0]`)`` -- measured live
	// by the framework agent against this very vector. Same rule, different index.
	//
	// So the check is two-sided: the spec's own `$pathRule` is first confirmed to
	// reproduce the vector's recorded suffix (which is what stops the rule itself
	// drifting), and only then applied to this fixture's argument index. Copying the
	// literal would have made the fixture stub an error the server never sends.
	const pathSuffix = index => `(at \`[${index}]\`)`;
	if (vectors.error) {
		const expect = vectors.error.expect;
		if (declared.errorBodyStartsWith !== expect.bodyStartsWith) {
			differences.push(`error prefix: fixture says ${JSON.stringify(declared.errorBodyStartsWith)}, vector ${vectors.error.id} says ${JSON.stringify(expect.bodyStartsWith)}`);
		}
		if (declared.errorStatus !== expect.status) {
			differences.push(`error status: fixture says ${declared.errorStatus}, vector ${vectors.error.id} says ${expect.status}`);
		}

		// The rule against the vector. `errorVectorArgumentIndex` is the index the
		// VECTOR's own bad argument sits at, so this is a closed loop with no free
		// parameter: if the rule and the recorded suffix disagree, one of them moved.
		const rebuilt = pathSuffix(declared.errorVectorArgumentIndex);
		if (rebuilt !== expect.bodyEndsWith) {
			differences.push(
				`the path rule does not reproduce vector ${vectors.error.id}'s own suffix: index ${declared.errorVectorArgumentIndex} renders ${JSON.stringify(rebuilt)}`
				+ ` but the vector records ${JSON.stringify(expect.bodyEndsWith)}. Either the rule changed or the vector did; do not fix this by editing the fixture.`,
			);
		}
		// The rule applied to THIS fixture's procedure.
		const mine = pathSuffix(declared.errorArgumentIndex);
		if (declared.errorBodyEndsWith !== mine) {
			differences.push(`error suffix: fixture says ${JSON.stringify(declared.errorBodyEndsWith)}, but the path rule at argument index ${declared.errorArgumentIndex} renders ${JSON.stringify(mine)}`);
		}
		if (vectors.error.$corrected) {
			notes.push(`vector ${vectors.error.id} carries $corrected: its rendered argument path was predicted wrong once (\`1\`) and observed as \`[1]\`. That is why this fixture derives the spelling from the spec's $pathRule instead of writing it out.`);
		}
	}

	// The call target. Two shapes, because a compiled island cannot always use the
	// procedure route: a procedure's id is a uuid minted at MACRO-EXPANSION time and
	// an island's file is expanded twice, once per crate, so the two expansions mint
	// DIFFERENT ids and a procedure declared in a shared file cannot address itself
	// from the client. demo-app's answer is a written-down route serving the same
	// wire. That is a real finding about the island model, not a shortcut, so the
	// fixture records which shape it targets rather than assuming the procedure one.
	if (declared.callTarget) {
		const kinds = ["procedureRoute", "exactPath"];
		if (!kinds.includes(declared.callTarget.kind)) {
			differences.push(`callTarget.kind is ${JSON.stringify(declared.callTarget.kind)}, which is not one of ${kinds.join(", ")}`);
		}
		if (declared.callTarget.kind === "exactPath" && typeof declared.callTarget.path !== "string") {
			differences.push("callTarget.kind is exactPath but no `path` is declared, so the fixture cannot say what URL it expects");
		}
	}

	// Errors are plain text with no envelope, which is what makes "the island
	// survived a procedure error" a meaningful assertion: the client cannot have
	// parsed a structured error, so whatever it did with the failure it did with a
	// string.
	if (declared.errorContentType !== "text/plain") {
		differences.push(`error content type: fixture says ${JSON.stringify(declared.errorContentType)}, procedure-wire.json's errors block says text/plain with no JSON envelope`);
	}

	return { differences, vectors, notes };
}

/**
 * Build the fetch stub a procedure-calling fixture drives its island with.
 *
 * A STUB AND NOT A SERVER, and the difference is the point: the fixture is
 * measuring the island's client half -- when it calls, what it sends, and what it
 * does with the answer -- and a real server would put the framework's own handler
 * between the island and the assertion. The bodies the stub returns are the ones
 * `procedure-wire.json` says a real handler produces, checked by `wireParity`
 * above, so the stub is the spec rather than a convenient approximation of it.
 *
 * Every call is recorded before it is answered, so a fixture can assert what was
 * sent as well as what came back. `queue` is consumed in order; when it runs out
 * the stub throws rather than inventing a reply, because a fixture that made one
 * more call than it planned has learned something and should not be told it passed.
 *
 * @param {object} wire the fixture's checked `wire` block
 * @param {{status: number, contentType: string, body: string}[]} queue replies, in order
 * @returns {{fetch: Function, calls: object[], pending: number}}
 */
export function fetchStub(wire, queue) {
	const calls = [];
	const replies = [...queue];

	async function stub(input, init = {}) {
		const url = typeof input === "string" ? input : input.url;
		const headers = new Map(Object.entries(init.headers ?? {}).map(([key, value]) => [key.toLowerCase(), value]));
		const call = {
			url,
			method: init.method ?? "GET",
			contentType: headers.get("content-type") ?? null,
			body: init.body ?? null,
			// Parsed here so a fixture asserts the DECODED arguments rather than a
			// byte string, which would make it fail on a harmless whitespace change.
			args: parseArgs(init.body),
			// The route shape, checked rather than the literal id: a procedure's id is
			// a uuid minted at macro-expansion time, so it is not knowable from here
			// and not stable across rebuilds (procedure-wire.json `route`).
			isProcedureRoute: typeof url === "string" && url.startsWith(`${wire.routePrefix}/`) && url.slice(wire.routePrefix.length + 1).length > 0 && !url.slice(wire.routePrefix.length + 1).includes("/"),
			// Whether it went where the fixture says the island calls, which for a
			// written-down route is an exact match and for the procedure route is the
			// shape above.
			onTarget: wire.callTarget?.kind === "exactPath" ? url === wire.callTarget.path : undefined,
		};
		call.onTarget ??= call.isProcedureRoute;
		calls.push(call);

		const reply = replies.shift();
		if (!reply) {
			throw new Error(`the island made ${calls.length} call(s) and the fixture queued ${queue.length}: ${call.method} ${url} ${call.body}`);
		}
		return {
			ok: reply.status >= 200 && reply.status < 300,
			status: reply.status,
			statusText: reply.status === 200 ? "OK" : "Bad Request",
			headers: { get: name => (name.toLowerCase() === "content-type" ? reply.contentType : null) },
			async text() {
				return reply.body;
			},
			async json() {
				return JSON.parse(reply.body);
			},
		};
	}

	return {
		fetch: stub,
		calls,
		get pending() {
			return replies.length;
		},
	};
}

function parseArgs(body) {
	if (typeof body !== "string") return null;
	try {
		return JSON.parse(body);
	} catch {
		return null;
	}
}
