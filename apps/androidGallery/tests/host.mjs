// Regression check for the shared host ABI (in particular 32-bit WASM buffer addresses).
// node apps/androidGallery/tests/host.mjs build/android-work
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createHash } from 'node:crypto';

const output = resolve(process.argv[2] || 'build/android-work');
let instance, pixels, frames = 0;
const module = await WebAssembly.compile(readFileSync(resolve(output, 'web/androidGallery/androidGallery.wasm')));
const imports = { jfgui: {
  write(stream, ptr, length) {
    if (stream === 2) throw Error(new TextDecoder().decode(new Uint8Array(instance.exports.memory.buffer, ptr, length)));
  },
  exit(code) { throw Error(`Unexpected exit ${code}`); },
  time_us() { return performance.now() * 1000; },
  present(ptr, length, width, height, stride) {
    if (width !== 390 || height !== 1100 || stride !== width * 4) throw Error('Incorrect framebuffer dimensions');
    pixels = new Uint8Array(instance.exports.memory.buffer, ptr, length).slice();
    frames++;
  },
  request() {},
} };
instance = await WebAssembly.instantiate(module, imports);
const api = instance.exports;
const args = new TextEncoder().encode('--theme\nlight\n--anim\noff');
const ptr = api.jfgui_buffer(args.length);
if (!ptr) throw Error('Host input buffer allocation failed');
new Uint8Array(api.memory.buffer, ptr, args.length).set(args);
if (api.jfgui_start(390, 1100, 96, args.length) !== 0) throw Error('App startup failed');
api.jfgui_tick(1);
if (!frames || !pixels || new Set(pixels).size < 32) throw Error('No rendered UI');
const first = createHash('sha256').update(pixels).digest('hex');
// Browser has a 32 DIP library caption above the same gallery layout.
for (const type of [1, 2, 3]) {
  api.jfgui_event(type, 1, 0, 0, 100, 334, 0);
  api.jfgui_input(10 + type);
}
api.jfgui_tick(20);
if (createHash('sha256').update(pixels).digest('hex') === first) throw Error('Pointer input did not change the UI');
if (api.jfgui_start(390, 1100, 96, 0) !== -1) throw Error('Duplicate startup was accepted');
console.log(`Shared WASM host: startup, buffer ABI, rendering, pointer input and duplicate-start rejection passed (${frames} frames).`);
