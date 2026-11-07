// Convenience entry point: `node --import ./harness/register-loader.mjs foo.mjs`
// installs the dom-expressions resolution hooks from loader.mjs.
import { register } from "node:module";

register("./loader.mjs", import.meta.url);
