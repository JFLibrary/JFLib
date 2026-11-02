/* The UI through the GPU on a device or emulator (lib/ui/gpu_out.rs, lib/plat/android_gpu.rs,
 * GalleryActivity's SurfaceView); run by apps/androidGallery/tests/android_gpu.py, one process per phase:
 *   adb shell am instrument -w -r -e phase NAME org.jflib.gallery/.GalleryGpuInstrumentation
 * phases:
 *   path      the first frames in the Bitmap, then the swapchain, the frame size the view's; every
 *             presented image read back = the framebuffer (jfgui_GPU_VERIFY), the composed screen = the
 *             Bitmap's picture; partial frames, a full frame, a menu (a layer), a size change (root
 *             padding: what the keyboard does; the real keyboard where one comes), rotation (no loop of
 *             rebuilds: SUBOPTIMAL means nothing), the activity going away and back (the frame in the
 *             Bitmap meanwhile), a lost device and a new one, a tool window (the CPU path while it
 *             shows), idle (no frame).  No usable GPU: skip=1, not a failure.
 *   images    the page "Bild & Leinwand": a canvas pans from tiles, a big image becomes a texture
 *   nogpu     jfgui_GPU_DLL names no library: the CPU path, the resolution the Bitmap path has, it works
 *   off       jfgui_GPU=0: no SurfaceView, the same resolution
 *   bench     numbers, one round (-e gpu 0|1 -e res cap|full, -e landscape 1): idle CPU and wake-ups, memory, a full frame,
 *             a small change to the screen, a finger's scroll, and with -e page images a canvas pan; what
 *             reached the screen and when: dumpsys SurfaceFlinger --frametimeline.  No checks.
 * The result has the counters and "skip".
 */
package org.jflib.gallery;

import android.app.Activity;
import android.app.Instrumentation;
import android.app.UiAutomation;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.graphics.Bitmap;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.View;
import android.view.ViewGroup;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.inputmethod.InputMethodManager;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.function.Supplier;

public final class GalleryGpuInstrumentation extends Instrumentation {
    private int checks;
    private String phase;
    private Bundle args;
    private Activity activity;
    private View view;

