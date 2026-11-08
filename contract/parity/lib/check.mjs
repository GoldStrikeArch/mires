// A check list that reports every result, not just the first failure.
//
// A parity run's value is the whole picture: "hydration claimed the server's
// node but mutated it twice" and "hydration did not claim the server's node at
// all" are different diagnoses, and stopping at the first failed assertion
// throws away the one that tells them apart.

/** @typedef {{what: string, ok: boolean, actual?: unknown, expected?: unknown, note?: string}} Result */

export class Checks {
	/** @param {string} label what this list is about */
	constructor(label) {
		this.label = label;
		/** @type {Result[]} */
		this.results = [];
		/** @type {string[]} */
		this.notes = [];
	}

	/** Record a comparison. Values are compared by JSON shape, not by identity. */
	is(what, actual, expected) {
		const ok = JSON.stringify(actual) === JSON.stringify(expected);
		this.results.push({ what, ok, actual, expected });
		return ok;
	}

	/** Record a condition that carries its own explanation. */
	ok(what, condition, note) {
		this.results.push({ what, ok: !!condition, note });
		return !!condition;
	}

	/** Something worth reading that is not pass or fail. */
	note(text) {
		this.notes.push(text);
	}

	get failures() {
		return this.results.filter(result => !result.ok);
	}

	get passed() {
		return this.results.length - this.failures.length;
	}

	/** The whole list as lines, failures carrying both sides. */
	report() {
		const lines = [`# ${this.label}`, ""];
		for (const result of this.results) {
			lines.push(`${result.ok ? "ok  " : "FAIL"}  ${result.what}`);
			if (!result.ok) {
				if ("expected" in result) {
					lines.push(`        expected ${render(result.expected)}`);
					lines.push(`        actual   ${render(result.actual)}`);
				}
				if (result.note) lines.push(`        ${result.note}`);
			}
		}
		if (this.notes.length) {
			lines.push("");
			for (const note of this.notes) lines.push(`note  ${note}`);
		}
		lines.push("");
		lines.push(this.failures.length ? `${this.failures.length} of ${this.results.length} checks failed` : `all ${this.results.length} checks passed`);
		return lines.join("\n");
	}
}

function render(value) {
	const text = typeof value === "string" ? JSON.stringify(value) : JSON.stringify(value, null, 1);
	return text === undefined ? String(value) : text.length > 600 ? `${text.slice(0, 600)}...` : text;
}
