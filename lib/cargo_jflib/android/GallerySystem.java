/* What the system says and does for plat besides files (lib/plat/web_system.rs, web_power.rs;
 * the notifications: GalleryNotify):
 * - the user's settings as "key=value" lines through nativeSettings (system.c -> jfgui_system):
 *   night mode (Configuration.uiMode), the font scale (Configuration.fontScale), fewer animations
 *   (Settings.Global.ANIMATOR_DURATION_SCALE 0), more contrast (high-contrast text, readable
 *   where the system lets the app read it; the contrast level of API 34 at "high"), the accent
 *   (API 31's dynamic colours, system_accent1_600) -- at the start and after every change:
 *   onConfigurationChanged (the manifest's configChanges name uiMode and fontScale), observers of
 *   the two settings, the contrast listener;
 * - keeping the screen on and the CPU running (call CALL_POWER: the INHIBIT_* union):
 *   FLAG_KEEP_SCREEN_ON on the activity's window, a partial WakeLock "jfgui:<package>"
 *   (android.permission.WAKE_LOCK), both taken again by a new activity;
 * - the lifecycle: onStop / onStart (not the first) -> E_POWER 2 / 3 through nativeEvent ->
 *   UIC_SUSPEND 2 / 0 (onPause / onResume are E_FOCUS_OUT / IN already: UIC_ACTIVE);
 * - the back key (call CALL_BACK, plat::back_trap): while the app wants it, Esc instead of the end
 *   of the activity (a page of the app goes back, a popup closes).
 * All of it on the UI thread, like the library. */
package org.jflib.gallery;

import android.app.Activity;
import android.app.UiModeManager;
import android.content.ContentResolver;
import android.content.Context;
import android.content.res.Configuration;
import android.database.ContentObserver;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.PowerManager;
import android.provider.Settings;
import android.util.Log;
import android.view.WindowManager;
import java.nio.charset.StandardCharsets;

final class GallerySystem {
    // android_host.rs: the calls of web_notify.rs (GalleryNotify) and web_power.rs
    static final int CALL_FIRST = 50, CALL_LAST = 59, CALL_POWER = 52, CALL_BACK = 55, CALL_LONG_PRESS = 56, CALL_FINISH = 57;
    // plat::INHIBIT_*, plat::E_POWER, E_WAKE
    static final int INHIBIT_SUSPEND = 1, INHIBIT_IDLE = 2, E_POWER = 20, E_WAKE = 14;
    // Settings.Secure.ACCESSIBILITY_HIGH_TEXT_CONTRAST_ENABLED (hidden; readable on most systems)
    static final String HIGH_TEXT_CONTRAST = "high_text_contrast_enabled";
    private static final String TAG = "jfgui";

    static native void nativeSettings(byte[] text);

    private static Activity activity;
    private static Context app;
    private static int power;
    private static PowerManager.WakeLock wake;
    private static boolean observing, stopped, backTrapped;
    private static String sent;

    static void attach(Activity a) {
        activity = a;
        app = a.getApplicationContext();
        observe();
        settings();
        keepScreen();
        GalleryNotify.attach(a);
        GallerySensors.attach(a);
    }
    static void detach(Activity a) {
        if (activity == a) activity = null;
        GalleryNotify.detach(a);
        GallerySensors.detach(a);
    }
    /** GalleryActivity.onBackPressed -> true: the app takes it (Esc). */
    static boolean back() {
        if (!backTrapped) return false;
        GalleryActivity.nativeEvent(5, 0, 0, 0, 0, 27);
        GalleryActivity.nativeEvent(6, 0, 0, 0, 0, 27);
        GalleryActivity.frameNow();
        return true;
    }
    /** GalleryActivity.onConfigurationChanged: night mode, font scale. */
    static void configuration(Activity a) { settings(); }
    static void started(Activity a) {
        if (!stopped) return;
        stopped = false;
        GalleryActivity.nativeEvent(E_POWER, 0, 0, 0, 0, 3);
    }
    static void stopped(Activity a) {
        stopped = true;
        GalleryActivity.nativeEvent(E_POWER, 0, 0, 0, 0, 2);
    }

    // ------------------------------------------------------------ the settings

