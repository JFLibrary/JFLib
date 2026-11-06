/* Thin Android host for jfguiLib. No widget toolkit or WebView renders the gallery.
 * How a frame reaches the screen (docs/GPU.md "Ausgabe unter Android"): GalleryView's Bitmap (the CPU
 * path; the first frames of every run), or a Vulkan swapchain on the SurfaceView above it (gpuView:
 * lib/ui/gpu_out.rs, lib/plat/android_gpu.rs, bridge.c). */
package org.jflib.gallery;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.PixelFormat;
import android.graphics.Rect;
import android.os.Build;
import android.os.Bundle;
import android.text.InputType;
import android.view.Choreographer;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.KeyCharacterMap;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.view.accessibility.AccessibilityNodeProvider;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.widget.FrameLayout;
import java.nio.charset.StandardCharsets;

public final class GalleryActivity extends Activity {
    // The UI through the GPU where the device has Vulkan 1.1 (off: the Bitmap alone, no driver is
    // loaded); these two lines decide, docs/GPU.md "Ausgabe unter Android" has the measurements
    private static final boolean GPU_DEFAULT = true;
    // With it, the view's whole resolution; off: the resolution the Bitmap path has (see scale())
    private static final boolean GPU_FULL_RESOLUTION = true;
    // What is largest on the way to the screen: a Bitmap's limit in bridge.c, a swapchain's in old systems
    private static final int MAX_SIDE = 4096;

    static { System.loadLibrary("jfgui_android"); }
    static native int nativeRender(Bitmap bitmap, int dpi, boolean newSurface);
    // the SurfaceView's surface, null when it went -> a frame waits to go to the Bitmap
    private static native boolean nativeGpuSurface(android.view.Surface surface);
    // (tests) the GPU output's counters; op 1 loses the device, 2 settles, 3 asks for a full frame
    static native byte[] nativeGpuTest(int op);
    private static native int nativeFrameDelay();
    private static native long nativePresentUs();
    static native byte[] nativeEditorState();
    static native void nativeEditorReplace(int id, byte[] text, int anchor, int caret);
    static native void nativeEvent(int type, int button, int mods, int x, int y, int data);
    private static native void nativeText(byte[] text);
    private static native boolean nativeIme();
    // a raw finger, phase 0 down, 1 move, 2 up, 3 cancel: pan, fling, tap and long press are decided
    // in Rust (lib/ui/touch.rs)
    static native void nativeFinger(int phase, int id, int x, int y, long time, boolean last);
    // the gesture that just ended reached a widget (a tap, a drag), it did not only scroll
    private static native boolean nativeFingerReached();
    // the accessibility tree (GalleryAccessibility): mirror on / off, its updates, an action
    static native void nativeA11y(boolean on);
    static native byte[] nativeA11yTake();
    static native void nativeA11yAct(int win, int keyLo, int keyHi, int act, int a, int b, double value, byte[] text);
    private GalleryView gallery;
    private static GalleryActivity current;
    private SurfaceView gpuView;
    // the frames fill the view's pixels (else scale()'s limit): the device is expected to show them
    // through the GPU, see gpuPlan
    private static boolean fullResolution;
    // what the GPU did is kept across runs (not for runs a test forced)
    private static boolean gpuRemember;
    private static int gpuTries;
    /** A frame now: what a sensor or a fix changed (GallerySensors, GallerySystem.back). */
    static void frameNow() {
        if (current != null && current.gallery != null) current.gallery.requestFrame();
    }
    /** A finger rested long enough: the system's feedback (lib/ui/touch.rs asks for it). */
    static void longPressed() {
        if (current == null || current.gallery == null) return;
        current.gallery.performHapticFeedback(android.view.HapticFeedbackConstants.LONG_PRESS);
        current.gallery.performLongClick();
    }

    // ---- the UI through the GPU

    static final int CALL_GPU = 60; // lib/plat/android_gpu.rs
    private static final int GPU_HIDE = 0, GPU_SHOW = 1, GPU_UNKNOWN = 2, GPU_TRYING = 3, GPU_OK = 4, GPU_NEVER = 5;
    private static final String GPU_FILE = "gpu-state";

