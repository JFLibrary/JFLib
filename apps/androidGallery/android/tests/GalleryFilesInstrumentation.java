/* Files, the documents of other apps and the system clipboard on a device or emulator, run by
 * apps/androidGallery/tests/android_files.py (it puts /sdcard/Documents/jfgui-test/hello.txt in
 * place, clears the app's data before phase 1 and stops the app between the phases):
 *   adb shell am instrument -w -r -e phase 1 org.jflib.gallery/.GalleryFilesInstrumentation
 * Phase 1 starts the gallery's self-test (apps/androidGallery/files.rs: config:// data:// cache://
 * on internal storage, the clipboard) and answers the system's picker through UiAutomation like a
 * user: the folder Documents/jfgui-test (its button, then the confirmation), hello.txt to open,
 * the save button for gespeichert.txt; then it reads the image the app left on the clipboard the
 * way another app would.  Phase 2 (a new process) lets the self-test check the files and grants
 * after the restart, then pastes the system clipboard into the note with Ctrl+V and copies it back
 * with Ctrl+A, Ctrl+C. */
package org.jflib.gallery;

import android.app.Activity;
import android.app.Instrumentation;
import android.app.UiAutomation;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Rect;
import android.net.Uri;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.os.SystemClock;
import android.view.View;
import android.view.accessibility.AccessibilityNodeInfo;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.util.List;

public final class GalleryFilesInstrumentation extends Instrumentation {
    static final String DIR = "content://com.android.externalstorage.documents/document/primary%3ADocuments%2Fjfgui-test";
    private String phase;
    private int checks, clicks;
    private String lastPressed;
    private long lastAt;
    private final StringBuilder log = new StringBuilder();

