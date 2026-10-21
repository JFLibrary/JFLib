// Run the actual WASM corpus against its native digest without browser or npm dependencies.
// node apps/portabletest/tests/compare.mjs build/portable-work
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';

const output = resolve(process.argv[2] || 'build/portable-work');
const native = execFileSync(resolve(output, process.platform === 'win32' ? 'win/portabletest.exe' : 'linux/portabletest'), [], {encoding: 'utf8'}).trim();
if (!/^portabletest ok digest=[0-9a-f]{16}$/.test(native)) throw new Error(`Native checks failed: ${native}`);
const module = await WebAssembly.compile(readFileSync(resolve(output, 'web/portabletest',process.argv[3]||'portabletest.wasm')));
let instance;
let stdout = '';
let stderr = '';
const requests = [];
const imports = {jfgui: {
  write(stream, ptr, len) {
    const text = new TextDecoder().decode(new Uint8Array(instance.exports.memory.buffer, ptr, len));
    if (stream === 1) stdout += text; else stderr += text;
  },
  exit(code) { throw new Error(`WASM exited ${code}: ${stderr}`); },
  time_us() { return performance.now()*1000; },
  present() {},
  request(kind, id, ptr, len) {
    requests.push({kind, id, payload: new Uint8Array(instance.exports.memory.buffer, ptr, len).slice()});
  },
}};
instance = await WebAssembly.instantiate(module, imports);
if (instance.exports.jfgui_agent(1) !== 0) throw new Error('Activation must require completed startup');
const code = instance.exports.jfgui_start(1000, 700, 96, 0);
if (code !== 0 || stdout.trim() !== native || stderr) throw new Error(`Cross-target mismatch:\nnative=${native}\nwasm=${stdout}\nstderr=${stderr}`);
// Agent requests remain inaccessible without the explicit startup flag.
const data = new TextEncoder().encode('{"jsonrpc":"2.0","id":1,"method":"actions/list"}');
const ptr = instance.exports.jfgui_buffer(data.length);
new Uint8Array(instance.exports.memory.buffer, ptr, data.length).set(data);
if (instance.exports.jfgui_control(data.length) !== 0) throw new Error('Control unexpectedly enabled');
if (instance.exports.jfgui_agent(1) !== 1) throw new Error('Explicit activation failed');
const len = instance.exports.jfgui_control(data.length);
const response = new TextDecoder().decode(new Uint8Array(instance.exports.memory.buffer, instance.exports.jfgui_response(), len));
if (response !== `probe:${new TextDecoder().decode(data)}`) throw new Error('Activation did not reach the owned runtime');
if (instance.exports.jfgui_agent(0) !== 1 || instance.exports.jfgui_control(data.length) !== 0) throw new Error('Revoked agent remained reachable');
if (JSON.stringify(requests.filter(r=>r.kind===10).map(r=>r.payload[0])) !== '[1,0]') throw new Error('Activation/revocation were not mirrored to the browser host');
const first = requests.find(r => r.kind === 1);
if (!first || !first.id) throw new Error('WASM did not request persistence through the host');
const pathLength = new DataView(first.payload.buffer).getUint32(0, true);
const path = new TextDecoder().decode(first.payload.subarray(4, 4 + pathLength));
const bytes = new TextDecoder().decode(first.payload.subarray(4 + pathLength));
if (path !== '/documents/Quellen-ä.bin' || bytes !== 'first') throw new Error('Host request payload framing or snapshot differs');
if (instance.exports.jfgui_complete(first.id, 0) !== 0) throw new Error('Completed/consumed request id was accepted twice');
console.log(`${native}\nNative and WASM byte-identical; VFS, completion errors/stale revisions, host framing and activation/revocation passed.`);
