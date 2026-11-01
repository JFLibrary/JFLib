// wgsl_run.mjs -- runs compute shaders in the installed Google Chrome (WebGPU, Tint) and writes
// their results: the browser side of gputest's WGSL behaviour test (apps/gputest/t_wgsl.rs).
//
//   gputest --test wgsl --dump cases.jsonl          the cases: {"name", "n", "outw", "src",
//                                                   "inputs"} per line
//   node tools/wgsl_run.mjs cases.jsonl results.txt Chrome runs each shader (entry point main,
//                                                   binding 0 the inputs as array<u32>, binding 1
//                                                   n * outw result words, workgroups of 64)
//   gputest --test wgsl --chrome results.txt        the results against lib/wgsl's constant
//                                                   evaluator with the test's tolerances
//
// results.txt: one line per case, `name<TAB>w,w,...` (or `name<TAB>error: ...`).  Playwright and
// Chrome as for tools/wgsl_ref.mjs (PLAYWRIGHT_MODULE, CHROME_PATH or the installed Chrome);
// headless, no flags, the page https://wgsl-run.invalid/ answered by page.route().
import {readFileSync, writeFileSync} from 'node:fs';
import {chromiumOptions, loadPlaywright} from './tests/playwright.mjs';

async function runCase(c) {
  const g = globalThis;
  if (!g.dev) {
    const adapter = await navigator.gpu.requestAdapter();
    g.dev = await adapter.requestDevice({requiredFeatures: [...adapter.features]});
  }
  const device = g.dev;
  device.pushErrorScope('validation');
  const m = device.createShaderModule({code: c.src});
  const info = await m.getCompilationInfo();
  const errs = info.messages.filter(x => x.type === 'error').map(x => `${x.lineNum}:${x.linePos} ${x.message}`);
  if (errs.length) {
    await device.popErrorScope();
    return {error: errs.join('; ')};
  }
  const pipe = device.createComputePipeline({layout: 'auto', compute: {module: m, entryPoint: 'main'}});
  const inWords = Math.max(c.inputs.length, 1);
  const ib = device.createBuffer({size: inWords * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST});
  device.queue.writeBuffer(ib, 0, new Uint32Array(c.inputs.length ? c.inputs : [0]));
  const outBytes = c.n * c.outw * 4;
  const ob = device.createBuffer({size: outBytes, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC});
  const rb = device.createBuffer({size: outBytes, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST});
  const bg = device.createBindGroup({layout: pipe.getBindGroupLayout(0),
    entries: [{binding: 0, resource: {buffer: ib}}, {binding: 1, resource: {buffer: ob}}]});
  const e = device.createCommandEncoder();
  const p = e.beginComputePass();
  p.setPipeline(pipe);
  p.setBindGroup(0, bg);
  p.dispatchWorkgroups(Math.ceil(c.n / 64));
  p.end();
  e.copyBufferToBuffer(ob, 0, rb, 0, outBytes);
  device.queue.submit([e.finish()]);
  const v = await device.popErrorScope();
  if (v) return {error: v.message};
  await rb.mapAsync(GPUMapMode.READ);
  const words = Array.from(new Uint32Array(rb.getMappedRange().slice(0)));
  rb.unmap();
  return {words};
}

async function main() {
  const [inFile, outFile] = process.argv.slice(2);
  if (!inFile || !outFile) throw new Error('node tools/wgsl_run.mjs cases.jsonl results.txt');
  const cases = readFileSync(inFile, 'utf8').split('\n').filter(l => l.trim()).map(l => JSON.parse(l));
  const pw = loadPlaywright();
  const browser = await pw.chromium.launch({headless: true, ...chromiumOptions()});
  const page = await browser.newPage();
  await page.route('https://wgsl-run.invalid/**', r => r.fulfill({contentType: 'text/html', body: '<!doctype html><title>wgsl_run</title>'}));
  await page.goto('https://wgsl-run.invalid/');
  const lines = [];
  for (const c of cases) {
    const r = await page.evaluate(runCase, c);
    lines.push(`${c.name}\t${r.error ? 'error: ' + r.error.replace(/\s+/g, ' ') : r.words.join(',')}`);
  }
  writeFileSync(outFile, lines.join('\n') + '\n');
  console.log(`Chrome ${browser.version()}: ${cases.length} cases, ${lines.filter(l => l.includes('\terror:')).length} errors -> ${outFile}`);
  await browser.close();
}

main().catch(e => {
  console.error(e.message || e);
  process.exit(1);
});
