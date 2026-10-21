/* Thin Android host for jfguiLib. No widget toolkit or WebView renders the gallery. */
package org.jflib.gallery;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Rect;
import android.os.Build;
import android.os.Bundle;
import android.text.InputType;
import android.view.Choreographer;
import android.view.KeyEvent;
import android.view.KeyCharacterMap;
import android.view.MotionEvent;
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
    static { System.loadLibrary("jfgui_android"); }
    static native int nativeRender(Bitmap bitmap, int dpi, boolean newSurface);
    private static native int nativeFrameDelay();
    static native byte[] nativeEditorState();
    static native void nativeEditorReplace(int id, byte[] text, int anchor, int caret);
    static native void nativeEvent(int type, int button, int mods, int x, int y, int data);
    private static native void nativeText(byte[] text);
    private static native boolean nativeIme();
    // the accessibility tree (GalleryAccessibility): mirror on / off, its updates, an action
    static native void nativeA11y(boolean on);
    static native byte[] nativeA11yTake();
    static native void nativeA11yAct(int win, int keyLo, int keyHi, int act, int a, int b, double value, byte[] text);
    private GalleryView gallery;

    private static native void nativeEnv(byte[] entry);

    private static void env(String name, String value) {
        nativeEnv((name + "=" + value).getBytes(StandardCharsets.UTF_8));
    }
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        GalleryHost.attach(this);
        // the system's languages for the app's texts and formats, before it starts
        String langs = android.os.LocaleList.getDefault().toLanguageTags();
        if (!langs.isEmpty()) env("LANGUAGE", langs);
        // config:// data:// cache:// on internal storage (lib/plat/android_files.rs)
        env("jfgui_ANDROID_FILES", getFilesDir().getAbsolutePath());
        env("jfgui_ANDROID_CACHE", getCacheDir().getAbsolutePath());
        // debug builds: jfgui_* string extras of the start intent as settings (self-tests)
        Bundle extras = getIntent() != null ? getIntent().getExtras() : null;
        if (extras != null && (getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0) {
            for (String key : extras.keySet()) {
                String value = key.startsWith("jfgui_") ? extras.getString(key) : null;
                if (value != null) env(key, value);
            }
        }
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
    @Override public void onDestroy() { GalleryHost.detach(this); super.onDestroy(); }
    // the system's picker answered (GalleryHost.request)
    @Override protected void onActivityResult(int code, int result, Intent data) {
        if (!GalleryHost.onResult(this, code, result, data)) super.onActivityResult(code, result, data);
        gallery.requestFrame();
    }

    private static final class GalleryView extends View implements Choreographer.FrameCallback {
        private Bitmap frame;
        private final Paint paint = new Paint(Paint.FILTER_BITMAP_FLAG);
        private final Rect dest = new Rect();
        private final int slop;
        private int dpi, mode, pointerId = -1;
        private float renderScale = 1, downX, downY, lastY, wheelRemainder;
        private boolean resumed, newSurface;
        private GalleryInputConnection connection;
        private String failure;
        private final GalleryAccessibility a11y;

        GalleryView(Context context) {
            super(context);
            setFocusable(true);
            setFocusableInTouchMode(true);
            setContentDescription("jfgui Pocket Gallery");
            slop = ViewConfiguration.get(context).getScaledTouchSlop();
            // library px -> view px: the bitmap is drawn scaled by 1 / renderScale
            a11y = new GalleryAccessibility(this, () -> 1.0 / renderScale, this::requestFrame);
        }
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
            Choreographer.getInstance().postFrameCallbackDelayed(this, nativeFrameDelay());
        }
        void requestFrame() {
            if (!resumed) return;
            Choreographer.getInstance().removeFrameCallback(this);
            Choreographer.getInstance().postFrameCallback(this);
        }
        @Override protected void onSizeChanged(int width, int height, int oldWidth, int oldHeight) {
            if (width <= 0 || height <= 0) return;
            float density = getResources().getDisplayMetrics().density;
            renderScale = Math.min(1f, 2f / density);
            renderScale = Math.min(renderScale, 2048f / Math.max(width, height));
            dpi = Math.max(48, Math.round(96 * density * renderScale));
            // Android reclaims the old Bitmap after any in-flight hardware Canvas releases it.
            frame = Bitmap.createBitmap(Math.max(1, Math.round(width * renderScale)),
                Math.max(1, Math.round(height * renderScale)), Bitmap.Config.ARGB_8888);
            dest.set(0, 0, width, height);
            newSurface = true;
            requestFrame();
        }
        private void render() {
            if (frame != null && failure == null) {
                int status = nativeRender(frame, dpi, newSurface);
                newSurface = false;
                if (status < 0) failure = "jfgui konnte nicht starten (" + status + ").";
                if (status != 0) invalidate();
            }
        }
        @Override protected void onDraw(Canvas canvas) {
            if (failure != null) {
                canvas.drawColor(Color.WHITE);
                paint.setColor(Color.RED); paint.setTextSize(18 * getResources().getDisplayMetrics().density);
                canvas.drawText(failure, 16, 80, paint);
            } else if (frame != null) canvas.drawBitmap(frame, null, dest, paint);
        }
        private void pointer(int type, float x, float y) {
            nativeEvent(type, 1, 0, Math.round(x * renderScale), Math.round(y * renderScale), 0);
        }
        private void cancelGesture() {
            // Release outside every widget so cancellation cannot accidentally click a button.
            if (mode == 2) pointer(3, -10000, -10000);
            mode = 0; pointerId = -1;
        }
        @Override public boolean onTouchEvent(MotionEvent event) {
            int action = event.getActionMasked();
            if (action == MotionEvent.ACTION_DOWN) {
                if (connection != null) connection.finishComposingText();
                requestFocus();
                pointerId = event.getPointerId(0);
                downX = event.getX(); downY = lastY = event.getY();
                mode = 1; wheelRemainder = 0;
                return true;
            }
            int index = event.findPointerIndex(pointerId);
            if (action == MotionEvent.ACTION_CANCEL || index < 0) { cancelGesture(); return true; }
            if (action == MotionEvent.ACTION_POINTER_UP && event.getPointerId(event.getActionIndex()) == pointerId) {
                cancelGesture(); return true;
            }
            float x = event.getX(index), y = event.getY(index);
            if (action == MotionEvent.ACTION_MOVE) {
                if (mode == 1 && Math.hypot(x - downX, y - downY) > slop) {
                    mode = Math.abs(y - downY) > Math.abs(x - downX) ? 3 : 2;
                    if (mode == 2) pointer(2, downX, downY);
                }
                if (mode == 2) pointer(1, x, y);
                if (mode == 3) {
                    // Wheel routing in Ui uses the last pointer position, even before the first tap.
                    pointer(1, x, y);
                    // The library uses 120 wheel units for 48 DIP of scroll travel.
                    wheelRemainder += (y - lastY) / getResources().getDisplayMetrics().density * 2.5f;
                    int delta = (int)wheelRemainder;
                    wheelRemainder -= delta;
                    if (delta != 0) nativeEvent(4, 0, 0, Math.round(x * renderScale),
                        Math.round(y * renderScale), delta & 0xffff);
                }
                lastY = y;
            } else if (action == MotionEvent.ACTION_UP) {
                if (mode == 1) pointer(2, downX, downY);
                if (mode == 1 || mode == 2) {
                    pointer(3, x, y);
                    performClick();
                    if (connection != null) connection.syncFromNative();
                    InputMethodManager ime = (InputMethodManager)getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
                    if (nativeIme()) ime.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT);
                    else ime.hideSoftInputFromWindow(getWindowToken(), 0);
                }
                mode = 0; pointerId = -1;
            }
            requestFrame();
            return true;
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