    private static void observe() {
        if (observing || app == null) return;
        observing = true;
        Handler main = new Handler(Looper.getMainLooper());
        ContentObserver changed = new ContentObserver(main) {
            @Override public void onChange(boolean self) { settings(); }
        };
        ContentResolver cr = app.getContentResolver();
        try {
            cr.registerContentObserver(Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), false, changed);
            cr.registerContentObserver(Settings.Secure.getUriFor(HIGH_TEXT_CONTRAST), false, changed);
        } catch (RuntimeException e) {
            Log.i(TAG, "no observer of the settings: " + e);
        }
        if (Build.VERSION.SDK_INT >= 34) {
            UiModeManager ui = (UiModeManager) app.getSystemService(Context.UI_MODE_SERVICE);
            if (ui != null) ui.addContrastChangeListener(app.getMainExecutor(), level -> settings());
        }
    }
    // high-contrast text (where readable) or the contrast level at "high" (API 34)
    private static boolean contrast(Context c) {
        try {
            if (Settings.Secure.getInt(c.getContentResolver(), HIGH_TEXT_CONTRAST, 0) == 1) return true;
        } catch (RuntimeException e) {
            // a system that does not let apps read it
        }
        if (Build.VERSION.SDK_INT >= 34) {
            UiModeManager ui = (UiModeManager) c.getSystemService(Context.UI_MODE_SERVICE);
            return ui != null && ui.getContrast() >= 0.75f;
        }
        return false;
    }
    static String text(Context c) {
        Configuration config = c.getResources().getConfiguration();
        int night = config.uiMode & Configuration.UI_MODE_NIGHT_MASK;
        float animations = Settings.Global.getFloat(c.getContentResolver(), Settings.Global.ANIMATOR_DURATION_SCALE, 1f);
        String accent = "-";
        if (Build.VERSION.SDK_INT >= 31) {
            try {
                accent = String.format(java.util.Locale.ROOT, "%06X",
                    c.getResources().getColor(android.R.color.system_accent1_600, null) & 0xFFFFFF);
            } catch (RuntimeException e) {
                accent = "-";
            }
        }
        return "dark=" + (night == Configuration.UI_MODE_NIGHT_YES ? "1" : night == Configuration.UI_MODE_NIGHT_NO ? "0" : "-")
            + "\ncontrast=" + (contrast(c) ? 1 : 0)
            + "\nmotion=" + (animations == 0f ? 1 : 0)
            + "\nscale=" + Math.round(config.fontScale * 100)
            + "\naccent=" + accent + "\n";
    }
    // to Rust when it changed (a change once the app runs is E_SETTINGS there)
    static void settings() {
        Context c = activity != null ? activity : app;
        if (c == null) return;
        String t = text(c);
        if (t.equals(sent)) return;
        sent = t;
        nativeSettings(t.getBytes(StandardCharsets.UTF_8));
        GalleryActivity.nativeEvent(E_WAKE, 0, 0, 0, 0, 0); // processed now (nothing before the start)
    }

    // ------------------------------------------------------------ the screen kept on

    static byte[] call(int kind, byte[] in) {
        if (kind == CALL_POWER) {
            int bits = in != null && in.length >= 4 ? (in[0] & 255) | (in[1] & 255) << 8 : 0;
            return new byte[] {(byte) (power(bits) ? 1 : 0)};
        }
        if (kind == CALL_BACK) {
            backTrapped = in != null && in.length >= 1 && in[0] == 1;
            return new byte[] {1};
        }
        if (kind == CALL_LONG_PRESS) {
            GalleryActivity.longPressed(); // a finger rested (lib/ui/touch.rs)
            return new byte[] {1};
        }
        if (kind == CALL_FINISH) { // plat app_finish: the runtime ended, a new start needs a new process
            final Activity a = activity;
            if (a != null) a.getWindow().getDecorView().post(() -> {
                a.finishAndRemoveTask();
                android.os.Process.killProcess(android.os.Process.myPid());
            });
            return new byte[] {1};
        }
        if (kind == GallerySensors.CALL_SENSORS || kind == GallerySensors.CALL_LOCATION) return GallerySensors.call(kind, in);
        return GalleryNotify.call(app, activity, kind, in);
    }
    private static boolean power(int bits) {
        try {
            if ((bits & INHIBIT_SUSPEND) != 0) {
                if (wake == null) {
                    PowerManager pm = (PowerManager) app.getSystemService(Context.POWER_SERVICE);
                    wake = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "jfgui:" + app.getPackageName());
                    wake.setReferenceCounted(false);
                }
                if (!wake.isHeld()) wake.acquire();
            } else if (wake != null && wake.isHeld()) {
                wake.release();
            }
        } catch (RuntimeException e) {
            // no WAKE_LOCK in the manifest
            Log.w(TAG, "wake lock: " + e);
            if ((bits & INHIBIT_SUSPEND) != 0) return false;
        }
        power = bits;
        keepScreen();
        return true;
    }
    private static void keepScreen() {
        Activity a = activity;
        if (a == null) return;
        if ((power & INHIBIT_IDLE) != 0) a.getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        else a.getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
    }
}
