// wgsl_ref.mjs -- Tint's verdict on WGSL files: the WGSL compiler of the installed Google
// Chrome (WebGPU) compiles each file and creates a pipeline per entry point, and the results go
// out as JSON.  It is the reference for JFLib's own WGSL compiler (G2): tools/wgsl_corpus/
// expected.json is its output over the corpus, so the compiler's tests compare without Chrome.
//
//   node tools/wgsl_ref.mjs [--out FILE] [--base DIR] [--headed] [--unsafe] FILE.wgsl|DIR ...
//     DIR: every .wgsl file below it (sorted); --base: the folder the keys of "files" are
//     relative to (default: the first DIR, else the working directory); --out: write the JSON
//     there (default stdout); --headed: a visible window; --unsafe: --enable-unsafe-webgpu
//     (experimental features: NOT what Chrome ships, so not for expected.json)
//   node tools/wgsl_ref.mjs --corpus    = --out tools/wgsl_corpus/expected.json tools/wgsl_corpus
//
// Playwright as for the browser tests (tools/tests/playwright.mjs: PLAYWRIGHT_MODULE, else
// Node's resolution); Chrome through CHROME_PATH or the installed Google Chrome (channel
// 'chrome').  What Chrome needs here (Chrome 156.0.8078.76, Windows 11, GeForce RTX 2070 SUPER):
//   - headless works, no flags needed: WebGPU is on by default and the real GPU is used
//     (adapter.info: vendor "nvidia", architecture "turing", not the fallback adapter);
//     --enable-unsafe-webgpu only adds experimental features (the WGSL language feature
//     fragment_depth, chromium-experimental-* device features), so it stays off
//   - a secure context: about:blank is none (navigator.gpu is undefined there); the page is
//     https://wgsl-ref.invalid/, answered by page.route() (no server, no network)
//   - the device: default limits; without optional features unless the file has an `enable`
//     directive or a `// features: all` line, then with every feature the adapter offers
//     (shader-f16, subgroups, clip-distances, dual-source-blending, primitive-index ...)
// The JSON records the Chrome version, the adapter, its features and the WGSL language
// features.
//
// Per file: createShaderModule + getCompilationInfo() inside error scopes (validation,
// internal); for a module without errors one pipeline per entry point, layout 'auto', inside
// error scopes (some errors only show there: overrides, limits, interfaces): compute as it is;
// vertex with one vertex buffer per @location input (float32 / sint32 / uint32 x N), no
// fragment stage, a depth32float depth target; fragment with the module's first vertex entry
// point, or, when it has none, a generated vertex shader that writes the fragment's inputs
// (same locations, types, interpolation); one r32float / r32sint / r32uint target per
// @location output (by its component type), a depth32float depth target.  Pipeline constants
// from a `// constants: {"name": value}` line.
//
// The file's own lines (the corpus convention, tools/wgsl_corpus/README.md): the first comment
// names the rule, `// expect: valid | error | pipeline-error` what we expect, `// uniformity`
// marks uniformity cases.  The verdict: "error" (getCompilationInfo has an error), else
// "pipeline-error" (a pipeline failed), else "valid"; "mismatches" lists the files whose
// verdict differs from their expect line.
//
// Output: { tool, chrome, headless, flags, adapter, features, wgslLanguageFeatures, files:
// { path: { rule, expect, uniformity, verdict, messages: [{ type, lineNum, linePos, offset,
// length, text }], pipelines: [{ entry, stage, ok, errors: [{ kind, text }] }] } }, summary }.
// Nothing time-dependent: the same Chrome gives the same file.
import {readFileSync, readdirSync, statSync, writeFileSync} from 'node:fs';
import {join, relative, resolve, sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromiumOptions, loadPlaywright} from './tests/playwright.mjs';

const HERE = fileURLToPath(new URL('.', import.meta.url));

