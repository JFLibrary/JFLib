# Audio-Test

The demo and test app of the library's sound output (`plat::audio`, `ui.beep`;
[GUIDE.md, section 33](../../docs/GUIDE.md)): a tone generator (sine or square, frequency,
volume) on a fill stream with its state, device, latency and underruns, the chime of the app's
assets as WAV and Ogg Vorbis (or a file the user opens), the system's sounds, and a dialog that
plays the warning sound when it opens. Made with `python build_rs.py new audiotest --title
"Audio-Test"` on 2026-10-22, then adapted.

## Build, run, test

```sh
python build_rs.py run audiotest                      # Windows (Linux: --target linux)
python build_rs.py test audiotest                     # tests/*.uis on Windows and Linux (WSL)
python build_rs.py app audiotest --target web         # then:
node apps/audiotest/tests/audio_web.mjs build --engines=chromium,firefox
python build_rs.py app audiotest --target android --android-abi x86_64   # then:
python apps/audiotest/tests/android_audio.py --serial emulator-5554 --apk build/android/audiotest-x86_64-debug.apk
```

| Test | What it checks |
|---|---|
| `tests/smoke.uis` | the UI headless; `expect sound` for the system sounds, the dialog's sound and the switch "Klänge an" |
| `tests/play.uis` (`--test play`) | push and fill streams with the real output: written as the ring takes it, `finish` -> Ended, latency, pause / start / stop, volume, a device change, `play` / `play_file`; Windows: WASAPI loopback hears the tones at their frequency |
| `tests/sounds.uis` (`--test sounds`) | the system sounds: dry run, every kind, Windows: what `PlaySound` played reaches the device |
| `tests/pulse.uis` (`--test pulse`) | Linux: the PulseAudio server (WSLg's, pipewire-pulse) lists the stream, corks it while paused, reports its latency; a system sound from a scratch sound theme |
| `tests/audio_web.mjs` | Chromium and Firefox: an AnalyserNode hears the 440 Hz tone through the SharedArrayBuffer ring and through posted buffers, the chime; `Blocked` until a click |
| `tests/android_audio.py` | an emulator or device: audioserver plays the app's AAudio track; the self-test (`jfgui_SELFTEST=audio`, `src/selftest.rs`) passes |

Without an output device the device tests report `SKIP`, not a failure. Their tones are quiet
(-26 dBFS) and short, but audible.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the app: generator, sounds, dialog |
| `src/tone.rs` | the generator's fill (frequency and volume glide, no clicks) |
| `src/tests.rs` | `--test play / sounds / pulse` |
| `src/selftest.rs` | the self-test on UI timers (Android) |
| `assets/klang.wav`, `assets/klang.oga` | the chime (`app://`) |
| `android/` | the Android manifest and icon (the shared Java host of `android/`) |
