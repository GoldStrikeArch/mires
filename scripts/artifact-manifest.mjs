// Reads the compiler's deployment contract for JavaScript packagers. Compiler-owned bytes
// are verified here; external modules and the final bundler graph need their own binding.
import { createHash } from 'node:crypto';
import { readFileSync, realpathSync } from 'node:fs';
import path from 'node:path';

export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

function record(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('invalid artifact contract record');
  return Object.entries(value);
}

const strings = value => Array.isArray(value) && value.every(item => typeof item === 'string');

function fields(value, required, optional = []) {
  record(value);
  if (required.some(key => !Object.hasOwn(value, key))
    || Object.keys(value).some(key => !required.includes(key) && !optional.includes(key))) {
    throw new Error('invalid artifact contract fields');
  }
}

function artifactBytes(directory, name, artifact) {
  const absolute = realpathSync(path.resolve(directory, name));
  if (!absolute.startsWith(`${directory}${path.sep}`)) throw new Error(`${name}: artifact escapes its package`);
  const bytes = readFileSync(absolute);
  if (artifact.bytes !== bytes.length || artifact.sha256 !== sha256(bytes)) {
    throw new Error(`${name}: artifact bytes disagree with linker manifest`);
  }
  return bytes;
}

export function readArtifactManifest(file) {
  const manifest = JSON.parse(readFileSync(file, 'utf8'));
  fields(manifest, ['schema', 'object_format', 'jir_schema', 'semantic_policy', 'syntax_ceiling', 'entries', 'artifacts']);
  if (manifest.schema !== 2 || manifest.object_format !== 27 || manifest.jir_schema !== 15
    || manifest.semantic_policy !== 'faithful' || !['2018', 'latest'].includes(manifest.syntax_ceiling)) {
    throw new Error(`${file}: unsupported compiler artifact contract`);
  }
  const directory = realpathSync(path.dirname(file));
  const contents = new Map();
  for (const [name, artifact] of record(manifest.artifacts)) {
    fields(artifact, ['kind', 'bytes', 'sha256', 'imports', 'items', 'exports'], ['source_map', 'declarations', 'runtime']);
    if (!['program', 'source-map', 'declarations', 'runtime'].includes(artifact.kind)
      || !strings(artifact.items) || !strings(artifact.exports)) {
      throw new Error(`${name}: invalid artifact kind or symbols`);
    }
    if (!name || name.includes('\\') || path.isAbsolute(name)
      || name.split('/').some(part => !part || part === '.' || part === '..')) {
      throw new Error(`${file}: invalid artifact path ${name}`);
    }
    const bytes = artifactBytes(directory, name, artifact);
    contents.set(name, bytes.toString('utf8'));
    for (const [, target] of record(artifact.imports)) {
      if (target !== null && typeof target !== 'string') throw new Error(`${name}: invalid import target`);
      if (target !== null && !['program', 'runtime'].includes(manifest.artifacts[target]?.kind)) {
        throw new Error(`${name}: missing imported artifact ${target}`);
      }
    }
    for (const [edge, kind] of [['source_map', 'source-map'], ['declarations', 'declarations']]) {
      if (artifact[edge] != null && (typeof artifact[edge] !== 'string' || manifest.artifacts[artifact[edge]]?.kind !== kind)) {
        throw new Error(`${name}: invalid ${edge} edge`);
      }
    }
    const runtime = artifact.runtime;
    if (runtime != null) fields(runtime, ['source_sha256', 'direct', 'closure', 'external'], ['module', 'artifact']);
    if (runtime && (!strings(runtime.direct) || !strings(runtime.closure) || !strings(runtime.external)
      || !runtime.direct.every(name => runtime.closure.includes(name))
      || runtime.external.some(name => runtime.closure.includes(name))
      || !/^[a-f0-9]{64}$/i.test(runtime.source_sha256)
      || runtime.module != null && (typeof runtime.module !== 'string' || !Object.hasOwn(artifact.imports, runtime.module))
      || runtime.artifact != null && (typeof runtime.artifact !== 'string' || manifest.artifacts[runtime.artifact]?.kind !== 'runtime'
        || runtime.module != null && artifact.imports[runtime.module] !== runtime.artifact))) {
      throw new Error(`${name}: runtime requirement disagrees with artifact graph`);
    }
  }
  for (const [name, entry] of record(manifest.entries)) {
    fields(entry, ['role', 'program', 'roots']);
    const roots = record(entry.roots);
    const program = manifest.artifacts[entry.program];
    if (typeof entry.program !== 'string' || program?.kind !== 'program' || !['main', 'worker', 'island', 'procedure', 'shared'].includes(entry.role)
      || entry.role !== 'shared' && !roots.length
      || roots.some(([, root]) => typeof root !== 'string' || !program.items.includes(root))) {
      throw new Error(`${name}: invalid entry root or role`);
    }
  }
  return { manifest, directory, contents };
}

