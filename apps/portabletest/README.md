# Portable backend conformance

Build with `python build_rs.py app portabletest --target all --out build/portable-work`.
Run the native executable, then `node apps/portabletest/tests/compare.mjs build/portable-work`
from `RustLib/`. The comparison uses Node's standard WebAssembly engine and built-in modules;
it does not install any JavaScript packages.

The same Rust test corpus serializes every scalar `Simd` operation and compares it with
SSE2 and AVX2 on native x86-64 hardware and with NEON on ARM64, and the portable `rt::fx`
with the instructions of either CPU. It covers lane layout,
saturation, wrapping arithmetic, large shifts, shuffles, pixel packs, memory helpers at odd
addresses, IEEE conversions and square roots, float arithmetic on raw bits (every pair of a
table of signed zeros, denormals, infinities and quiet and signalling NaNs, with SSE's NaN
rules as the expected result), decimal scaling, allocator reuse, and supported transcendental
error bounds. A failure names the operation with its input and both results. The browser
run and the ARM64 run must produce the exact same final digest.

ARM64 runs as a static Linux program under `qemu-aarch64` in WSL (ARM2 of
[ARM64.md](../../docs/ARM64.md)), or from `adb shell` on a device; the x86-64 Android emulator runs
SSE2 again, no NEON:

```text
python build_rs.py run portabletest --target linux --arch aarch64
python build_rs.py app portabletest --target android --android-abi arm64-v8a
python apps/portabletest/tests/android.py --serial SERIAL --digest DIGEST_OF_THE_NATIVE_RUN
```

`portabletest --math-dump` prints `rt::x87` and `rt::portable::math` for a fixed set of inputs;
`python apps/portabletest/tests/math_diff.py` compares the dump of the x86-64 build (the x87 unit)
with the one of the ARM64 build (`portable/math.rs`) and both with the C library: the deviations
ARM64.md lists under ARM2.

Additional browser checks exercise virtual
files, temporary-versus-durable save status, locks, and disabled-by-default agent access.
The request tests distinguish durable persistence, exported downloads, quota errors,
denied access and cancellation. They reject duplicate acknowledgements and prevent
old requests from marking edited or deleted/recreated files as saved. More than 512
completed saves verify bounded task-history eviction; 256 simultaneous pending requests
still fail cleanly at the limit. Cooperative jobs are checked for progress, cancellation
and stopping on drop. Explicit agent activation/revocation reaches the owned app runtime.

The x86 implementations and their instruction order are unchanged. Portable decimal
scaling uses a software 64-bit significand with nearest-even intermediate rounding, as
the native x87 path does. Portable elementary transcendentals use explicit binary64
series: they are compared within the stated numeric tolerances, not claimed bit-identical
to arbitrary x87 transcendental instructions. Large-angle trigonometric reduction is a
remaining precision limitation; the UI/effect representative tests cover -20 to +20 radians.

Real canvas/browser acceptance is separate:

```text
node tools/tests/web_apps.mjs build/office-universal --engine=chromium,firefox --only=template,gallery,jfWord,Kritzel,PaintCraft
python tools/tests/web_transfer.py build/office-universal
```

The runner uses the installed Playwright module (Node's module resolution, or the package
directory in `PLAYWRIGHT_MODULE`) and Firefox from `build/browser-engines`
(`PLAYWRIGHT_BROWSERS_PATH`). For Chromium, `chromiumOptions()` in
`tools/tests/playwright.mjs` takes `CHROME_PATH` (a Chrome or Chromium executable) if set,
otherwise an installed Google Chrome (Playwright's `chrome` channel), otherwise the Chromium
Playwright installed (`npx playwright install chromium`). It writes screenshots and structured
results below the selected build directory. It serves the checked-out shared host adapter
with the built WASM apps. IME and paste tests drive DOM events and the real worker, without modifying
the user's operating-system clipboard; they do not automate a particular OS IME.
The canvas checks compare the shared Rust PNG resource with the presented frame pixel
for pixel. The transfer test opens the generated browser DOCX files in the Windows and
Linux agents, checks text/formatting/sources, and requires byte-identical DOCX exports.
The private browser test context injects IndexedDB quota/access errors and withholds
real transaction completion callbacks. It verifies that failed saves, edits and document
replacement during a pending save never clear a newer document's dirty state. Keyboard
tests also drive format transfer/reset through the DOM and shared worker dispatcher.
The installed fonts still determine glyph coverage independently of Unicode input;
the bundled browser fonts currently lack CJK glyphs.