    @Override public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        phase = arguments != null ? arguments.getString("phase", "path") : "path";
        args = arguments != null ? arguments : new Bundle();
        start();
    }
    private void check(boolean condition, String what) {
        if (!condition) throw new AssertionError(what);
        checks++;
    }
    private static Object field(Object object, String name) throws Exception {
        Field f = object.getClass().getDeclaredField(name);
        f.setAccessible(true);
        return f.get(object);
    }
    private static boolean flag(Class<?> c, String name) throws Exception {
        Field f = c.getDeclaredField(name);
        f.setAccessible(true);
        return f.getBoolean(null);
    }
    // the output's counters, on the UI thread
    private HashMap<String, String> status(int op) {
        byte[][] out = new byte[1][];
        runOnMainSync(() -> out[0] = GalleryActivity.nativeGpuTest(op));
        HashMap<String, String> m = new HashMap<>();
        if (out[0] != null)
            for (String kv : new String(out[0], StandardCharsets.UTF_8).split(" ")) {
                int eq = kv.indexOf('=');
                if (eq > 0) m.put(kv.substring(0, eq), kv.substring(eq + 1));
            }
        return m;
    }
    private HashMap<String, String> status() { return status(0); }
    private static long num(HashMap<String, String> s, String key) {
        String v = s.get(key);
        return v == null ? -1 : Long.parseLong(v);
    }
    private boolean until(long ms, Supplier<Boolean> probe) {
        long end = SystemClock.uptimeMillis() + ms;
        while (SystemClock.uptimeMillis() < end) {
            if (probe.get()) return true;
            frames();
            SystemClock.sleep(30);
        }
        return probe.get();
    }
    private void frames() { runOnMainSync(GalleryActivity::frameNow); }
    private boolean onGpu() { return num(status(), "on") == 1; }
    private void noBadFrames(String when) {
        HashMap<String, String> s = status();
        check(num(s, "verify_bad") == 0, when + ": every presented image = the framebuffer (" + s.get("verified") + " read back, " + s.get("verify_bad") + " differ)");
    }
    private int[] bitmapNow() throws Exception {
        Bitmap frame = (Bitmap)field(view, "frame");
        int[] px = new int[frame.getWidth() * frame.getHeight()];
        frame.getPixels(px, 0, frame.getWidth(), 0, 0, frame.getWidth(), frame.getHeight());
        return px;
    }
    // what the display composes in the view's rectangle against pixels (w x h, RGBA ints), when the
    // frame is 1:1: place and orientation, no more (a phone's screenshot comes in the display's colour
    // space, a few levels off in the conversion: android_gpu.py compares the screen of both paths byte
    // for byte from outside) -> how many differ by more than 8 levels.  A system dialog over the app
    // (an emulator's "not responding") is none of ours: looked at again
    private long screenDiffers(int[] px, int w, int h) {
        long bad = -1;
        for (int round = 0; round < 8 && bad != 0; round++) {
            SystemClock.sleep(round == 0 ? 200 : 1000);
            Bitmap shot = getUiAutomation().takeScreenshot();
            if (shot == null) return 0; // (no picture from this system: nothing compared)
            int[] at = new int[2];
            runOnMainSync(() -> view.getLocationOnScreen(at));
            if (at[0] < 0 || at[1] < 0 || at[0] + w > shot.getWidth() || at[1] + h > shot.getHeight()) {
                bad = Long.MAX_VALUE;
                continue;
            }
            int[] got = new int[w * h];
            shot.getPixels(got, 0, w, at[0], at[1], w, h);
            bad = 0;
            for (int i = 0; i < got.length; i++) {
                int a = got[i], b = px[i];
                if (Math.abs((a >> 16 & 255) - (b >> 16 & 255)) > 8 || Math.abs((a >> 8 & 255) - (b >> 8 & 255)) > 8 || Math.abs((a & 255) - (b & 255)) > 8) bad++;
            }
        }
        return bad;
    }
    private AccessibilityNodeInfo find(AccessibilityNodeInfo root, String text) {
        if (root == null) return null;
        CharSequence t = root.getText() != null ? root.getText() : root.getContentDescription();
        if (t != null && t.toString().startsWith(text)) return root;
        for (int i = 0; i < root.getChildCount(); i++) {
            AccessibilityNodeInfo found = find(root.getChild(i), text);
            if (found != null) return found;
        }
        return null;
    }
    private AccessibilityNodeInfo node(UiAutomation ui, String text) {
        AccessibilityNodeInfo[] n = new AccessibilityNodeInfo[1];
        until(4000, () -> (n[0] = find(ui.getRootInActiveWindow(), text)) != null);
        if (n[0] == null) throw new AssertionError("no node \"" + text + "\"");
        return n[0];
    }
    // a press where the accessibility tree says the widget is (no touch injected: the events go in as
    // the View would send them for a mouse)
    private void click(UiAutomation ui, String text) throws Exception { click(ui, text, false); }
    private void click(UiAutomation ui, String text, boolean rightEnd) throws Exception {
        android.graphics.Rect r = new android.graphics.Rect();
        node(ui, text).getBoundsInScreen(r);
        int[] at = new int[2];
        runOnMainSync(() -> view.getLocationOnScreen(at));
        float scale = ((Bitmap)field(view, "frame")).getWidth() / (float)view.getWidth();
        int x = Math.round(((rightEnd ? r.right - 16 : r.centerX()) - at[0]) * scale), y = Math.round((r.centerY() - at[1]) * scale);
        runOnMainSync(() -> { GalleryActivity.nativeEvent(1, 0, 0, x, y, 0); GalleryActivity.nativeEvent(2, 1, 0, x, y, 0); });
        frames();
        SystemClock.sleep(60);
        runOnMainSync(() -> GalleryActivity.nativeEvent(3, 1, 0, x, y, 0));
        frames();
        SystemClock.sleep(60);
        checks++;
    }
    private void stream(Bundle result, HashMap<String, String> s) {
        for (HashMap.Entry<String, String> e : s.entrySet()) result.putString("gpu_" + e.getKey(), e.getValue());
    }

    private Intent start(String... extras) {
        Intent intent = new Intent(getTargetContext(), GalleryActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        intent.putExtra("jfgui_LANG", "de");
        for (int i = 0; i + 1 < extras.length; i += 2) intent.putExtra(extras[i], extras[i + 1]);
        return intent;
    }

    // ---- the phase "path"
    private void path(Bundle result, UiAutomation ui) throws Exception {
        if (!until(30000, () -> { HashMap<String, String> s = status(); return num(s, "on") == 1 || num(s, "never") == 1; })) throw new AssertionError("neither the swapchain nor a verdict in 30 s: " + status());
        HashMap<String, String> s = status();
        if (num(s, "never") == 1) {
            result.putInt("skip", 1);
            result.putString("skip_why", s.get("error"));
            check(!flag(GalleryActivity.class, "fullResolution") || "full".equals(activity.getIntent().getStringExtra("jfgui_GPU_RES")), "no usable GPU: the resolution the Bitmap path has");
            return;
        }
        // ---- on the swapchain; the frame size is the view's
        Bitmap frame = (Bitmap)field(view, "frame");
        int w = frame.getWidth(), h = frame.getHeight();
        check(w == view.getWidth() && h == view.getHeight(), "the frame has the view's pixels: " + w + "x" + h + ", view " + view.getWidth() + "x" + view.getHeight());
        check(s.get("fb").equals(w + "x" + h), "the framebuffer has them too: " + s.get("fb"));
        check(num(s, "cpu_frames") >= 1, "the first frame went through the CPU path");
        noBadFrames("after the switch");
        // the screen: a static start page, the Bitmap holds its picture from the CPU frames before the switch
        int[] picture = bitmapNow();
        long bad = screenDiffers(picture, w, h);
        result.putLong("screen_differs", bad);
        check(bad == 0, "the composed screen shows the CPU path's picture: " + bad + " pixels differ");
        int[] at = new int[2];
        runOnMainSync(() -> view.getLocationOnScreen(at));
        result.putString("view_rect", at[0] + "," + at[1] + "," + w + "," + h);
        // ---- partial frames (hover), a full frame
        long f0 = num(status(), "frames");
        for (int i = 0; i < 40; i++) {
            final int k = i;
            runOnMainSync(() -> GalleryActivity.nativeEvent(1, 0, 0, 40 + (k * 29) % Math.max(1, w - 80), 120 + (k * 41) % Math.max(1, h - 200), 0));
            frames();
            SystemClock.sleep(20);
        }
        status(3);
        until(2000, () -> num(status(), "frames") > f0);
        check(num(status(), "frames") > f0, "frames go through the swapchain");
        check(onGpu(), "still on the swapchain");
        noBadFrames("partial frames, a full frame");
        // ---- a menu: a layer of its own
        long up0 = num(status(), "upload_bytes");
        click(ui, "Seiten");
        check(until(4000, () -> num(status(), "layer_textures") >= 1), "the menu has a texture of its own");
        noBadFrames("menu open");
        result.putLong("menu_upload", num(status(), "upload_bytes") - up0);
        runOnMainSync(() -> { GalleryActivity.nativeEvent(5, 0, 0, 0, 0, 27); GalleryActivity.nativeEvent(6, 0, 0, 0, 0, 27); });
        check(until(4000, () -> num(status(), "layer_textures") == 0), "the menu closed: its pixels back without a new base");
        noBadFrames("menu closed");
        // ---- the size changes, as with the keyboard: CPU frames, then the swapchain again
        ViewGroup root = (ViewGroup)view.getParent();
        int pad = root.getPaddingBottom();
        long left0 = num(status(), "leaves"), sw0 = num(status(), "switches");
        runOnMainSync(() -> root.setPadding(root.getPaddingLeft(), root.getPaddingTop(), root.getPaddingRight(), pad + 300));
        check(until(5000, () -> num(status(), "leaves") > left0), "a new size: back to the CPU path");
        check(until(10000, () -> num(status(), "switches") > sw0 && onGpu()), "the new size settled: the swapchain again");
        check(status().get("fb").equals(view.getWidth() + "x" + view.getHeight()), "the framebuffer has the new size: " + status().get("fb"));
        noBadFrames("a new size");
        runOnMainSync(() -> root.setPadding(root.getPaddingLeft(), root.getPaddingTop(), root.getPaddingRight(), pad));
        check(until(10000, () -> status().get("fb").equals(view.getWidth() + "x" + view.getHeight()) && onGpu()), "and back");
        noBadFrames("the old size again");
        // ---- the real keyboard, where one comes (a phone; an emulator's image may have none)
        InputMethodManager ime = (InputMethodManager)activity.getSystemService(Context.INPUT_METHOD_SERVICE);
        long left1 = num(status(), "leaves");
        int hBefore = view.getHeight();
        runOnMainSync(() -> { view.requestFocus(); ime.showSoftInput(view, 0); });
        boolean came = until(3000, () -> view.getHeight() != hBefore);
        result.putString("keyboard", came ? "came" : "none");
        if (came) {
            check(until(10000, () -> num(status(), "leaves") > left1 && onGpu()), "the keyboard: CPU frames, then the swapchain again");
            noBadFrames("the keyboard");
            runOnMainSync(() -> ime.hideSoftInputFromWindow(view.getWindowToken(), 0));
            check(until(10000, () -> view.getHeight() == hBefore && status().get("fb").equals(view.getWidth() + "x" + view.getHeight()) && onGpu()), "the keyboard gone: the swapchain at the old size");
        }
        // ---- rotation: requested by the activity (no setting of the system changes)
        long left2 = num(status(), "leaves");
        int wBefore = view.getWidth();
        runOnMainSync(() -> activity.setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE));
        boolean turned = until(8000, () -> view.getWidth() != wBefore);
        result.putString("rotation", turned ? "turned" : "none");
        if (turned) {
            check(until(10000, () -> num(status(), "leaves") > left2 && status().get("fb").equals(view.getWidth() + "x" + view.getHeight()) && onGpu()), "turned: the swapchain at the new size");
            noBadFrames("turned");
            // SUBOPTIMAL at every present while the display is turned must not leave the swapchain
            long left3 = num(status(), "leaves"), fr3 = num(status(), "frames");
            for (int i = 0; i < 30; i++) {
                status(3);
                frames();
                SystemClock.sleep(30);
            }
            HashMap<String, String> after = status();
            result.putLong("rotated_frames", num(after, "frames") - fr3);
            check(num(after, "leaves") == left3 && onGpu(), "turned: frames stay on the swapchain (" + left3 + " -> " + after.get("leaves") + " leaves)");
            check(num(after, "frames") > fr3, "turned: frames arrive");
            noBadFrames("turned, partial frames");
            // the screen, turned: the compositor turns the upright image
            status(3);
            until(1000, () -> false);
            int[] turnedPicture = landscapePicture();
            if (turnedPicture != null) {
                long badTurned = screenDiffers(turnedPicture, view.getWidth(), view.getHeight());
                result.putLong("screen_differs_turned", badTurned);
                check(badTurned == 0, "turned: the composed screen shows the framebuffer: " + badTurned + " pixels differ");
            }
            // (not UNSPECIFIED: how a phone lies would decide; the wish ends with the activity)
            runOnMainSync(() -> activity.setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_PORTRAIT));
            boolean back = until(10000, () -> view.getWidth() == wBefore && status().get("fb").equals(view.getWidth() + "x" + view.getHeight()) && onGpu());
            check(back, "turned back: the swapchain again (view " + view.getWidth() + " of " + wBefore + ", " + status().get("fb") + ", on " + status().get("on") + ")");
        }
        // ---- the activity goes away and comes back: the surface goes and comes
        noBadFrames("before going away");
        long sw1 = num(status(), "switches");
        int[] before = null;
        runOnMainSync(() -> activity.moveTaskToBack(true));
        check(until(8000, () -> !onGpu()), "away: back to the CPU path, the swapchain gone");
        // the frame reached the Bitmap: the window may be seen before the next frame
        Bitmap kept = (Bitmap)field(view, "frame");
        before = bitmapNow();
        boolean some = false;
        for (int i = 1; i < before.length && !some; i++) some = before[i] != before[0];
        check(some && kept.getWidth() == view.getWidth(), "away: the Bitmap holds the frame");
        Intent back = new Intent(getTargetContext(), GalleryActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_REORDER_TO_FRONT);
        getTargetContext().startActivity(back);
        check(until(10000, () -> num(status(), "switches") > sw1 && onGpu()), "back: the surface came back, the swapchain again");
        noBadFrames("back");
        // ---- a lost device: the CPU path, a new device after the wait
        long left4 = num(status(), "leaves");
        status(1);
        status(3); // (a frame finds out)
        check(until(6000, () -> num(status(), "leaves") > left4), "a lost device: back to the CPU path");
        check(until(15000, () -> onGpu()), "a lost device: a new one, the swapchain again");
        noBadFrames("a new device");
        // ---- a tool window is composed on the canvas: the CPU path while it shows
        long left5 = num(status(), "leaves");
        toolWindow(ui, left5, result);
        // ---- idle: no frames
        SystemClock.sleep(600);
        HashMap<String, String> i0 = status();
        SystemClock.sleep(2000);
        HashMap<String, String> i1 = status();
        result.putLong("idle_frames", num(i1, "frames") + num(i1, "cpu_frames") - num(i0, "frames") - num(i0, "cpu_frames"));
        check(num(i1, "frames") == num(i0, "frames") && num(i1, "cpu_frames") == num(i0, "cpu_frames"), "idle: no frame in 2 s");
        check(num(i1, "verified") > 0, "frames were read back at all");
        noBadFrames("the whole run");
        stream(result, i1);
    }
    // the framebuffer turned: a static picture, through the Bitmap after a size change it may not be
    // current (the framebuffer is not reachable from here): only with a frame through the CPU path after
    // the size change; on the swapchain that was the size's last CPU frame, which holds the same page
    private int[] landscapePicture() throws Exception {
        Bitmap frame = (Bitmap)field(view, "frame");
        if (frame.getWidth() != view.getWidth() || frame.getHeight() != view.getHeight()) return null;
        return bitmapNow();
    }
    private void toolWindow(UiAutomation ui, long left, Bundle result) throws Exception {
        click(ui, "Seiten");
        click(ui, "Fenster");
        node(ui, "Werkzeugfenster");
        click(ui, "Werkzeugfenster");
        check(until(5000, () -> num(status(), "leaves") > left && !onGpu()), "a tool window: the CPU path while it shows");
        click(ui, "Werkzeugfenster", true); // (the window lies over the button's middle)
        check(until(10000, () -> onGpu()), "the tool window gone: the swapchain again");
        noBadFrames("after the tool window");
    }

    // ---- the phase "images"
    private void images(Bundle result) throws Exception {
        if (!until(30000, () -> { HashMap<String, String> s = status(); return num(s, "on") == 1 || num(s, "never") == 1; })) throw new AssertionError("no verdict: " + status());
        if (num(status(), "never") == 1) { result.putInt("skip", 1); return; }
        // a left drag over the view at the top of the page
        int x = view.getWidth() / 2, y = (int)(220 * view.getResources().getDisplayMetrics().density);
        runOnMainSync(() -> { GalleryActivity.nativeEvent(1, 0, 0, x, y, 0); GalleryActivity.nativeEvent(2, 1, 0, x, y, 0); });
        frames();
        SystemClock.sleep(50);
        long maxApart = 0;
        for (int i = 1; i <= 40; i++) {
            final int k = i;
            runOnMainSync(() -> GalleryActivity.nativeEvent(1, 0, 0, x - k * 3, y - k, 0));
            frames();
            SystemClock.sleep(20);
            maxApart = Math.max(maxApart, num(status(), "canvas_apart"));
        }
        HashMap<String, String> s = status();
        result.putLong("canvas_apart_max", maxApart);
        result.putString("canvas_tiles", s.get("canvas_tiles"));
        check(maxApart >= 1 && num(s, "canvas_tiles") > 0, "the canvas pans from tiles: " + s.get("canvas_tiles"));
        runOnMainSync(() -> GalleryActivity.nativeEvent(3, 1, 0, x - 120, y - 40, 0));
        frames();
        check(until(3000, () -> num(status(), "canvas_apart") == 0), "at rest it is the CPU's picture again");
        noBadFrames("the canvas");
        // the big picture: scrolled up and down until its redraws have paid for a texture
        long textures = 0;
        for (int i = 0; i < 400 && textures == 0; i++) {
            final int dy = i % 2 == 0 ? -120 : 120;
            final int k = i;
            runOnMainSync(() -> {
                GalleryActivity.nativeEvent(1, 0, 0, 20, view.getHeight() - 60, 0);
                GalleryActivity.nativeEvent(4, 0, 0, 20, view.getHeight() - 60, dy & 0xffff);
            });
            frames();
            SystemClock.sleep(k < 10 ? 40 : 12);
            textures = num(status(), "image_textures");
        }
        result.putLong("image_textures", textures);
        check(textures >= 1, "the big picture became a texture");
        noBadFrames("the picture");
        check(onGpu(), "still on the swapchain");
        stream(result, status());
    }

    // ---- the phase "bench"
    private static String shellOutput(UiAutomation ui, String command) throws Exception {
        android.os.ParcelFileDescriptor fd = ui.executeShellCommand(command);
        StringBuilder b = new StringBuilder();
        try (java.io.FileInputStream in = new android.os.ParcelFileDescriptor.AutoCloseInputStream(fd)) {
            byte[] buf = new byte[65536];
            for (int n; (n = in.read(buf)) > 0; ) b.append(new String(buf, 0, n, StandardCharsets.UTF_8));
        }
        return b.toString();
    }
    // what the compositor did with this app's buffers: their present times (ns, CLOCK_MONOTONIC as
    // System.nanoTime), rows { 0, present, 0 } in order.  dumpsys SurfaceFlinger --latency answers nothing on
    // Android 16; --frametimeline counts from its first frame, and a frame's token ties that to the clock: a
    // frame of the View (invalidate) has the token and the intended vsync in dumpsys gfxinfo framestats
    private long[][] latency(UiAutomation ui, long[] period) throws Exception {
        String pkg = getTargetContext().getPackageName();
        runOnMainSync(view::invalidate);
        SystemClock.sleep(120);
        HashMap<Long, Long> vsync = new HashMap<>();
        boolean head = false;
        for (String line : shellOutput(ui, "dumpsys gfxinfo " + pkg + " framestats").split("\n")) {
            if (line.startsWith("Flags,")) { head = true; continue; }
            if (!head) continue;
            if (line.startsWith("---")) { head = false; continue; }
            String[] f = line.split(",");
            if (f.length > 2) { try { vsync.put(Long.parseLong(f[1]), Long.parseLong(f[2])); } catch (NumberFormatException e) { } }
        }
        long base = Long.MIN_VALUE;
        java.util.ArrayList<Double> shown = new java.util.ArrayList<>();
        String layer = null;
        long token = 0;
        boolean buffer = false;
        double expected = 0;
        for (String raw : shellOutput(ui, "dumpsys SurfaceFlinger --frametimeline -all").split("\n")) {
            String line = raw.trim();
            if (line.startsWith("Vsync Period:") && period[0] == 0) period[0] = (long)(Double.parseDouble(line.substring(13).trim()) * 1e6);
            if (line.startsWith("Layer - ")) { layer = line; token = 0; buffer = false; continue; }
            if (layer == null) continue;
            if (line.startsWith("Token:")) { try { token = Long.parseLong(line.substring(6).trim()); } catch (NumberFormatException e) { } }
            else if (line.startsWith("Is Buffer?:")) buffer = line.endsWith("1");
            else if (line.startsWith("Expected") || line.startsWith("Actual")) {
                String[] f = line.split("\\|");
                if (f.length < 4) continue;
                boolean mine = layer.contains(pkg) && !layer.contains("Background for") && !layer.contains("Bounds for") && !layer.contains("ActivityRecord") && !layer.contains("Splash");
                if (line.startsWith("Expected")) {
                    expected = real(f[1]);
                    if (expected < 0) continue;
                    Long v = vsync.get(token);
                    if (mine && v != null && base == Long.MIN_VALUE) base = v - (long)(expected * 1e6);
                } else if (mine && buffer) {
                    double present = real(f[3]);
                    if (present > 0) shown.add(present);
                }
            } else if (line.startsWith("---")) layer = null;
        }
        if (base == Long.MIN_VALUE) return new long[0][];
        java.util.Collections.sort(shown);
        long[][] rows = new long[shown.size()][];
        for (int i = 0; i < rows.length; i++) rows[i] = new long[] { 0, base + (long)(shown.get(i) * 1e6), 0 };
        return rows;
    }
    private static double real(String v) {
        try { return Double.parseDouble(v.trim()); } catch (NumberFormatException e) { return -1; } // ("N/A")
    }
    private static long pct(long[] v, int p) {
        long[] s = v.clone();
        java.util.Arrays.sort(s);
        return s.length == 0 ? -1 : s[Math.min(s.length - 1, s.length * p / 100)];
    }
    private static String ms(long ns) { return String.format(java.util.Locale.ROOT, "%.2f", ns / 1e6); }
    // input at times[i] -> the first frame on the screen after it
    private void toScreen(Bundle r, String name, long[] times, UiAutomation ui) throws Exception {
        long[] period = new long[1];
        long[][] rows = latency(ui, period);
        long[] d = new long[times.length];
        int n = 0;
        for (long t : times) {
            long first = Long.MAX_VALUE;
            for (long[] row : rows) if (row[1] >= t && row[1] < first) first = row[1];
            if (first != Long.MAX_VALUE) d[n++] = first - t;
        }
        d = java.util.Arrays.copyOf(d, n);
        r.putString(name + "_screen_p50", ms(pct(d, 50)));
        r.putString(name + "_screen_p95", ms(pct(d, 95)));
        r.putInt(name + "_screen_n", n);
        r.putString("refresh_ms", ms(period[0]));
    }
    private void render(int[] status, long[] took, int i) throws Exception {
        Bitmap frame = (Bitmap)field(view, "frame");
        int dpi = (Integer)field(view, "dpi");
        long t0 = System.nanoTime();
        status[0] = GalleryActivity.nativeRender(frame, dpi, false);
        took[i] = System.nanoTime() - t0;
        if (status[0] == 1) view.invalidate();
    }
    private void wall(Bundle r, String name, long[] took) {
        r.putString(name + "_ui_p50", ms(pct(took, 50)));
        r.putString(name + "_ui_p95", ms(pct(took, 95)));
    }
    // one call per vsync, n of them (the frames the host would have)
    private interface Step { void at(int i) throws Exception; }
    private void perVsync(int n, Step step) throws Exception {
        java.util.concurrent.CountDownLatch done = new java.util.concurrent.CountDownLatch(1);
        Throwable[] failed = new Throwable[1];
        runOnMainSync(() -> {
            android.view.Choreographer c = android.view.Choreographer.getInstance();
            c.postFrameCallback(new android.view.Choreographer.FrameCallback() {
                int i;
                @Override public void doFrame(long time) {
                    try { step.at(i); } catch (Throwable e) { failed[0] = e; done.countDown(); return; }
                    if (++i < n) c.postFrameCallback(this); else done.countDown();
                }
            });
        });
        done.await();
        if (failed[0] != null) throw new AssertionError(failed[0]);
    }
    private void jank(Bundle r, String name, UiAutomation ui, long since) throws Exception {
        long[] period = new long[1];
        long[][] rows = latency(ui, period);
        java.util.ArrayList<Long> at = new java.util.ArrayList<>();
        for (long[] row : rows) if (row[1] >= since) at.add(row[1]);
        java.util.Collections.sort(at);
        long[] gaps = new long[Math.max(0, at.size() - 1)];
        int late = 0;
        for (int i = 0; i + 1 < at.size(); i++) {
            gaps[i] = at.get(i + 1) - at.get(i);
            if (gaps[i] > period[0] * 3 / 2) late++;
        }
        r.putInt(name + "_shown", at.size());
        r.putInt(name + "_late", late);
        r.putString(name + "_gap_p50", ms(pct(gaps, 50)));
        r.putString(name + "_gap_p95", ms(pct(gaps, 95)));
    }
    private static long[] selfStat() throws Exception {
        String[] f = new String(java.nio.file.Files.readAllBytes(java.nio.file.Paths.get("/proc/self/stat")), StandardCharsets.UTF_8).split(" ");
        return new long[] { Long.parseLong(f[13]), Long.parseLong(f[14]) };
    }
    // voluntary + involuntary context switches: the whole process, the UI thread
    private static long[] switches() throws Exception {
        long all = 0, main = 0;
        java.io.File[] tasks = new java.io.File("/proc/self/task").listFiles();
        String pid = new java.io.File("/proc/self").getCanonicalFile().getName();
        for (java.io.File t : tasks) {
            long n = 0;
            try {
                for (String line : new String(java.nio.file.Files.readAllBytes(new java.io.File(t, "status").toPath()), StandardCharsets.UTF_8).split("\n"))
                    if (line.contains("ctxt_switches")) n += Long.parseLong(line.substring(line.indexOf(':') + 1).trim());
            } catch (java.io.IOException e) { continue; }
            all += n;
            if (t.getName().equals(pid)) main = n;
        }
        return new long[] { all, main, tasks.length };
    }
    private void bench(Bundle r, UiAutomation ui, Bundle args) throws Exception {
        boolean gpu = "1".equals(args.getString("gpu", "1"));
        boolean images = "images".equals(args.getString("page", ""));
        if (gpu && !until(30000, () -> { HashMap<String, String> s = status(); return num(s, "on") == 1 || num(s, "never") == 1; })) throw new AssertionError("no verdict");
        if (gpu && num(status(), "never") == 1) { r.putInt("skip", 1); return; }
        SystemClock.sleep(1500);
        int w = ((Bitmap)field(view, "frame")).getWidth(), h = ((Bitmap)field(view, "frame")).getHeight();
        r.putString("frame", w + "x" + h);
        if (!images) {
            // ---- idle: nothing asked of it for 10 s
            HashMap<String, String> s0 = status();
            long[] c0 = selfStat(), x0 = switches();
            SystemClock.sleep(10000);
            long[] c1 = selfStat(), x1 = switches();
            HashMap<String, String> s1 = status();
            r.putLong("idle_cpu_ms", (c1[0] + c1[1] - c0[0] - c0[1]) * 10);
            r.putLong("idle_switches", x1[0] - x0[0]);
            r.putLong("idle_switches_ui", x1[1] - x0[1]);
            r.putLong("idle_frames", num(s1, "frames") + num(s1, "cpu_frames") - num(s0, "frames") - num(s0, "cpu_frames"));
            r.putLong("threads", x1[2]);
            // ---- memory
            android.os.Debug.MemoryInfo mem = new android.os.Debug.MemoryInfo();
            android.os.Debug.getMemoryInfo(mem);
            r.putString("pss_kb", mem.getMemoryStat("summary.total-pss"));
            r.putString("native_kb", mem.getMemoryStat("summary.native-heap"));
            r.putString("graphics_kb", mem.getMemoryStat("summary.graphics"));
            r.putLong("reserved_kb", num(s1, "reserved") / 1024);
            // ---- a full frame: what the UI thread pays, what reaches the screen
            long[] full = new long[30], fullAt = new long[30];
            int[] st = new int[1];
            for (int i = 0; i < full.length; i++) {
                final int k = i;
                runOnMainSync(() -> {
                    try {
                        GalleryActivity.nativeGpuTest(3);
                        fullAt[k] = System.nanoTime();
                        render(st, full, k);
                    } catch (Exception e) { throw new AssertionError(e); }
                });
                SystemClock.sleep(60);
            }
            wall(r, "full", full);
            toScreen(r, "full", fullAt, ui);
            // ---- a small change: the check box, press and release (as a mouse: no touch injected)
            android.graphics.Rect box = new android.graphics.Rect();
            node(ui, "Option aktivieren").getBoundsInScreen(box);
            int[] at = new int[2];
            runOnMainSync(() -> view.getLocationOnScreen(at));
            float sc = w / (float)view.getWidth();
            int bx = Math.round((box.left + 40 - at[0]) * sc), by = Math.round((box.centerY() - at[1]) * sc);
            runOnMainSync(() -> GalleryActivity.nativeEvent(1, 0, 0, bx, by, 0));
            long smallPresent0 = num(status(), "present_us");
            long[] small = new long[40], smallAt = new long[40];
            for (int i = 0; i < small.length; i++) {
                final int k = i;
                runOnMainSync(() -> GalleryActivity.nativeEvent(2, 1, 0, bx, by, 0));
                SystemClock.sleep(30);
                runOnMainSync(() -> {
                    try {
                        smallAt[k] = System.nanoTime();
                        GalleryActivity.nativeEvent(3, 1, 0, bx, by, 0);
                        render(st, small, k);
                    } catch (Exception e) { throw new AssertionError(e); }
                });
                SystemClock.sleep(120);
            }
            r.putLong("small_present_us", (num(status(), "present_us") - smallPresent0) / small.length);
            wall(r, "small", small);
            toScreen(r, "small", smallAt, ui);
            // ---- a finger's scroll: as a MotionEvent would bring it, one move per vsync
            float density = view.getResources().getDisplayMetrics().density;
            int fx = w / 8, fy0 = h - (int)(40 * density * sc), step = Math.max(1, h / 160);
            long[] took = new long[120];
            long t0 = System.nanoTime();
            runOnMainSync(() -> GalleryActivity.nativeFinger(0, 0, fx, fy0, SystemClock.uptimeMillis(), true));
            SystemClock.sleep(40);
            perVsync(took.length, i -> {
                GalleryActivity.nativeFinger(1, 0, fx, fy0 - (i + 1) * step, SystemClock.uptimeMillis(), true);
                render(st, took, i);
            });
            long t1 = System.nanoTime();
            runOnMainSync(() -> GalleryActivity.nativeFinger(3, 0, 0, 0, SystemClock.uptimeMillis(), true));
            wall(r, "scroll", took);
            r.putString("scroll_ms_per_frame", ms((t1 - t0) / took.length));
            jank(r, "scroll", ui, t0);
        } else {
            // ---- a canvas: a left drag over the image view, one move per vsync
            int[] st = new int[1];
            float density = view.getResources().getDisplayMetrics().density;
            float sc = w / (float)view.getWidth();
            int x = w * 3 / 4, y = (int)(240 * density * sc);
            long[] took = new long[120];
            long t0 = System.nanoTime();
            runOnMainSync(() -> { GalleryActivity.nativeEvent(1, 0, 0, x, y, 0); GalleryActivity.nativeEvent(2, 1, 0, x, y, 0); });
            SystemClock.sleep(40);
            perVsync(took.length, i -> {
                GalleryActivity.nativeEvent(1, 0, 0, x - (i + 1) * 2, y - (i + 1), 0);
                render(st, took, i);
            });
            long t1 = System.nanoTime();
            runOnMainSync(() -> GalleryActivity.nativeEvent(3, 1, 0, x - 240, y - 120, 0));
            wall(r, "pan", took);
            r.putString("pan_ms_per_frame", ms((t1 - t0) / took.length));
            jank(r, "pan", ui, t0);
            HashMap<String, String> s = status();
            r.putString("canvas_frames", s.get("canvas_frames"));
            r.putString("canvas_tiles", s.get("canvas_tiles"));
            r.putString("reserved_after_pan_kb", String.valueOf(num(s, "reserved") / 1024));
        }
    }

    // ---- the phases without the output
    private void cpuPath(Bundle result, boolean surfaceExpected) throws Exception {
        if (surfaceExpected) check(until(30000, () -> num(status(), "never") == 1), "no loader: the CPU path for good: " + status());
        else check(num(status(), "wanted") == 0, "jfgui_GPU=0: not wanted");
        check(!flag(GalleryActivity.class, "fullResolution"), "the resolution the Bitmap path has");
        Bitmap frame = (Bitmap)field(view, "frame");
        float density = view.getResources().getDisplayMetrics().density;
        int w = view.getWidth(), h = view.getHeight();
        float scale = Math.min(Math.min(1f, 2f / density), 2048f / Math.max(w, h));
        check(until(4000, () -> {
            try { Bitmap f = (Bitmap)field(view, "frame"); return f.getWidth() == Math.round(w * scale); } catch (Exception e) { return false; }
        }), "the frame is capped: " + frame.getWidth() + "x" + frame.getHeight() + " of " + w + "x" + h);
        check(field(activity, "gpuView") == null || ((View)field(activity, "gpuView")).getVisibility() == View.GONE, "no SurfaceView on the screen");
        // the app works: a change reaches the Bitmap
        int[] a = bitmapNow();
        boolean moved = false;
        for (int i = 0; i < 60 && !moved; i++) {
            final int k = i;
            int fw = ((Bitmap)field(view, "frame")).getWidth();
            runOnMainSync(() -> GalleryActivity.nativeEvent(1, 0, 0, 30 + (k * 29) % Math.max(1, fw - 60), 100 + k * 9, 0));
            frames();
            SystemClock.sleep(30);
            int[] b = bitmapNow();
            moved = a.length == b.length && !java.util.Arrays.equals(a, b);
        }
        check(moved, "a change reaches the Bitmap");
        stream(result, status());
    }

    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            UiAutomation ui = getUiAutomation();
            Intent intent;
            switch (phase) {
                case "path": intent = start("jfgui_GPU", "1", "jfgui_GPU_VERIFY", "1", "jfgui_GPU_RETRY_MS", "200", "jfgui_GPU_TRACE", "1"); break;
                case "images": intent = start("jfgui_GPU", "1", "jfgui_GPU_VERIFY", "1", "jfgui_PAGE", "images", "jfgui_GPU_TRACE", "1"); break;
                case "nogpu": intent = start("jfgui_GPU", "1", "jfgui_GPU_DLL", "/data/local/tmp/no-such-vulkan.so"); break;
                case "off": intent = start("jfgui_GPU", "0"); break;
                case "bench": intent = start("jfgui_GPU", args.getString("gpu", "1"), "jfgui_GPU_RES", args.getString("res", "full"), "jfgui_PAGE", args.getString("page", ""),
                                             "jfgui_TEST_LANDSCAPE", args.getString("landscape", "0")); break;
                default: throw new AssertionError("no phase " + phase);
            }
            activity = startActivitySync(intent);
            waitForIdleSync();
            view = (View)field(activity, "gallery");
            switch (phase) {
                case "path": path(result, ui); break;
                case "images": images(result); break;
                case "nogpu": cpuPath(result, true); break;
                case "bench": bench(result, ui, args); break;
                default: cpuPath(result, false); break;
            }
            result.putInt("checks", checks);
            result.putString("stream", "PASS: " + phase + ": " + checks + " checks.\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putInt("checks", checks);
            try { result.putString("status", String.valueOf(status())); } catch (Throwable ignored) {}
            // what the screen showed, for whoever looks (the app's own folder: run-as cat files/gpu-fail.png)
            try (java.io.FileOutputStream out = new java.io.FileOutputStream(new java.io.File(getTargetContext().getFilesDir(), "gpu-fail.png"))) {
                getUiAutomation().takeScreenshot().compress(Bitmap.CompressFormat.PNG, 100, out);
            } catch (Throwable ignored) {}
            result.putString("stream", "FAIL: " + phase + " after " + checks + " checks: " + error + "\n");
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
