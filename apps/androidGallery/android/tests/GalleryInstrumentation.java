/* Android runtime regressions, run explicitly with adb shell am instrument -w.
 * Exercises real BaseInputConnection, JNI, Rust edit state, filters and frame damage.
 */
package org.jflib.gallery;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Intent;
import android.graphics.Bitmap;
import android.os.Bundle;
import android.view.View;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;

public final class GalleryInstrumentation extends Instrumentation {
    private int checks;
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); start(); }
    private static Object field(Object object, String name) throws Exception {
        Field field = object.getClass().getDeclaredField(name);
        field.setAccessible(true);
        return field.get(object);
    }
    private void expectText(String expected) {
        byte[] snapshot = GalleryActivity.nativeEditorState();
        String actual = new String(snapshot, 16, snapshot.length - 16, StandardCharsets.UTF_8);
        if (!actual.equals(expected)) throw new AssertionError("Editor mismatch: " + actual + " != " + expected);
        checks++;
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            Intent intent = new Intent(getTargetContext(), GalleryActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            Activity activity = startActivitySync(intent);
            waitForIdleSync();
            final Throwable[] failure = new Throwable[1];
            runOnMainSync(() -> {
                try {
                    View view = (View)field(activity, "gallery");
                    Bitmap frame = (Bitmap)field(view, "frame");
                    int dpi = (Integer)field(view, "dpi");
                    GalleryActivity.nativeRender(frame, dpi, true);
                    // Keyboard focus uses the real library navigation order; no pixel coordinates.
                    for (int i = 0; i < 24; i++) {
                        byte[] state = GalleryActivity.nativeEditorState();
                        if ((state[0] & 255) == (300 & 255) && (state[1] & 255) == (300 >> 8)) break;
                        GalleryActivity.nativeEvent(5, 0, 0, 0, 0, 9);
                        GalleryActivity.nativeEvent(6, 0, 0, 0, 0, 9);
                    }
                    GalleryInputConnection input = new GalleryInputConnection(view, view::postInvalidateOnAnimation);
                    input.setSelection(0, input.getEditable().length());
                    input.commitText("", 1);
                    input.setComposingText("h", 1); expectText("h");
                    input.setComposingText("ha", 1); expectText("ha");
                    input.setComposingText("hallo", 1); expectText("hallo");
                    input.commitText("Hallo ", 1); expectText("Hallo ");
                    input.finishComposingText(); expectText("Hallo ");
                    input.commitText("\uD83D\uDE42", 1); expectText("Hallo \uD83D\uDE42");
                    input.deleteSurroundingTextInCodePoints(1, 0); expectText("Hallo ");
                    input.setSelection(0, 5);
                    input.commitText("Hi", 1); expectText("Hi ");
                    input.setSelection(3, 3);
                    input.setComposingText("du", 1); expectText("Hi du");
                    input.setComposingRegion(3, 5);
                    input.commitText("Du!", 1); expectText("Hi Du!");
                    input.deleteSurroundingText(1, 0); expectText("Hi Du");
                    if (!input.getTextBeforeCursor(5, 0).toString().equals("Hi Du")) throw new AssertionError("IME query stale");
                    checks++;
                    input.beginBatchEdit();
                    input.setSelection(0, 5);
                    input.setComposingText("\u00e4\u00f6\u00fc", 1);
                    input.endBatchEdit(); expectText("\u00e4\u00f6\u00fc");
                    input.finishComposingText();
                    input.setSelection(0, 3);
                    char[] oversized = new char[200]; Arrays.fill(oversized, 'x');
                    input.commitText(new String(oversized), 1);
                    expectText(new String(oversized, 0, 127));
                    if (input.getEditable().length() != 127) throw new AssertionError("Capacity not mirrored");
                    checks++;
                    input.setSelection(0, 127); input.commitText("", 1);
                    long[] times = new long[60];
                    for (int i = 0; i < times.length; i++) {
                        String text = "Schnelle Texteingabe".substring(0, 1 + i % 19);
                        long start = System.nanoTime();
                        input.setComposingText(text, 1);
                        GalleryActivity.nativeRender(frame, dpi, false);
                        times[i] = System.nanoTime() - start;
                        expectText(text);
                    }
                    input.commitText("Sofort sichtbar", 1); expectText("Sofort sichtbar");
                    input.closeConnection();
                    input.commitText("obsolete", 1); expectText("Sofort sichtbar");
                    GalleryActivity.nativeRender(frame, dpi, false);
                    int idleChanges = 0;
                    for (int i = 0; i < 20; i++) if (GalleryActivity.nativeRender(frame, dpi, false) != 0) idleChanges++;
                    if (idleChanges > 1) throw new AssertionError("Unchanged bitmap uploaded repeatedly: " + idleChanges);
                    checks++;
                    Arrays.sort(times);
                    result.putInt("checks", checks);
                    result.putDouble("edit_and_raster_p50_ms", times[30] / 1e6);
                    result.putDouble("edit_and_raster_p95_ms", times[57] / 1e6);
                    result.putDouble("edit_and_raster_max_ms", times[59] / 1e6);
                    result.putInt("idle_changed_frames", idleChanges);
                    if (times[59] > 500000000L) throw new AssertionError("Edit stalls beyond 500 ms");
                } catch (Throwable error) { failure[0] = error; }
            });
            if (failure[0] != null) throw new AssertionError(failure[0]);
            result.putString("stream", "PASS: Android composing input, Unicode, selection, capacity, stale sessions and idle damage.\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", "FAIL: " + error + "\n");
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
