function evaluate(ops, keep) {
  const summary = { value: 2, accepted: 0 };

  for (const op of ops) {
    let candidate;
    switch (op.tag) {
      case 0:
        candidate = (summary.value + op.value) | 0;
        break;
      case 1:
        candidate = Math.imul(summary.value, op.value);
        break;
      default:
        candidate = op.value;
    }

    if (keep(candidate)) {
      summary.value = candidate;
      summary.accepted += 1;
    }
  }

  return summary;
}

const minimum = 5;
let checked = 0;
const summary = evaluate(
  [
    { tag: 0, value: 4 },
    { tag: 1, value: 3 },
    { tag: 2, value: 7 },
    { tag: 0, value: -8 },
    { tag: 1, value: 0 },
  ],
  (value) => {
    checked += 1;
    return value >= minimum && value % 2 === 0;
  },
);

console.log(`${summary.value} ${summary.accepted} ${checked}`);