function parseArgs(argv) {
  const a = {out: null, base: null, headed: false, unsafe: false, inputs: []};
  for (let i = 0; i < argv.length; i++) {
    const x = argv[i];
    if (x === '--out') a.out = argv[++i];
    else if (x === '--base') a.base = argv[++i];
    else if (x === '--headed') a.headed = true;
    else if (x === '--unsafe') a.unsafe = true;
    else if (x === '--corpus') {
      a.out = join(HERE, 'wgsl_corpus', 'expected.json');
      a.inputs.push(join(HERE, 'wgsl_corpus'));
    } else if (x === '--help' || x === '-h') {
      console.log('node tools/wgsl_ref.mjs [--out FILE] [--base DIR] [--headed] [--unsafe] FILE.wgsl|DIR ... | --corpus');
      process.exit(0);
    } else a.inputs.push(x);
  }
  if (!a.inputs.length) throw new Error('no input: give .wgsl files or folders (or --corpus)');
  return a;
}

function collect(inputs) {
  const files = [];
  const walk = d => {
    for (const n of readdirSync(d).sort()) {
      const p = join(d, n);
      if (statSync(p).isDirectory()) walk(p);
      else if (n.endsWith('.wgsl')) files.push(p);
    }
  };
  for (const x of inputs) {
    if (statSync(x).isDirectory()) walk(x);
    else files.push(x);
  }
  return files;
}

// ---------------------------------------------------------------- the file's own lines

function header(src) {
  const h = {rule: '', expect: null, uniformity: false, constants: {}, allFeatures: false};
  for (const line of src.split(/\r?\n/)) {
    const m = /^\s*\/\/\s?(.*)$/.exec(line);
    if (!m) break;
    const t = m[1].trim();
    if (!h.rule && !/^(expect|constants|features)\s*:/.test(t) && t !== 'uniformity') h.rule = t;
    let k;
    if ((k = /^expect:\s*(\S+)/.exec(t))) h.expect = k[1];
    else if (t === 'uniformity') h.uniformity = true;
    else if ((k = /^constants:\s*(\{.*\})\s*$/.exec(t))) h.constants = JSON.parse(k[1]);
    else if (/^features:\s*all\s*$/.test(t)) h.allFeatures = true;
  }
  return h;
}

// ---------------------------------------------------------------- a small WGSL reader:
// entry points with their inputs and outputs (enough to make dummy pipeline state)

function stripComments(s) {
  let out = '', i = 0, depth = 0;
  while (i < s.length) {
    if (depth === 0 && s.startsWith('//', i)) {
      while (i < s.length && s[i] !== '\n') i++;
    } else if (s.startsWith('/*', i)) {
      depth++;
      i += 2;
    } else if (depth > 0 && s.startsWith('*/', i)) {
      depth--;
      i += 2;
    } else {
      out += depth > 0 ? (s[i] === '\n' ? '\n' : ' ') : s[i];
      i++;
    }
  }
  return out;
}

/// split at top-level commas (outside (), [], <>, {})
function splitTop(s) {
  const parts = [];
  let depth = 0, cur = '';
  for (const c of s) {
    if ('([{<'.includes(c)) depth++;
    if (')]}>'.includes(c)) depth--;
    if (c === ',' && depth === 0) {
      parts.push(cur);
      cur = '';
    } else cur += c;
  }
  if (cur.trim()) parts.push(cur);
  return parts.map(p => p.trim()).filter(Boolean);
}

/// the text from an opening bracket at i to its match (exclusive) and the index after it
function balanced(s, i, open, close) {
  let depth = 0;
  for (let j = i; j < s.length; j++) {
    if (s[j] === open) depth++;
    else if (s[j] === close && --depth === 0) return [s.slice(i + 1, j), j + 1];
  }
  return [s.slice(i + 1), s.length];
}

/// "@location(0) @interpolate(flat) name: type" -> {attrs: {location: '0', ...}, name, type}
function decl(text) {
  const attrs = {};
  let t = text.trim();
  for (;;) {
    const m = /^@\s*([A-Za-z_]\w*)\s*/.exec(t);
    if (!m) break;
    t = t.slice(m[0].length);
    let arg = '';
    if (t.startsWith('(')) {
      const [inner, next] = balanced(t, 0, '(', ')');
      arg = inner.trim();
      t = t.slice(next).trim();
    }
    attrs[m[1]] = arg;
  }
  const c = t.indexOf(':');
  return c < 0 ? {attrs, name: t.trim(), type: ''} : {attrs, name: t.slice(0, c).trim(), type: t.slice(c + 1).trim()};
}