    /* Rust's question on the UI thread: the SurfaceView (the system may call back at once from
     * setVisibility while Rust holds the app: hence post), how the output came up. */
    static byte[] gpuCall(byte[] in) {
        GalleryActivity a = current;
        if (a == null || in == null || in.length != 1) return null;
        switch (in[0]) {
            case GPU_HIDE:
            case GPU_SHOW:
                if (a.gpuView != null) {
                    int visibility = in[0] == GPU_SHOW ? View.VISIBLE : View.INVISIBLE;
                    a.gpuView.post(() -> { if (a.gpuView.getVisibility() != visibility) a.gpuView.setVisibility(visibility); });
                }
                break;
            case GPU_NEVER:
                a.gpuRemember(GPU_NEVER, 0);
                if (fullResolution) { // a frame of the size the Bitmap path has
                    fullResolution = false;
                    if (a.gallery != null) a.gallery.post(a.gallery::rescale);
                }
                if (a.gpuView != null) a.gpuView.post(() -> a.gpuView.setVisibility(View.GONE));
                break;
            case GPU_TRYING: a.gpuRemember(GPU_TRYING, gpuTries + 1); break;
            case GPU_OK: a.gpuRemember(GPU_OK, 0); break;
            default: a.gpuRemember(GPU_UNKNOWN, 0); break;
        }
        return new byte[0];
    }
    // "the system's build\nok|never|trying N": a new system gets a new chance
    private void gpuRemember(int state, int tries) {
        if (!gpuRemember) return;
        java.io.File f = new java.io.File(getFilesDir(), GPU_FILE);
        if (state == GPU_UNKNOWN) { f.delete(); return; }
        String word = state == GPU_OK ? "ok" : state == GPU_NEVER ? "never" : "trying " + tries;
        GalleryHost.write(f, (Build.FINGERPRINT + "\n" + word).getBytes(StandardCharsets.UTF_8));
    }
    private String gpuRemembered() {
        try {
            byte[] all = java.nio.file.Files.readAllBytes(new java.io.File(getFilesDir(), GPU_FILE).toPath());
            String[] lines = new String(all, StandardCharsets.UTF_8).split("\n");
            return lines.length == 2 && lines[0].equals(Build.FINGERPRINT) ? lines[1] : "";
        } catch (java.io.IOException | RuntimeException e) {
            return "";
        }
    }
    /* Before the first frame: is the output wanted, and at which resolution do the frames start.
     * Off without Vulkan 1.1 in the system's features (no driver is touched), when the last run found
     * it unusable, or when two runs in a row died while it came up (a broken driver).  debug builds:
     * the extra jfgui_GPU=0|1 forces it, jfgui_GPU_RES=cap|full the resolution. */
    private void gpuPlan(Bundle extras) {
        String forced = extras != null ? extras.getString("jfgui_GPU") : null;
        String res = extras != null ? extras.getString("jfgui_GPU_RES") : null;
        boolean on;
        if (forced != null) {
            on = !forced.equals("0") && !forced.equalsIgnoreCase("off");
        } else {
            on = GPU_DEFAULT && getPackageManager().hasSystemFeature(
                android.content.pm.PackageManager.FEATURE_VULKAN_HARDWARE_VERSION, (1 << 22) | (1 << 12));
            String known = on ? gpuRemembered() : "";
            if (known.equals("never")) on = false;
            else if (known.startsWith("trying ")) {
                try { gpuTries = Integer.parseInt(known.substring(7)); } catch (NumberFormatException e) { gpuTries = 2; }
                if (gpuTries >= 2) on = false;
            }
        }
        gpuRemember = forced == null && (extras == null || extras.getString("jfgui_GPU_DLL") == null);
        if (gpuRemember && gpuTries >= 2) gpuRemember(GPU_NEVER, 0);
        env("jfgui_GPU", on ? "1" : "0");
        fullResolution = "full".equals(res) || (!"cap".equals(res) && on && GPU_FULL_RESOLUTION);
        if (!on) return;
        // above the window's contents, nothing to see until its first present; the View below takes the input
        gpuView = new SurfaceView(this);
        gpuView.setZOrderOnTop(true);
        gpuView.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        gpuView.getHolder().setFormat(PixelFormat.RGBX_8888);
        gpuView.getHolder().addCallback(new SurfaceHolder.Callback() {
            @Override public void surfaceCreated(SurfaceHolder holder) {}
            @Override public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
                nativeGpuSurface(holder.getSurface());
                if (gallery != null) gallery.requestFrame();
            }
            @Override public void surfaceDestroyed(SurfaceHolder holder) {
                if (nativeGpuSurface(null) && gallery != null) gallery.renderNow();
            }
        });
    }
    // the surface's buffers: the frame's size (the compositor scales them to the view)
    private void gpuSize(int frameWidth, int frameHeight, int width, int height) {
        if (gpuView == null) return;
        if (frameWidth == width && frameHeight == height) gpuView.getHolder().setSizeFromLayout();
        else gpuView.getHolder().setFixedSize(frameWidth, frameHeight);
    }

    private static native void nativeEnv(byte[] entry);
    // the displays (lib/plat/web_screen.rs jfgui_screens): one line each
    private static native void nativeScreens(byte[] lines);
    private android.hardware.display.DisplayManager.DisplayListener displayListener;

    private static void env(String name, String value) {
        nativeEnv((name + "=" + value).getBytes(StandardCharsets.UTF_8));
    }
    // every display: "0 0 w h 0 0 w h dpi refresh_mHz primary name" (real pixels; 96 dpi = 100 %)
    private void screens() {
        android.hardware.display.DisplayManager dm =
            (android.hardware.display.DisplayManager) getSystemService(Context.DISPLAY_SERVICE);
        if (dm == null) return;
        StringBuilder b = new StringBuilder();
        for (android.view.Display d : dm.getDisplays()) {
            android.util.DisplayMetrics m = new android.util.DisplayMetrics();
            d.getRealMetrics(m);
            int w = m.widthPixels, h = m.heightPixels;
            b.append("0 0 ").append(w).append(' ').append(h).append(" 0 0 ").append(w).append(' ').append(h)
                .append(' ').append(Math.max(48, Math.round(96 * m.density)))
                .append(' ').append(Math.round(d.getRefreshRate() * 1000))
                .append(' ').append(d.getDisplayId() == android.view.Display.DEFAULT_DISPLAY ? 1 : 0)
                .append(' ').append(d.getName().replace('\n', ' ').replace('\r', ' ')).append('\n');
        }
        nativeScreens(b.toString().getBytes(StandardCharsets.UTF_8));
    }
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        current = this;
        GalleryHost.attach(this);
        GallerySystem.attach(this); // the system's settings, the screen kept on, notifications, sensors
        // the displays now and after every change (a projection, a folding phone)
        screens();
        android.hardware.display.DisplayManager dm =
            (android.hardware.display.DisplayManager) getSystemService(Context.DISPLAY_SERVICE);
        if (dm != null) {
            displayListener = new android.hardware.display.DisplayManager.DisplayListener() {
                @Override public void onDisplayAdded(int id) { screens(); }
                @Override public void onDisplayRemoved(int id) { screens(); }
                @Override public void onDisplayChanged(int id) { screens(); }
            };
            dm.registerDisplayListener(displayListener, null);
        }
        // the system's languages for the app's texts and formats, before it starts
        String langs = android.os.LocaleList.getDefault().toLanguageTags();
        if (!langs.isEmpty()) env("LANGUAGE", langs);
        // the device's numbers for the finger (lib/ui/touch.rs): slop, fling velocities (DIP), long press (ms)
        ViewConfiguration vc = ViewConfiguration.get(this);
        float dip = getResources().getDisplayMetrics().density;
        env("jfgui_TOUCH", Math.round(vc.getScaledTouchSlop() / dip) + " " + Math.round(vc.getScaledMinimumFlingVelocity() / dip)
            + " " + Math.round(vc.getScaledMaximumFlingVelocity() / dip) + " " + ViewConfiguration.getLongPressTimeout());
        // config:// data:// cache:// on internal storage (lib/plat/android_files.rs)
        env("jfgui_ANDROID_FILES", getFilesDir().getAbsolutePath());
        env("jfgui_ANDROID_CACHE", getCacheDir().getAbsolutePath());
        // debug builds: jfgui_* string extras of the start intent as settings (self-tests)
        Bundle extras = getIntent() != null ? getIntent().getExtras() : null;
        boolean debuggable = (getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0;
        if (extras != null && debuggable) {
            for (String key : extras.keySet()) {
                String value = key.startsWith("jfgui_") ? extras.getString(key) : null;
                if (value != null) env(key, value);
            }
        }
        gpuPlan(debuggable ? extras : null);
        // (android_gpu.py: both paths on a turned display, compared from outside; the activity's own wish)
        if (debuggable && extras != null && "1".equals(extras.getString("jfgui_TEST_LANDSCAPE")))
            setRequestedOrientation(android.content.pm.ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE);
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE
            | WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN);
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.rgb(243, 243, 243));
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
            root.setOnApplyWindowInsetsListener((view, insets) -> {
                android.graphics.Insets bars = insets.getInsets(WindowInsets.Type.systemBars()
                    | WindowInsets.Type.displayCutout() | WindowInsets.Type.ime());
                view.setPadding(bars.left, bars.top, bars.right, bars.bottom);
                return insets;
            });
        } else {
            root.setFitsSystemWindows(true);
        }
        gallery = new GalleryView(this);
        if (gpuView != null) root.addView(gpuView, new FrameLayout.LayoutParams(-1, -1));
        root.addView(gallery, new FrameLayout.LayoutParams(-1, -1));
        setContentView(root);
        if (Build.VERSION.SDK_INT >= 30) {
            WindowInsetsController controller = getWindow().getInsetsController();
            if (controller != null) {
                int appearance = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                    | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                controller.setSystemBarsAppearance(appearance, appearance);
            }
        } else {
            getWindow().getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR);
        }
        root.requestApplyInsets();
    }
    @Override public void onResume() { super.onResume(); gallery.resume(); }
    @Override public void onPause() { gallery.pause(); super.onPause(); }
    // the lifecycle, the configuration, a tapped notification, its permission (GallerySystem, GalleryNotify)
    @Override public void onStart() { super.onStart(); GallerySystem.started(this); }
    @Override public void onStop() { GallerySystem.stopped(this); super.onStop(); }
    @Override public void onConfigurationChanged(android.content.res.Configuration c) {
        super.onConfigurationChanged(c);
        GallerySystem.configuration(this);
    }
    @Override protected void onNewIntent(Intent intent) { super.onNewIntent(intent); setIntent(intent); GalleryNotify.intent(intent); }
    @Override public void onRequestPermissionsResult(int code, String[] permissions, int[] results) {
        if (!GalleryNotify.permissions(code, results) && !GallerySensors.permissions(code, results))
            super.onRequestPermissionsResult(code, permissions, results);
    }
    // a page of the app goes back first (plat::back_trap); targetSdk 35: no OnBackInvokedCallback needed
    @Override public void onBackPressed() {
        if (!GallerySystem.back()) super.onBackPressed();
    }
    @Override public void onDestroy() {
        android.hardware.display.DisplayManager dm =
            (android.hardware.display.DisplayManager) getSystemService(Context.DISPLAY_SERVICE);
        if (dm != null && displayListener != null) dm.unregisterDisplayListener(displayListener);
        if (current == this) current = null;
        GalleryHost.detach(this);
        GallerySystem.detach(this);
        super.onDestroy();
    }
    // the system's picker answered (GalleryHost.request)
    @Override protected void onActivityResult(int code, int result, Intent data) {
        if (!GalleryHost.onResult(this, code, result, data)) super.onActivityResult(code, result, data);
        gallery.requestFrame();
    }

    private static final class GalleryView extends View implements Choreographer.FrameCallback {
        private Bitmap frame;
        private final Paint paint = new Paint(Paint.FILTER_BITMAP_FLAG);
        private final Rect dest = new Rect();
        private int dpi, fingers; // (fingers: a bit per pointer id that is down)
        private float renderScale = 1;
        private boolean resumed, newSurface, mouseDown;
        private int mouseButton = 1;
        private GalleryInputConnection connection;
        private String failure;
        private final GalleryAccessibility a11y;
        private long[] perf;
        private long perfPresent;
        private int perfN;

        GalleryView(Context context) {
            super(context);
            setFocusable(true);
            setFocusableInTouchMode(true);
            setContentDescription("jfgui Pocket Gallery");
            // library px -> view px: the bitmap is drawn scaled by 1 / renderScale
            a11y = new GalleryAccessibility(this, () -> 1.0 / renderScale, this::requestFrame);
            Intent start = ((Activity)context).getIntent();
            if ((context.getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0 && start != null
                && "1".equals(start.getStringExtra("jfgui_PERF"))) perf = new long[120];
        }
        private boolean imeWas;
        void resume() {
            if (resumed) return;
            resumed = true;
            nativeEvent(10, 0, 0, 0, 0, 0);
            a11y.resume();
            requestFrame();
        }
        @Override public AccessibilityNodeProvider getAccessibilityNodeProvider() { return a11y; }
        @Override public boolean dispatchHoverEvent(MotionEvent event) {
            return a11y.hover(event) || super.dispatchHoverEvent(event);
        }
        void pause() {
            resumed = false;
            cancelGesture();
            nativeEvent(11, 0, 0, 0, 0, 0);
            Choreographer.getInstance().removeFrameCallback(this);
        }
        @Override protected void onDetachedFromWindow() { pause(); super.onDetachedFromWindow(); }
        @Override public void doFrame(long time) {
            if (!resumed) return;
            render();
            if (connection != null) connection.syncFromNative();
            a11y.pull();
            // a text box went away (its dialog closed): the keyboard goes with it
            boolean ime = nativeIme();
            if (imeWas && !ime) ((InputMethodManager)getContext().getSystemService(Context.INPUT_METHOD_SERVICE)).hideSoftInputFromWindow(getWindowToken(), 0);
            imeWas = ime;
            Choreographer.getInstance().postFrameCallbackDelayed(this, nativeFrameDelay());
        }
        void requestFrame() {
            if (!resumed) return;
            Choreographer.getInstance().removeFrameCallback(this);
            Choreographer.getInstance().postFrameCallback(this);
        }
        // library px per view px.  The Bitmap path keeps what it could always compute: at most 2 px per
        // DIP and 2048 px on the longer side; through the GPU the view's own pixels
        private float scale(int width, int height, float density) {
            if (fullResolution) return Math.min(1f, (float)MAX_SIDE / Math.max(width, height));
            return Math.min(Math.min(1f, 2f / density), 2048f / Math.max(width, height));
        }
        void rescale() {
            if (getWidth() > 0 && getHeight() > 0) onSizeChanged(getWidth(), getHeight(), getWidth(), getHeight());
        }
        @Override protected void onSizeChanged(int width, int height, int oldWidth, int oldHeight) {
            if (width <= 0 || height <= 0) return;
            float density = getResources().getDisplayMetrics().density;
            renderScale = scale(width, height, density);
            dpi = Math.max(48, Math.round(96 * density * renderScale));
            // Android reclaims the old Bitmap after any in-flight hardware Canvas releases it.
            frame = Bitmap.createBitmap(Math.max(1, Math.round(width * renderScale)),
                Math.max(1, Math.round(height * renderScale)), Bitmap.Config.ARGB_8888);
            dest.set(0, 0, width, height);
            newSurface = true;
            GalleryActivity a = (GalleryActivity)getContext();
            a.gpuSize(frame.getWidth(), frame.getHeight(), width, height);
            requestFrame();
        }
        // the system took the surface: the frame in the Bitmap, the window may be seen again before the next
        void renderNow() { render(); }
        private void render() {
            if (frame != null && failure == null) {
                long start = perf != null ? System.nanoTime() : 0;
                int status = nativeRender(frame, dpi, newSurface);
                if (perf != null && status > 0) perfAdd(System.nanoTime() - start);
                newSurface = false;
                if (status < 0) failure = "jfgui konnte nicht starten (" + status + ").";
                if (status != 0 && status != 2) invalidate(); // (2: through the GPU, the Bitmap rests)
            }
        }
        // debug builds with the extra jfgui_PERF=1: how long nativeRender takes on frames that
        // changed something, in logcat (tag jfgui-perf) every 120 of them
        private void perfAdd(long nanos) {
            perf[perfN++] = nanos;
            if (perfN < perf.length) return;
            long[] v = perf.clone();
            java.util.Arrays.sort(v);
            long sum = 0;
            for (long x : v) sum += x;
            long present = nativePresentUs();
            android.util.Log.i("jfgui-perf", String.format(java.util.Locale.ROOT,
                "nativeRender over %d changed frames: mean %.2f ms, median %.2f, p95 %.2f, max %.2f (%dx%d), of the mean presenting %.2f",
                v.length, sum / 1e6 / v.length, v[v.length / 2] / 1e6, v[v.length * 95 / 100] / 1e6, v[v.length - 1] / 1e6,
                frame.getWidth(), frame.getHeight(), (present - perfPresent) / 1e3 / v.length));
            perfPresent = present;
            perfN = 0;
        }
        @Override protected void onDraw(Canvas canvas) {
            if (failure != null) {
                canvas.drawColor(Color.WHITE);
                paint.setColor(Color.RED); paint.setTextSize(18 * getResources().getDisplayMetrics().density);
                canvas.drawText(failure, 16, 80, paint);
            } else if (frame != null) canvas.drawBitmap(frame, null, dest, paint);
        }
        private void pointer(int type, float x, float y) {
            nativeEvent(type, 0, 0, Math.round(x * renderScale), Math.round(y * renderScale), 0);
        }
        private void finger(int phase, int id, float x, float y, long time, boolean last) {
            nativeFinger(phase, id, Math.round(x * renderScale), Math.round(y * renderScale), time, last);
        }
        private void cancelGesture() {
            long now = android.os.SystemClock.uptimeMillis();
            for (int id = 0; fingers >>> id != 0; id++)
                if ((fingers >>> id & 1) != 0) nativeFinger(3, id, 0, 0, now, true);
            fingers = 0;
        }
        // a real mouse (DeX, USB): an ordinary pointer, no gestures; the scroll bars come back
        private boolean mouse(MotionEvent event) {
            int action = event.getActionMasked();
            if (action == MotionEvent.ACTION_DOWN) {
                if (connection != null) connection.finishComposingText();
                requestFocus();
                int buttons = event.getButtonState();
                mouseButton = (buttons & MotionEvent.BUTTON_SECONDARY) != 0 ? 2 : (buttons & MotionEvent.BUTTON_TERTIARY) != 0 ? 3 : 1;
                pointer(1, event.getX(), event.getY());
                nativeEvent(2, mouseButton, modifiers(event.getMetaState()), Math.round(event.getX() * renderScale), Math.round(event.getY() * renderScale), 0);
                mouseDown = true;
            } else if (action == MotionEvent.ACTION_MOVE) {
                pointer(1, event.getX(), event.getY());
            } else if ((action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) && mouseDown) {
                mouseDown = false;
                nativeEvent(3, mouseButton, modifiers(event.getMetaState()), Math.round(event.getX() * renderScale), Math.round(event.getY() * renderScale), 0);
                afterGesture();
            }
            requestFrame();
            return true;
        }
        // a tap or a drag ended: the system's click, the text field's state, the keyboard follows
        private void afterGesture() {
            performClick();
            if (connection != null) connection.syncFromNative();
            InputMethodManager ime = (InputMethodManager)getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
            if (nativeIme()) ime.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT);
            else ime.hideSoftInputFromWindow(getWindowToken(), 0);
        }
        @Override public boolean onTouchEvent(MotionEvent event) {
            if (event.getToolType(0) == MotionEvent.TOOL_TYPE_MOUSE || mouseDown) return mouse(event);
            int action = event.getActionMasked(), index = event.getActionIndex();
            long time = event.getEventTime();
            switch (action) {
                case MotionEvent.ACTION_DOWN:
                    if (connection != null) connection.finishComposingText();
                    requestFocus();
                    cancelGesture(); // (a finger that never lifted)
                    // fall through
                case MotionEvent.ACTION_POINTER_DOWN: {
                    int id = event.getPointerId(index);
                    fingers |= 1 << id;
                    finger(0, id, event.getX(index), event.getY(index), time, true);
                    break;
                }
                case MotionEvent.ACTION_MOVE: {
                    // every finger, every sample of the batch with its own time: the velocity needs them
                    int pointers = event.getPointerCount();
                    for (int h = 0, n = event.getHistorySize(); h < n; h++)
                        for (int p = 0; p < pointers; p++)
                            finger(1, event.getPointerId(p), event.getHistoricalX(p, h), event.getHistoricalY(p, h), event.getHistoricalEventTime(h), false);
                    for (int p = 0; p < pointers; p++)
                        finger(1, event.getPointerId(p), event.getX(p), event.getY(p), time, p == pointers - 1);
                    break;
                }
                case MotionEvent.ACTION_POINTER_UP:
                case MotionEvent.ACTION_UP: {
                    int id = event.getPointerId(index);
                    fingers &= ~(1 << id);
                    finger(2, id, event.getX(index), event.getY(index), time, true);
                    if (action == MotionEvent.ACTION_UP && nativeFingerReached()) afterGesture();
                    break;
                }
                case MotionEvent.ACTION_CANCEL:
                    cancelGesture();
                    break;
                default:
                    break;
            }
            requestFrame();
            return true;
        }
        @Override public boolean onHoverEvent(MotionEvent event) {
            // a mouse moving without a button: hover, the scroll bars
            if (event.isFromSource(InputDevice.SOURCE_MOUSE) && event.getActionMasked() == MotionEvent.ACTION_HOVER_MOVE) {
                pointer(1, event.getX(), event.getY());
                requestFrame();
                return true;
            }
            return super.onHoverEvent(event);
        }
        @Override public boolean performClick() { super.performClick(); return true; }
        @Override public boolean onGenericMotionEvent(MotionEvent event) {
            if (event.getActionMasked() != MotionEvent.ACTION_SCROLL) return super.onGenericMotionEvent(event);
            pointer(1, event.getX(), event.getY());
            nativeEvent(4, 0, modifiers(event.getMetaState()), Math.round(event.getX() * renderScale),
                Math.round(event.getY() * renderScale), ((int)(event.getAxisValue(MotionEvent.AXIS_VSCROLL) * 120)) & 0xffff);
            requestFrame();
            return true;
        }
        private static int modifiers(int meta) {
            return ((meta & KeyEvent.META_SHIFT_ON) != 0 ? 1 : 0)
                | ((meta & KeyEvent.META_CTRL_ON) != 0 ? 2 : 0) | ((meta & KeyEvent.META_ALT_ON) != 0 ? 4 : 0);
        }
        private static int virtualKey(int code) {
            switch (code) {
                case KeyEvent.KEYCODE_DEL: return 8;
                case KeyEvent.KEYCODE_FORWARD_DEL: return 46;
                case KeyEvent.KEYCODE_ENTER: return 13;
                case KeyEvent.KEYCODE_TAB: return 9;
                case KeyEvent.KEYCODE_ESCAPE: return 27;
                case KeyEvent.KEYCODE_DPAD_LEFT: return 37;
                case KeyEvent.KEYCODE_DPAD_UP: return 38;
                case KeyEvent.KEYCODE_DPAD_RIGHT: return 39;
                case KeyEvent.KEYCODE_DPAD_DOWN: return 40;
                case KeyEvent.KEYCODE_MOVE_HOME: return 36;
                case KeyEvent.KEYCODE_MOVE_END: return 35;
                default: return code >= KeyEvent.KEYCODE_A && code <= KeyEvent.KEYCODE_Z ? 65 + code - KeyEvent.KEYCODE_A : 0;
            }
        }
        private void key(int vk, int mods) {
            nativeEvent(5, 0, mods, 0, 0, vk);
            nativeEvent(6, 0, mods, 0, 0, vk);
            requestFrame();
        }
        private void commit(CharSequence text) {
            nativeText(text.toString().getBytes(StandardCharsets.UTF_8));
            requestFrame();
        }
        @Override public boolean onKeyDown(int code, KeyEvent event) {
            if (code == KeyEvent.KEYCODE_BACK || code == KeyEvent.KEYCODE_VOLUME_UP || code == KeyEvent.KEYCODE_VOLUME_DOWN)
                return super.onKeyDown(code, event);
            int mods = modifiers(event.getMetaState());
            int vk = virtualKey(code);
            if (vk != 0 && (vk < 65 || (mods & 6) != 0)) key(vk, mods);
            else {
                int unicode = event.getUnicodeChar();
                if (unicode != 0 && (unicode & KeyCharacterMap.COMBINING_ACCENT) == 0)
                    commit(new String(Character.toChars(unicode)));
            }
            return true;
        }
        @Override public boolean onCheckIsTextEditor() { return true; }
        @Override public InputConnection onCreateInputConnection(EditorInfo info) {
            info.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
            info.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_ACTION_DONE;
            if (connection != null) connection.closeConnection();
            connection = new GalleryInputConnection(this, this::requestFrame);
            connection.fillEditorInfo(info);
            return connection;
        }
    }
}
