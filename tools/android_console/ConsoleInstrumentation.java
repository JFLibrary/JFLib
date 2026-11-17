/* A program without a UI in an app's process (docs/ANDROID.md, AN0; tools/android.py build_console_apk):
 * the test APK of coretest, plattest, asynctest ... has no activity, only this instrumentation, which
 * runs the program (android/exe_main.c built with -DJFGUI_CONSOLE_APK) on a thread of its own and ends.
 * So the program runs in the SELinux domain untrusted_app and under the seccomp filter Zygote gives
 * every app, which `adb shell` has neither of.  tools/android_console.py starts it:
 *
 *   adb shell am instrument -w -e args64 B64 [-e env64 B64] org.jflib.console.NAME/org.jflib.console.ConsoleInstrumentation
 *
 * args64: the arguments, one per line, Base64; env64: NAME=VALUE lines (only jfgui_* are taken), after
 * jfgui_ANDROID_FILES / _CACHE (the app's folders, as GalleryActivity passes them).  The program runs
 * in the app's files folder, its output goes to files/console.txt (then to logcat, tag jfgui-console),
 * the exit code is its last line (exit=N) and the result's "exit". */
package org.jflib.console;

import android.app.Activity;
import android.app.Instrumentation;
import android.os.Bundle;
import android.util.Base64;
import java.io.File;
import java.nio.charset.StandardCharsets;

public final class ConsoleInstrumentation extends Instrumentation {
    /** the main thread of `adb shell`'s programs has 8 MiB (its stack limit); the program's here */
    private static final long STACK = 8L << 20;
    private Bundle arguments;

    private static native int run(String args, String env, String dir, String out);

    private static String text(Bundle arguments, String key) {
        String value = arguments == null ? null : arguments.getString(key);
        return value == null ? "" : new String(Base64.decode(value, Base64.DEFAULT), StandardCharsets.UTF_8);
    }

    @Override public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        this.arguments = arguments;
        start();
    }

    @Override public void onStart() {
        Bundle result = new Bundle();
        int code = 2;
        try {
            System.loadLibrary("jfgui_console");
            File dir = getTargetContext().getFilesDir();
            File out = new File(dir, "console.txt");
            String args = text(arguments, "args64");
            // the app's folders, as GalleryActivity passes them (lib/plat/android_files.rs); env64 after them
            String env = "jfgui_ANDROID_FILES=" + dir.getPath() + "\njfgui_ANDROID_CACHE=" + getTargetContext().getCacheDir().getPath()
                + "\n" + text(arguments, "env64");
            final int[] exit = {2};
            Thread thread = new Thread(null, () -> exit[0] = run(args, env, dir.getPath(), out.getPath()), "jfgui-console", STACK);
            thread.start();
            thread.join();
            code = exit[0];
            result.putString(REPORT_KEY_STREAMRESULT, "exit=" + code + "\n");
        } catch (Throwable t) {
            result.putString(REPORT_KEY_STREAMRESULT, "jfgui-console: " + t + "\n");
        }
        result.putInt("exit", code);
        finish(code == 0 ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