/// an attribute with arguments up to one level of nested parentheses
const ATTR = String.raw`@\s*[A-Za-z_]\w*\s*(?:\((?:[^()]|\([^()]*\))*\))?`;

function readModule(src) {
  const s = stripComments(src);
  const structs = {}, aliases = {};
  for (const m of s.matchAll(/\bstruct\s+([A-Za-z_]\w*)\s*\{/g)) {
    const [body] = balanced(s, m.index + m[0].length - 1, '{', '}');
    structs[m[1]] = splitTop(body).map(decl);
  }
  for (const m of s.matchAll(/\balias\s+([A-Za-z_]\w*)\s*=\s*([^;]+);/g)) aliases[m[1]] = m[2].trim();
  const entries = [];
  for (const m of s.matchAll(new RegExp(`((?:${ATTR}\\s*)+)fn\\s+([A-Za-z_]\\w*)\\s*\\(`, 'g'))) {
    const stage = /@\s*(compute|vertex|fragment)\b/.exec(m[1]);
    if (!stage) continue;
    const [params, after] = balanced(s, m.index + m[0].length - 1, '(', ')');
    const rest = s.slice(after);
    const arrow = /^\s*->\s*/.exec(rest);
    let ret = null;
    if (arrow) {
      let t = rest.slice(arrow[0].length);
      const end = t.indexOf('{');
      t = t.slice(0, end < 0 ? undefined : end).trim();
      const lead = new RegExp(`^((?:${ATTR}\\s*)*)`).exec(t)[1];
      ret = decl(`${lead} ret: ${t.slice(lead.length)}`);
    }
    entries.push({stage: stage[1], name: m[2], params: splitTop(params).map(decl), ret});
  }
  const enables = [...s.matchAll(/\benable\s+[^;]+;/g)].map(x => x[0]);
  return {structs, aliases, entries, enables};
}

/// the IO of a list of declarations, structs expanded: [{location, type, interpolate, builtin}]
function io(mod, decls) {
  const out = [];
  for (const d of decls) {
    const ty = mod.aliases[d.type] ?? d.type;
    if (mod.structs[ty]) out.push(...io(mod, mod.structs[ty]));
    else out.push({location: d.attrs.location, builtin: d.attrs.builtin, interpolate: d.attrs.interpolate, type: ty});
  }
  return out;
}

/// 'f' / 'i' / 'u' of a scalar or vector type (f16 counts as 'f'), and its components
function kind(t) {
  const m = /^(?:vec([234])\s*<\s*(\w+)\s*>|vec([234])([fihu])|(\w+))$/.exec(t.replace(/\s+/g, ''));
  if (!m) return {k: 'f', n: 4};
  const n = Number(m[1] || m[3] || 1);
  const s = m[2] || m[4] || m[5];
  const k = /^(i|i32)$/.test(s) ? 'i' : /^(u|u32)$/.test(s) ? 'u' : 'f';
  return {k, n};
}

function vertexFormat(t) {
  const {k, n} = kind(t);
  const base = {f: 'float32', i: 'sint32', u: 'uint32'}[k];
  return n === 1 ? base : `${base}x${n}`;
}

function dummyVertex(mod, inputs) {
  const members = ['@builtin(position) wgsl_ref_pos: vec4f'];
  for (const x of inputs) {
    if (x.location === undefined) continue;
    const interp = x.interpolate !== undefined ? ` @interpolate(${x.interpolate})` : '';
    members.push(`@location(${x.location})${interp} m${x.location}: ${x.type}`);
  }
  return `${mod.enables.join('\n')}\nstruct WgslRefOut { ${members.join(', ')} }\n` +
    '@vertex fn wgsl_ref_vertex() -> WgslRefOut { var o: WgslRefOut; o.wgsl_ref_pos = vec4f(0.0, 0.0, 0.0, 1.0); return o; }\n';
}

/// the pipelines to try: [{entry, stage, desc}] where desc is a plain object, modules by name
function pipelines(mod) {
  const list = [];
  const firstVertex = mod.entries.find(e => e.stage === 'vertex');
  for (const e of mod.entries) {
    if (e.stage === 'compute') {
      list.push({entry: e.name, stage: 'compute', compute: {entryPoint: e.name}});
    } else if (e.stage === 'vertex') {
      const ins = io(mod, e.params).filter(x => x.location !== undefined);
      const buffers = ins.map(x => ({arrayStride: 16, attributes: [{shaderLocation: Number(x.location), offset: 0, format: vertexFormat(x.type)}]}));
      list.push({entry: e.name, stage: 'vertex', vertex: {entryPoint: e.name, buffers}});
    } else {
      const ins = io(mod, e.params);
      const outs = e.ret ? io(mod, [e.ret]) : [];
      const targets = [];
      for (const o of outs) {
        if (o.location === undefined) continue;
        const loc = Number(o.location);
        while (targets.length <= loc) targets.push(null);
        targets[loc] = {format: {f: 'r32float', i: 'r32sint', u: 'r32uint'}[kind(o.type).k]};
      }
      let vertex;
      if (firstVertex) {
        const vins = io(mod, firstVertex.params).filter(x => x.location !== undefined);
        vertex = {entryPoint: firstVertex.name, buffers: vins.map(x => ({arrayStride: 16,
          attributes: [{shaderLocation: Number(x.location), offset: 0, format: vertexFormat(x.type)}]}))};
      } else {
        vertex = {dummy: dummyVertex(mod, ins), entryPoint: 'wgsl_ref_vertex', buffers: []};
      }
      list.push({entry: e.name, stage: 'fragment', vertex, fragment: {entryPoint: e.name, targets}});
    }
  }
  return list;
}

// ---------------------------------------------------------------- in the page

async function pageCheck(arg) {
  const {code, plan, constants, allFeatures} = arg;
  const g = globalThis;
  if (!g.wgslRef) g.wgslRef = {devices: {}};
  const st = g.wgslRef;
  const want = allFeatures ? 'all' : 'core';
  let dev = st.devices[want];
  if (!dev || dev.lost) {
    const adapter = await navigator.gpu.requestAdapter(); // an adapter gives one device
    const requiredFeatures = want === 'all' ? [...adapter.features] : [];
    const d = await adapter.requestDevice({requiredFeatures});
    dev = {d, lost: false, uncaptured: []};
    d.lost.then(() => { dev.lost = true; });
    d.addEventListener('uncapturederror', ev => dev.uncaptured.push(String(ev.error && ev.error.message)));
    st.devices[want] = dev;
  }
  const device = dev.d;
  const scoped = async fn => {
    device.pushErrorScope('internal');
    device.pushErrorScope('validation');
    let exception = null, value = null;
    try {
      value = await fn();
    } catch (e) {
      exception = String(e && (e.message || e));
    }
    const v = await device.popErrorScope();
    const i = await device.popErrorScope();
    const errors = [];
    if (exception) errors.push({kind: 'exception', text: exception});
    if (v) errors.push({kind: 'validation', text: v.message});
    if (i) errors.push({kind: 'internal', text: i.message});
    return {value, errors};
  };
  const res = {messages: [], moduleErrors: [], pipelines: []};
  const mod = await scoped(async () => {
    const m = device.createShaderModule({code});
    const info = await m.getCompilationInfo();
    res.messages = info.messages.map(x => ({type: x.type, lineNum: x.lineNum, linePos: x.linePos, offset: x.offset,
                                           length: x.length, text: x.message}));
    return m;
  });
  res.moduleErrors = mod.errors;
  const compileError = res.messages.some(x => x.type === 'error') || mod.errors.length > 0;
  if (compileError || !mod.value) return res;
  const m = mod.value;
  for (const p of plan) {
    const r = await scoped(async () => {
      if (p.stage === 'compute') {
        device.createComputePipeline({layout: 'auto', compute: {module: m, entryPoint: p.compute.entryPoint, constants}});
        return;
      }
      const vmod = p.vertex && p.vertex.dummy ? device.createShaderModule({code: p.vertex.dummy}) : m;
      const desc = {
        layout: 'auto',
        vertex: {module: vmod, entryPoint: p.vertex.entryPoint, buffers: p.vertex.buffers, constants: p.vertex.dummy ? {} : constants},
        depthStencil: {format: 'depth32float', depthWriteEnabled: false, depthCompare: 'always'},
      };
      if (p.fragment) desc.fragment = {module: m, entryPoint: p.fragment.entryPoint, targets: p.fragment.targets, constants};
      device.createRenderPipeline(desc);
    });
    res.pipelines.push({entry: p.entry, stage: p.stage, ok: r.errors.length === 0, errors: r.errors});
  }
  return res;
}

async function main() {
  const a = parseArgs(process.argv.slice(2));
  const files = collect(a.inputs);
  const base = resolve(a.base ?? (statSync(a.inputs[0]).isDirectory() ? a.inputs[0] : '.'));
  const pw = loadPlaywright();
  const flags = a.unsafe ? ['--enable-unsafe-webgpu'] : [];
  const browser = await pw.chromium.launch({headless: !a.headed, args: flags, ...chromiumOptions()});
  const page = await browser.newPage();
  await page.route('https://wgsl-ref.invalid/**', r => r.fulfill({contentType: 'text/html', body: '<!doctype html><title>wgsl_ref</title>'}));
  await page.goto('https://wgsl-ref.invalid/');
  const env = await page.evaluate(async () => {
    if (!navigator.gpu) return {error: 'no navigator.gpu (WebGPU off or no secure context)'};
    const ad = await navigator.gpu.requestAdapter();
    if (!ad) return {error: 'requestAdapter gave null'};
    const i = ad.info;
    return {adapter: {vendor: i.vendor, architecture: i.architecture, device: i.device, description: i.description,
                      isFallbackAdapter: !!i.isFallbackAdapter},
            features: [...ad.features].sort(), wgslLanguageFeatures: [...navigator.gpu.wgslLanguageFeatures].sort()};
  });
  if (env.error) throw new Error(`Chrome ${browser.version()}: ${env.error}`);
  const out = {tool: 'tools/wgsl_ref.mjs', chrome: browser.version(), headless: !a.headed, flags, ...env, files: {}};
  const mismatches = [];
  const counts = {files: 0, valid: 0, error: 0, 'pipeline-error': 0};
  for (const f of files) {
    const src = readFileSync(f, 'utf8');
    const h = header(src);
    const mod = readModule(src);
    const plan = pipelines(mod);
    const r = await page.evaluate(pageCheck, {code: src, plan, constants: h.constants,
                                              allFeatures: h.allFeatures || mod.enables.length > 0});
    const verdict = r.messages.some(x => x.type === 'error') || r.moduleErrors.length ? 'error'
      : r.pipelines.some(p => !p.ok) ? 'pipeline-error' : 'valid';
    const key = relative(base, resolve(f)).split(sep).join('/');
    const entry = {rule: h.rule, expect: h.expect, uniformity: h.uniformity, verdict, messages: r.messages};
    // createShaderModule's own error repeats getCompilationInfo's: kept only when it adds one
    if (r.moduleErrors.length && !r.messages.some(x => x.type === 'error')) entry.moduleErrors = r.moduleErrors;
    entry.pipelines = r.pipelines;
    out.files[key] = entry;
    counts.files++;
    counts[verdict]++;
    if (h.expect && h.expect !== verdict) mismatches.push({file: key, expect: h.expect, verdict});
  }
  out.summary = {...counts, mismatches};
  await browser.close();
  const text = JSON.stringify(out, null, 1) + '\n';
  if (a.out) {
    writeFileSync(a.out, text);
    console.error(`wgsl_ref: ${counts.files} files (${counts.valid} valid, ${counts.error} error, ` +
                  `${counts['pipeline-error']} pipeline-error), ${mismatches.length} not as expected -> ${a.out}`);
  } else process.stdout.write(text);
}

main().catch(e => {
  console.error(`wgsl_ref: ${e.message}`);
  process.exit(1);
});