    @Override public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        phase = arguments != null && arguments.getString("phase") != null ? arguments.getString("phase") : "1";
        start();
    }
    private void check(boolean condition, String what) {
        if (!condition) throw new AssertionError(what);
        checks++;
    }
    private static String read(File f) {
        if (!f.isFile()) return null;
        try (InputStream in = new FileInputStream(f)) {
            byte[] b = new byte[(int)f.length()];
            int n = 0;
            while (n < b.length) { int k = in.read(b, n, b.length - n); if (k < 0) break; n += k; }
            return new String(b, 0, n, StandardCharsets.UTF_8);
        } catch (Exception e) { return null; }
    }
    private static Object field(Object object, String name) throws Exception {
        Field f = object.getClass().getDeclaredField(name);
        f.setAccessible(true);
        return f.get(object);
    }
    private static ClipboardManager clipboard(Context c) {
        return (ClipboardManager)c.getSystemService(Context.CLIPBOARD_SERVICE);
    }

    // the system's picker in front: press what a user presses (view ids, not texts: any language)
    private void answerPicker(UiAutomation ui) {
        AccessibilityNodeInfo root = ui.getRootInActiveWindow();
        if (root == null || root.getPackageName() == null || !root.getPackageName().toString().contains("documentsui")) return;
        // "Use this folder" and its confirmation, "Save": the dialog's / bar's first button
        List<AccessibilityNodeInfo> buttons = root.findAccessibilityNodeInfosByViewId("android:id/button1");
        if (!buttons.isEmpty() && buttons.get(0).isEnabled()) {
            AccessibilityNodeInfo b = buttons.get(0);
            // the folder picker shows its button again while it closes after the confirmation
            String text = String.valueOf(b.getText());
            if (text.equals(lastPressed) && SystemClock.uptimeMillis() - lastAt < 5000) return;
            lastPressed = text;
            lastAt = SystemClock.uptimeMillis();
            log.append("picker: ").append(b.getText()).append('\n');
            b.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            clicks++;
            settle(ui, root.getWindowId(), b.getText());
            return;
        }
        // open: the document hello.txt, tapped like a finger ("input tap"; an accessibility click
        // on the item opens DocumentsUI's preview instead of picking it)
        for (AccessibilityNodeInfo n : root.findAccessibilityNodeInfosByText("hello.txt")) {
            if (!"hello.txt".contentEquals(n.getText() == null ? "" : n.getText())) continue;
            Rect r = new Rect();
            n.getBoundsInScreen(r);
            log.append("picker: hello.txt at ").append(r.centerX()).append(',').append(r.centerY()).append('\n');
            shell(ui, "input tap " + r.centerX() + " " + r.centerY());
            clicks++;
            settle(ui, root.getWindowId(), null);
            return;
        }
    }
    // after a press: until the window (or the pressed button's text) changed, at most 4 s
    private static void settle(UiAutomation ui, int window, CharSequence pressed) {
        long end = SystemClock.uptimeMillis() + 4000;
        while (SystemClock.uptimeMillis() < end) {
            SystemClock.sleep(200);
            AccessibilityNodeInfo now = ui.getRootInActiveWindow();
            if (now == null || now.getWindowId() != window) return;
            if (pressed == null) continue;
            List<AccessibilityNodeInfo> b = now.findAccessibilityNodeInfosByViewId("android:id/button1");
            if (b.isEmpty() || !String.valueOf(pressed).contentEquals(b.get(0).getText() == null ? "" : b.get(0).getText())) return;
        }
    }
    private static void shell(UiAutomation ui, String command) {
        try (ParcelFileDescriptor out = ui.executeShellCommand(command);
             InputStream in = new ParcelFileDescriptor.AutoCloseInputStream(out)) {
            byte[] buf = new byte[256];
            while (in.read(buf) > 0) { }
        } catch (Exception e) { throw new AssertionError(command + ": " + e); }
    }

    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            Context target = getTargetContext();
            File results = new File(target.getFilesDir(), "fs/data/androidGallery/selftest.txt");
            if (results.exists() && !results.delete()) throw new AssertionError("old results stay");
            UiAutomation ui = getUiAutomation();
            String clipIn = "Von Android ✓ " + phase;
            runOnMainSync(() -> clipboard(target).setPrimaryClip(ClipData.newPlainText("test", clipIn)));
            Intent intent = new Intent(target, GalleryActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                .putExtra("jfgui_SELFTEST", "phase" + phase).putExtra("jfgui_SELFTEST_CLIP", clipIn)
                .putExtra("jfgui_SELFTEST_DIR", DIR);
            Activity activity = startActivitySync(intent);
            waitForIdleSync();
            long end = SystemClock.uptimeMillis() + 120000;
            String text;
            while ((text = read(results)) == null || !text.contains("done ")) {
                if (SystemClock.uptimeMillis() > end) throw new AssertionError("self-test timed out:\n" + text + log);
                answerPicker(ui);
                SystemClock.sleep(250);
            }
            int ok = 0;
            StringBuilder fails = new StringBuilder();
            for (String line : text.split("\n")) {
                if (line.startsWith("ok ")) ok++;
                else if (line.startsWith("FAIL")) fails.append(line).append('\n');
            }
            check(fails.length() == 0, "self-test:\n" + fails);
            check(ok >= (phase.equals("1") ? 70 : 12), "self-test ran " + ok + " checks");
            result.putInt("app_checks", ok);
            if (phase.equals("1")) {
                check(clicks >= 4, "picker answered (folder, allow, open, save): " + clicks + "\n" + log);
                imageClip(target);
            } else {
                pasteAndCopy(target, activity);
            }
            result.putInt("checks", checks);
            result.putString("stream", "PASS: phase " + phase + ": " + ok + " self-test checks, " + checks + " checks\n" + log);
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", "FAIL: phase " + phase + ": " + error + "\n" + log);
            finish(Activity.RESULT_CANCELED, result);
        }
    }

    // the image the app copied, read as another app reads it (the URI of GalleryFiles)
    private void imageClip(Context target) throws Exception {
        final Object[] got = new Object[1];
        runOnMainSync(() -> got[0] = clipboard(target).getPrimaryClip());
        ClipData clip = (ClipData)got[0];
        check(clip != null && clip.getItemCount() == 1, "an image clip");
        Uri u = clip.getItemAt(0).getUri();
        check(u != null && (target.getPackageName() + ".jfgui.files").equals(u.getAuthority()), "the clip's URI is GalleryFiles': " + u);
        check(clip.getDescription().hasMimeType("image/png"), "the clip says image/png");
        try (InputStream in = target.getContentResolver().openInputStream(u)) {
            Bitmap b = BitmapFactory.decodeStream(in);
            check(b != null && b.getWidth() == 3 && b.getHeight() == 2, "the PNG decodes to 3 x 2");
            int px = b.getPixel(1, 1);
            check((px & 0xFFFFFF) == 0x2080C0, "its colour: " + Integer.toHexString(px));
        }
    }

    // the note (id 410): Ctrl+V pastes the system's text, Ctrl+A Ctrl+C copies it back
    private void pasteAndCopy(Context target, Activity activity) throws Exception {
        final Throwable[] failure = new Throwable[1];
        final String pasted = "Eingefügt aus Android ✓";
        runOnMainSync(() -> {
            try {
                clipboard(target).setPrimaryClip(ClipData.newPlainText("test", pasted));
                View view = (View)field(activity, "gallery");
                Bitmap frame = (Bitmap)field(view, "frame");
                int dpi = (Integer)field(view, "dpi");
                GalleryActivity.nativeRender(frame, dpi, false);
                for (int i = 0; i < 40 && editor() != 410; i++) key(9, 0);
                check(editor() == 410, "the note has the focus");
                key(65, 2); // Ctrl+A
                key(86, 2); // Ctrl+V
                GalleryActivity.nativeRender(frame, dpi, false);
                check(text().equals(pasted), "pasted: " + text());
                clipboard(target).setPrimaryClip(ClipData.newPlainText("test", "anders"));
                key(65, 2);
                key(67, 2); // Ctrl+C
                GalleryActivity.nativeRender(frame, dpi, false);
                ClipData c = clipboard(target).getPrimaryClip();
                String copied = c == null ? null : c.getItemAt(0).coerceToText(target).toString();
                check(pasted.equals(copied), "copied: " + copied);
            } catch (Throwable e) { failure[0] = e; }
        });
        if (failure[0] != null) throw new AssertionError(failure[0]);
        // the note is kept in config:// (internal storage) at once
        String note = read(new File(target.getFilesDir(), "fs/config/androidGallery/note.txt"));
        check(pasted.equals(note), "the note on internal storage: " + note);
    }
    private static void key(int vk, int mods) {
        GalleryActivity.nativeEvent(5, 0, mods, 0, 0, vk);
        GalleryActivity.nativeEvent(6, 0, mods, 0, 0, vk);
    }
    private static int editor() {
        byte[] s = GalleryActivity.nativeEditorState();
        return (s[0] & 255) | (s[1] & 255) << 8 | (s[2] & 255) << 16 | (s[3] & 255) << 24;
    }
    private static String text() {
        byte[] s = GalleryActivity.nativeEditorState();
        return new String(s, 16, s.length - 16, StandardCharsets.UTF_8);
    }
}
