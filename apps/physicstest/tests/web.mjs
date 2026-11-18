// physicstest's WebAssembly build in Node, without a browser and without npm packages: the same
// program and arguments as on the desktop (the checksum must be the one of the native run).
//
//   python build_rs.py app physicstest --target web
//   node apps/physicstest/tests/web.mjs [BUILD_DIR] [-- ARGS]      (default: build, --checksum)
//
// Runs every .wasm of BUILD_DIR/web/physicstest (the plain and the SIMD build) and prints its output;
// exit code 1 when a run fails or two builds print different lines.
import { readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';

const argv = process.argv.slice(2);
const dashes = argv.indexOf('--');
const dir = resolve(dashes === 0 ? 'build' : (argv[0] || 'build'), 'web', 'physicstest');
const args = dashes >= 0 ? argv.slice(dashes + 1) : ['--checksum'];
const files = readdirSync(dir).filter(f => f.endsWith('.wasm')).sort();
if (!files.length) throw new Error(`no .wasm in ${dir}`);
let first = null;
let failed = false;
for (const file of files) {
  const module = await WebAssembly.compile(readFileSync(resolve(dir, file)));
  let instance;
  let stdout = '';
  let stderr = '';
  const known = {
    write(stream, ptr, len) {
      const text = new TextDecoder().decode(new Uint8Array(instance.exports.memory.buffer, ptr, len));
      if (stream === 1) stdout += text; else stderr += text;
    },
    exit(code) { throw new Error(`exit ${code}: ${stderr}`); },
    time_us() { return performance.now() * 1000; },
    thread_count() { return 1; },
  };
  // anything else the runtime imports (frames, services, threads) is not used by a console program
  const jfgui = new Proxy(known, { get: (t, k) => (k in t ? t[k] : () => 0) });
  const imports = {};
  for (const imp of WebAssembly.Module.imports(module)) {
    imports[imp.module] ??= {};
    if (imp.kind === 'function') imports[imp.module][imp.name] = imp.module === 'jfgui' ? jfgui[imp.name] : () => 0;
  }
  instance = await WebAssembly.instantiate(module, imports);
  const data = new TextEncoder().encode(args.join('\n'));
  const ptr = instance.exports.jfgui_buffer(data.length);
  new Uint8Array(instance.exports.memory.buffer, ptr, data.length).set(data);
  const t0 = performance.now();
  const code = instance.exports.jfgui_start(800, 600, 96, data.length);
  const ms = (performance.now() - t0).toFixed(0);
  process.stdout.write(`== ${file} (exit ${code}, ${ms} ms)\n${stdout}${stderr ? 'stderr: ' + stderr : ''}`);
  if (code !== 0) failed = true;
  if (first === null) first = stdout;
  else if (args.includes('--checksum') && stdout !== first) {
    console.log('the builds differ');
    failed = true;
  }
}
process.exit(failed ? 1 : 0);
