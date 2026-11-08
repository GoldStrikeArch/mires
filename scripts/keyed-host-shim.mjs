// Test-only host module for the compiled host-backed `Rows` source fixture.
export * from "../build/keyedtest/shim.mjs";

const generations = [
  [{ id: 1, label: 1 }, { id: 2, label: 2 }, { id: 3, label: 3 }],
  [{ id: 3, label: 30 }, { id: 1, label: 10 }, { id: 2, label: 20 }],
];
let generation = 0;

export const keyed_rows_len = () => generations[generation].length;
export const keyed_rows_id = index => generations[generation][index].id;
export const keyed_rows_label = index => generations[generation][index].label;
export const keyed_rows_flip = () => { generation = 1; };