export function entryResources(manifest, entries = Object.keys(manifest.entries)) {
  const pending = entries.map(name => {
    if (!manifest.entries[name]) throw new Error(`unknown entry ${name}`);
    return manifest.entries[name].program;
  });
  const resources = new Set();
  while (pending.length) {
    const name = pending.pop();
    if (resources.has(name)) continue;
    resources.add(name);
    const artifact = manifest.artifacts[name];
    pending.push(...Object.values(artifact.imports).filter(name => name !== null));
    if (artifact.source_map) pending.push(artifact.source_map);
    if (artifact.declarations) pending.push(artifact.declarations);
    if (artifact.runtime?.artifact) pending.push(artifact.runtime.artifact);
  }
  return resources;
}

// Resolve the public binding, including named/star re-exports and imported
// aliases. Binding identity distinguishes duplicate paths from ambiguous stars.
// Rollup's ModuleInfo.exports only reports '*' for export-all declarations.
async function runtimeExport(context, id, name, seen = new Set()) {
  const key = JSON.stringify([id, name]);
  if (seen.has(key)) return null;
  seen = new Set([...seen, key]);
  const info = context.getModuleInfo(id);
  if (!info || info.isExternal || !info.ast) return null;
  const body = info.ast.body;
  const symbol = node => node.name ?? node.value;
  const from = async (source, imported) => {
    const target = await context.resolve(source, id, { skipSelf: true });
    if (!target || target.external) return null;
    return imported === null ? JSON.stringify([target.id, '*namespace*'])
      : runtimeExport(context, target.id, imported, seen);
  };
  const local = async name => {
    for (const node of body) {
      if (node.type !== 'ImportDeclaration') continue;
      const specifier = node.specifiers.find(item => item.local.name === name);
      if (specifier) return from(node.source.value, specifier.type === 'ImportNamespaceSpecifier' ? null
        : specifier.type === 'ImportDefaultSpecifier' ? 'default' : symbol(specifier.imported));
    }
    return JSON.stringify([id, name]);
  };
  for (const node of body) {
    if (node.type === 'ExportNamedDeclaration') {
      const specifier = node.specifiers.find(item => symbol(item.exported) === name);
      if (specifier) return node.source ? from(node.source.value, symbol(specifier.local)) : local(symbol(specifier.local));
    }
    if (node.type === 'ExportAllDeclaration' && node.exported && symbol(node.exported) === name) {
      return from(node.source.value, null);
    }
    if (node.type === 'ExportDefaultDeclaration' && name === 'default') {
      return JSON.stringify([id, node.declaration.id?.name ?? '*default*']);
    }
  }
  if (info.exportedBindings['.']?.includes(name)) return local(name);
  if (name === 'default') return null;
  const bindings = new Set();
  for (const node of body) {
    if (node.type !== 'ExportAllDeclaration' || node.exported) continue;
    const binding = await from(node.source.value, name);
    if (binding === undefined) return undefined;
    if (binding) bindings.add(binding);
  }
  return bindings.size > 1 ? undefined : bindings.size === 1 ? [...bindings][0] : null;
}

