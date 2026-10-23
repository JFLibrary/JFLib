/* What a frame costs to reach the Bitmap (lib/plat/web_windows.rs, android/bridge.c), and that
 * partial frames leave the same pixels as a full one; run explicitly with
 * adb shell am instrument -w -r org.jflib.gallery/.GalleryPresentInstrumentation.
 * Full frames: nativeRender with a new surface (the whole canvas again, nothing re-rendered);
 * partial frames: the pointer over the widgets. The result has the times (median, ms) and a
 * hash of the Bitmap (the same in every build that shows the same pixels).
 */
package org.jflib.gallery;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Intent;
import android.graphics.Bitmap;
import android.os.Bundle;
import android.view.View;
import java.lang.reflect.Field;
import java.util.Arrays;

public final class GalleryPresentInstrumentation extends Instrumentation {
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); start(); }
    private static Object field(Object object, String name) throws Exception {
        Field field = object.getClass().getDeclaredField(name);
        field.setAccessible(true);
        return field.get(object);
    }
    private static int[] pixels(Bitmap frame) {
        int[] px = new int[frame.getWidth() * frame.getHeight()];
        frame.getPixels(px, 0, frame.getWidth(), 0, 0, frame.getWidth(), frame.getHeight());
        return px;
    }
    private static double median(long[] t) {
        long[] s = t.clone();
        Arrays.sort(s);
        return s[s.length / 2] / 1e6;
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
                    long[] full = new long[30], partial = new long[30];
                    for (int i = 0; i < full.length; i++) {
                        long start = System.nanoTime();
                        GalleryActivity.nativeRender(frame, dpi, true);
                        full[i] = System.nanoTime() - start;
                    }
                    int w = frame.getWidth(), h = frame.getHeight(), changed = 0;
                    for (int i = 0; i < partial.length; i++) {
                        GalleryActivity.nativeEvent(1, 0, 0, 40 + (i * 29) % Math.max(1, w - 80), 120 + (i * 41) % Math.max(1, h - 200), 0);
                        long start = System.nanoTime();
                        if (GalleryActivity.nativeRender(frame, dpi, false) != 0) changed++;
                        partial[i] = System.nanoTime() - start;
                    }
                    GalleryActivity.nativeEvent(1, 0, 0, w / 2, h - 20, 0);
                    GalleryActivity.nativeRender(frame, dpi, false);
                    int[] afterPartial = pixels(frame);
                    GalleryActivity.nativeRender(frame, dpi, true);
                    int[] afterFull = pixels(frame);
                    if (!Arrays.equals(afterPartial, afterFull)) {
                        int first = 0;
                        while (afterPartial[first] == afterFull[first]) first++;
                        throw new AssertionError("Partial frames differ from a full one at " + (first % w) + ", " + (first / w));
                    }
                    if (changed == 0) throw new AssertionError("No partial frame changed the Bitmap");
                    result.putString("bitmap", w + "x" + h);
                    result.putInt("bitmap_hash", Arrays.hashCode(afterFull));
                    result.putDouble("full_present_p50_ms", median(full));
                    result.putDouble("partial_frame_p50_ms", median(partial));
                    result.putInt("partial_frames_changed", changed);
                } catch (Throwable error) { failure[0] = error; }
            });
            if (failure[0] != null) throw new AssertionError(failure[0]);
            result.putString("stream", "PASS: Android presents: partial frames equal a full one.\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", "FAIL: " + error + "\n");
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
