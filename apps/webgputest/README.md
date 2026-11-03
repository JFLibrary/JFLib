# webgputest

`lib/gpu`'s WebGPU backend behind its API, in the browser (docs/GPU.md "Ausgabe im Browser"). A program
without a window: its checks run on the app's worker, step by step, as the browser answers, and say
`ok` / `FAIL` on the console.

```text
python build_rs.py app webgputest --target web
node apps/webgputest/tests/api_web.mjs build --engines=chromium,firefox
```

`tests/api_web.mjs` starts it in each of the three bundles (scalar, SIMD, threads) and reads the lines. A
browser without a working WebGPU says `SKIP` (Playwright's Firefox here: its build blocks WebGPU).
The UI's output through the same backend: `apps/gallery/tests/gpu_web.mjs`.
