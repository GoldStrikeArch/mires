// The local OCaml switch shared by the Melange and js_of_ocaml builders, as the comparator builds
// use it: opam root .toolchains/opam, switch mires-comparators (experiments/tools/
// setup-ocaml-comparators.sh creates both), DUNE_JOBS=4 because Dune's automatic job detection
// crashes on this Mac. MIRES_OPAM_ROOT and MIRES_OPAM_SWITCH override the location.

import { existsSync } from 'node:fs'
import { resolve } from 'node:path'

import { repositoryRoot, run, versionOf } from './common.mjs'

export const opamRoot = process.env.MIRES_OPAM_ROOT || resolve(repositoryRoot, '.toolchains/opam')
export const opamSwitch = process.env.MIRES_OPAM_SWITCH || 'mires-comparators'

export function opamEnv() {
  return { ...process.env, DUNE_JOBS: process.env.DUNE_JOBS || '4' }
}

export function requireSwitch() {
  if (!existsSync(resolve(opamRoot, opamSwitch))) {
    throw new Error(`opam switch ${opamSwitch} missing under ${opamRoot}; run experiments/tools/setup-ocaml-comparators.sh`)
  }
}

export function opamExec(args, options = {}) {
  return run('opam', ['exec', '--root', opamRoot, '--switch', opamSwitch, '--', ...args], { env: opamEnv(), ...options })
}

export function opamVersion(args) {
  return versionOf('opam', ['exec', '--root', opamRoot, '--switch', opamSwitch, '--', ...args], { env: opamEnv() })
}