// Rollup supplies the resolved module graph. Compare that graph with the compiler's import
// contract before tree shaking or final printing can conceal an incorrectly wired provider.
export function artifactGraphPlugin(manifestFiles, { onReceipt = () => {} } = {}) {
  const deployments = manifestFiles.map(input => typeof input === 'string'
    ? readArtifactManifest(input)
    : { ...readArtifactManifest(input.path), entryNames: input.entries });
  const compilerModules = new Set(deployments.flatMap(({ manifest, directory, entryNames }) =>
    [...entryResources(manifest, entryNames)].filter(name => manifest.artifacts[name].kind === 'program')
      .map(name => realpathSync(path.resolve(directory, name)))));
  return {
    name: 'rcgjs-artifact-contract',
    transform(code, id) {
      // The linker owns program reachability. Rollup otherwise removes empty
      // infinite loops, allowing Rust panic handlers to return. Runtime modules
      // retain ordinary tree shaking; no source bytes or maps change here.
      return compilerModules.has(id) ? { moduleSideEffects: 'no-treeshake' } : null;
    },
    async buildEnd(error) {
      if (error) return;
      const providers = new Map();
      const programs = [];
      for (const { manifest, directory, entryNames } of deployments) {
        for (const name of entryResources(manifest, entryNames)) {
          const artifact = manifest.artifacts[name];
          if (artifact.kind !== 'program') continue;
          const id = realpathSync(path.resolve(directory, name));
          const info = this.getModuleInfo(id);
          if (!info || info.isExternal) throw new Error(`${name}: compiler program is absent from bundler graph`);
          if (sha256(info.code ?? '') !== artifact.sha256) throw new Error(`${name}: bundler input differs from compiler artifact`);
          if (info.moduleSideEffects !== 'no-treeshake') throw new Error(`${name}: compiler program preservation was overridden`);
          const expected = new Set();
          for (const [specifier, target] of Object.entries(artifact.imports)) {
            const resolution = await this.resolve(specifier, id, { skipSelf: true });
            if (!resolution || resolution.external) throw new Error(`${name}: unattested external module ${specifier}`);
            const actual = realpathSync(resolution.id);
            const local = target ?? (specifier.startsWith('./') ? specifier : null);
            if (local && actual !== realpathSync(path.resolve(directory, local))) {
              throw new Error(`${name}: ${specifier} resolved outside its declared artifact identity`);
            }
            expected.add(resolution.id);
            if (artifact.runtime?.module === specifier) {
              const provider = this.getModuleInfo(resolution.id);
              const bytes = readFileSync(actual);
              if (!provider || sha256(provider.code ?? '') !== sha256(bytes)) {
                throw new Error(`${name}: runtime provider bytes differ from bundler input`);
              }
              const requiredExports = [...new Set([...artifact.runtime.direct, ...artifact.runtime.external])].sort();
              for (const symbol of requiredExports) {
                if (!await runtimeExport(this, resolution.id, symbol)) {
                  throw new Error(`${name}: runtime provider ${specifier} has no unambiguous export ${symbol}`);
                }
              }
              const previous = providers.get(actual);
              providers.set(actual, { path: actual, sha256: sha256(bytes), bytes: bytes.length,
                requiredExports: [...new Set([...(previous?.requiredExports ?? []), ...requiredExports])].sort(),
                required: [...new Set([...(previous?.required ?? []), ...artifact.runtime.closure, ...artifact.runtime.external])].sort(),
                requiredBy: [...new Set([...(previous?.requiredBy ?? []), id])].sort() });
            }
          }
          const actual = new Set(info.importedIds);
          if (actual.size !== expected.size || [...actual].some(id => !expected.has(id))) {
            throw new Error(`${name}: generated imports disagree with linker artifact manifest`);
          }
          // Dynamic imports require explicit graph edges; none of the currently emitted chunk
          // imports are dynamic. Refuse an unrecorded edge rather than certifying partial closure.
          if (info.dynamicallyImportedIds.length) throw new Error(`${name}: unrecorded dynamic import in compiler artifact`);
          programs.push({ path: id, sha256: artifact.sha256, imports: [...actual].sort(), moduleSideEffects: info.moduleSideEffects });
        }
      }
      const modules = [...this.getModuleIds()].map(id => {
        const info = this.getModuleInfo(id);
        if (!info || info.isExternal || id.startsWith('\0')) throw new Error(`unattested bundled resource ${id}`);
        const bytes = readFileSync(id);
        if (sha256(info.code ?? '') !== sha256(bytes)) throw new Error(`bundler resource transformed without an artifact receipt: ${id}`);
        return { path: id, sha256: sha256(bytes), bytes: bytes.length,
          imports: [...info.importedIds].sort(), dynamicImports: [...info.dynamicallyImportedIds].sort() };
      }).sort((a, b) => a.path.localeCompare(b.path));
      // Resolve hooks above may be asynchronous. Recheck the original contract
      // after those hooks, including runtime and metadata that are not programs.
      for (const { manifest, directory } of deployments) {
        for (const [name, artifact] of Object.entries(manifest.artifacts)) {
          artifactBytes(directory, name, artifact);
        }
      }
      await onReceipt({ schema: 1, scope: 'verified Rollup input graph', programs, runtimeProviders: [...providers.values()], modules });
    },
  };
}
