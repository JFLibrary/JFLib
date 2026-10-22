// Sound output of the shared web host (lib/plat/web_audio.rs). One module in three places:
//   page (loader.js)    AudioHost: the AudioContext (made for the first stream; browsers keep it
//                       suspended until a user gesture, the first pointer or key event resumes
//                       it), one AudioWorkletNode per stream and a MessagePort from it to the
//                       worker; window.jfguiAudio (read-only, for tests): the context and nodes
//   worker (worker.js)  WorkerAudio: the pump. Every 10 ms it asks WASM (jfgui_audio_pull) for
//                       the frames a stream's worklet lacks and moves them into a
//                       SharedArrayBuffer ring (cross-origin isolated pages) or posts them; the
//                       frames the worklet played pace it
//   AudioWorklet        the processor "jfgui-audio": plays from the ring or the posted chunks,
//                       silence when nothing is there, reports what it played
// Requests (REQUEST_AUDIO, five little-endian u32): 1 open (id, rate, channels, latency
// frames), 2 close, 3 play, 4 pause, 5 flush.
const NAME = "jfgui-audio";

if (typeof registerProcessor === "function") {
  registerProcessor(NAME, class extends AudioWorkletProcessor {
    constructor(options) {
      super();
      this.ch = options.processorOptions.channels;
      this.chunks = []; this.at = 0; // posted chunks (interleaved), the frame read in the first
      this.ring = null; this.played = 0; this.told = 0; this.paused = false; this.alive = true; this.worker = null;
      this.port.onmessage = ({data}) => {
        if (data.type === "port") { this.worker = data.port; this.worker.onmessage = e => this.message(e.data); }
        else this.message(data);
      };
    }
    message(m) {
      if (m.type === "ring") this.ring = {data: new Float32Array(m.sab, 8), idx: new Int32Array(m.sab, 0, 2), mask: m.frames - 1};
      else if (m.type === "chunk") this.chunks.push(m.data);
      else if (m.type === "pause") this.paused = m.on;
      else if (m.type === "flush") {
        // dropped frames count as played: the pump sees an empty queue
        for (const c of this.chunks) this.played += c.length / this.ch;
        this.played -= this.at; this.chunks = []; this.at = 0;
        if (this.ring) Atomics.store(this.ring.idx, 1, Atomics.load(this.ring.idx, 0));
        this.report(true);
      } else if (m.type === "close") this.alive = false;
    }
    report(force) {
      if (this.worker && !this.ring && (force || this.played - this.told >= 512)) { this.worker.postMessage({played: this.played}); this.told = this.played; }
    }
    process(inputs, outputs) {
      const out = outputs[0], n = out[0].length, ch = this.ch;
      let got = 0;
      if (!this.paused && this.ring) {
        const {data, idx, mask} = this.ring;
        let r = Atomics.load(idx, 1);
        got = Math.max(0, Math.min(n, (Atomics.load(idx, 0) - r) | 0));
        for (let i = 0; i < got; i++, r++) {
          const p = (r & mask) * ch;
          for (let c = 0; c < out.length; c++) out[c][i] = data[p + Math.min(c, ch - 1)];
        }
        Atomics.store(idx, 1, r);
      } else if (!this.paused) {
        while (got < n && this.chunks.length) {
          const chunk = this.chunks[0], frames = chunk.length / ch, k = Math.min(n - got, frames - this.at);
          for (let i = 0; i < k; i++) {
            const p = (this.at + i) * ch;
            for (let c = 0; c < out.length; c++) out[c][got + i] = chunk[p + Math.min(c, ch - 1)];
          }
          got += k; this.at += k;
          if (this.at >= frames) { this.chunks.shift(); this.at = 0; }
        }
      }
      for (const o of out) o.fill(0, got);
      this.played += got;
      this.report(false);
      return this.alive;
    }
  });
}

/** The page's side: the context, the nodes, the gestures that unblock sound. */
export class AudioHost {
  constructor(worker) {
    this.worker = worker; this.ctx = null; this.loading = null; this.nodes = new Map(); this.closed = new Set(); this.mode = "";
    const resume = () => { if (this.ctx && this.ctx.state === "suspended") this.ctx.resume().catch(() => {}); };
    for (const kind of ["pointerdown", "keydown", "touchend"]) addEventListener(kind, resume, {capture: true});
    const host = this;
    Object.defineProperty(window, "jfguiAudio", {value: Object.freeze({get context() { return host.ctx; }, get mode() { return host.mode; }, nodes: this.nodes})});
  }
  state() {
    const c = this.ctx;
    return {type: "audio-state", blocked: !c || c.state !== "running", base: Math.round(((c?.baseLatency || 0) + (c?.outputLatency || 0)) * 1e6)};
  }
  context() {
    if (!this.loading) {
      const Context = globalThis.AudioContext || globalThis.webkitAudioContext;
      if (!Context) return Promise.reject(new Error("WebAudio fehlt"));
      this.ctx = new Context({latencyHint: "interactive"});
      this.ctx.onstatechange = () => this.worker.postMessage(this.state());
      this.ctx.resume().catch(() => {}); // plays at once where the page may (an earlier gesture)
      this.loading = this.ctx.audioWorklet ? this.ctx.audioWorklet.addModule(new URL("audio.js", import.meta.url)) : Promise.reject(new Error("AudioWorklet fehlt"));
    }
    return this.loading.then(() => this.ctx);
  }
  request(data) {
    const v = new DataView(data), op = v.getUint32(0, true), id = v.getUint32(4, true);
    if (op === 1) return this.open(id, v.getUint32(8, true), v.getUint32(12, true), v.getUint32(16, true));
    const node = this.nodes.get(id);
    if (!node) { if (op === 2) this.closed.add(id); return; }
    if (op === 2) { node.port.postMessage({type: "close"}); node.disconnect(); this.nodes.delete(id); }
    else if (op === 3 || op === 4) node.port.postMessage({type: "pause", on: op === 4});
    else if (op === 5) node.port.postMessage({type: "flush"});
  }
  async open(id, rate, channels, latency) {
    try {
      const ctx = await this.context();
      if (this.closed.delete(id)) return;
      const node = new AudioWorkletNode(ctx, NAME, {numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [channels], processorOptions: {channels}});
      node.connect(ctx.destination);
      const {port1, port2} = new MessageChannel();
      node.port.postMessage({type: "port", port: port2}, [port2]);
      this.nodes.set(id, node);
      const target = Math.max(512, Math.ceil(latency * ctx.sampleRate / rate));
      const shared = globalThis.crossOriginIsolated === true && typeof SharedArrayBuffer === "function";
      this.mode = shared ? "ring" : "posted";
      this.worker.postMessage({...this.state(), type: "audio-port", id, port: port1, rate: ctx.sampleRate, channels, target, shared}, [port1]);
    } catch (error) {
      console.warn("Audio:", error);
      this.worker.postMessage({type: "audio-port", id, error: String(error?.message || error)});
    }
  }
}

/** The worker's side: the pump between WASM and the worklets. */
export class WorkerAudio {
  constructor(exports, memory) { this.exports = exports; this.memory = memory; this.streams = new Map(); this.timer = 0; }
  message(m) {
    if (m.type === "audio-state") {
      for (const s of this.streams.values()) { s.blocked = m.blocked; s.base = m.base; }
      this.pump(); return;
    }
    const w = this.exports();
    if (m.error || !m.port) { w.jfgui_audio_ready(m.id, 0, 0); return; }
    const s = {id: m.id, port: m.port, ch: m.channels, target: m.target, sent: 0, played: 0, blocked: m.blocked, base: m.base, ended: false, ring: null};
    if (m.shared) {
      let frames = 1024;
      while (frames < m.target * 2) frames *= 2;
      const sab = new SharedArrayBuffer(8 + frames * m.channels * 4);
      s.ring = {idx: new Int32Array(sab, 0, 2), data: new Float32Array(sab, 8), mask: frames - 1};
      m.port.postMessage({type: "ring", sab, frames});
    }
    m.port.onmessage = ({data}) => { if (typeof data.played === "number") s.played = data.played; };
    this.streams.set(m.id, s);
    w.jfgui_audio_ready(m.id, m.rate, 1);
    if (!this.timer) this.timer = setInterval(() => this.pump(), 10);
    this.pump();
  }
  pump() {
    const w = this.exports();
    for (const s of this.streams.values()) {
      const played = (s.ring ? Atomics.load(s.ring.idx, 1) : s.played) >>> 0;
      const queued = (s.sent - played) >>> 0;
      const drained = s.ended && queued === 0;
      const want = s.ended ? 0 : Math.max(0, s.target - queued);
      let n = w.jfgui_audio_pull(s.id, want >= 128 ? want : 0, played, s.base, (s.blocked ? 1 : 0) | (drained ? 2 : 0)) >>> 0;
      if (n === 0xFFFFFFFF) { s.port.close(); this.streams.delete(s.id); continue; } // closed
      if (drained) s.ended = false;
      const end = n >= 0x80000000; n &= 0x7FFFFFFF;
      if (n) {
        const src = new Float32Array(this.memory().buffer, w.jfgui_audio_data() >>> 0, n * s.ch);
        if (s.ring) {
          const {idx, data, mask} = s.ring, at = Atomics.load(idx, 0), start = at & mask, first = Math.min(n, mask + 1 - start);
          data.set(src.subarray(0, first * s.ch), start * s.ch);
          if (first < n) data.set(src.subarray(first * s.ch), 0);
          Atomics.store(idx, 0, (at + n) | 0);
        } else {
          const chunk = src.slice();
          s.port.postMessage({type: "chunk", data: chunk}, [chunk.buffer]);
        }
        s.sent = (s.sent + n) >>> 0;
      }
      if (end) s.ended = true;
    }
    if (!this.streams.size && this.timer) { clearInterval(this.timer); this.timer = 0; }
  }
}
