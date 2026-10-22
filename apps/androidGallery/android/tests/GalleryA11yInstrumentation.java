/* The accessibility tree through Android's own accessibility framework, run explicitly:
 *   adb shell am instrument -w -r org.jflib.gallery/.GalleryA11yInstrumentation
 * UiAutomation is an accessibility client like TalkBack: it reads the virtual views of
 * GalleryAccessibility (classes, texts, headings, states, ranges, bounds, selections) and acts on them
 * (click, set progress, scroll, set text, focus, select, accessibility focus); events arrive
 * for text and focus; the hit test of explore by touch finds the node under a point.
 */
package org.jflib.gallery;

import android.accessibilityservice.AccessibilityServiceInfo;
import android.app.Activity;
import android.app.Instrumentation;
import android.app.UiAutomation;
import android.content.Intent;
import android.graphics.Rect;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.View;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction;
import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.function.Supplier;

public final class GalleryA11yInstrumentation extends Instrumentation {
    private int checks;
    private final ArrayList<int[]> events = new ArrayList<>(); // type, content change types
    private final ArrayList<String> eventTexts = new ArrayList<>();

    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); start(); }
    private void check(boolean condition, String what) {
        if (!condition) throw new AssertionError(what);
        checks++;
    }
    private static <T> T until(Supplier<T> probe, String what) {
        long end = SystemClock.uptimeMillis() + 8000;
        while (SystemClock.uptimeMillis() < end) {
            T v = probe.get();
            if (v != null && !Boolean.FALSE.equals(v)) return v;
            SystemClock.sleep(50);
        }
        throw new AssertionError("timed out: " + what);
    }
    private static String label(AccessibilityNodeInfo n) {
        CharSequence t = n.getText() != null ? n.getText() : n.getContentDescription();
        return t == null ? "" : t.toString();
    }
    // the first node below root with this class whose text / description starts with text
    private static AccessibilityNodeInfo find(AccessibilityNodeInfo root, String cls, String text) {
        if (root == null) return null;
        if ((cls == null || cls.contentEquals(root.getClassName() == null ? "" : root.getClassName()))
                && label(root).startsWith(text)) return root;
        for (int i = 0; i < root.getChildCount(); i++) {
            AccessibilityNodeInfo found = find(root.getChild(i), cls, text);
            if (found != null) return found;
        }
        return null;
    }
    private AccessibilityNodeInfo node(UiAutomation ui, String cls, String text) {
        return until(() -> find(ui.getRootInActiveWindow(), cls, text), cls + " " + text);
    }
    private boolean sawEvent(int type, String text) {
        synchronized (events) {
            for (int i = 0; i < events.size(); i++)
                if (events.get(i)[0] == type && (text == null || eventTexts.get(i).contains(text))) return true;
        }
        return false;
    }
    private static Object field(Object object, String name) throws Exception {
        Field f = object.getClass().getDeclaredField(name);
        f.setAccessible(true);
        return f.get(object);
    }

    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            // an accessibility client: the app's AccessibilityManager reports accessibility on
            UiAutomation ui = getUiAutomation();
            ui.setOnAccessibilityEventListener(e -> {
                if (!"org.jflib.gallery".contentEquals(e.getPackageName() == null ? "" : e.getPackageName())) return;
                synchronized (events) {
                    events.add(new int[] {e.getEventType(), e.getContentChangeTypes()});
                    eventTexts.add(String.valueOf(e.getText()) + " " + e.getContentDescription());
                }
            });
            Intent intent = new Intent(getTargetContext(), GalleryActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            Activity activity = startActivitySync(intent);
            waitForIdleSync();

            // the tree: classes, texts, states, ranges, bounds
            AccessibilityNodeInfo box = node(ui, "android.widget.CheckBox", "Option aktivieren");
            check(box.isCheckable() && !box.isChecked() && box.isEnabled() && box.isClickable(), "check box: checkable, unchecked, clickable");
            Rect r = new Rect();
            box.getBoundsInScreen(r);
            View view = (View)field(activity, "gallery");
            int[] at = new int[2];
            view.getLocationOnScreen(at);
            check(r.width() > 50 && r.height() > 20 && r.left >= at[0] && r.top >= at[1]
                && r.right <= at[0] + view.getWidth() && r.bottom <= at[1] + view.getHeight(), "check box bounds inside the view: " + r);
            AccessibilityNodeInfo sw = node(ui, "android.widget.Switch", "Regler aktiv");
            check(sw.isChecked(), "switch on");
            node(ui, "android.widget.Button", "Klick mich");
            AccessibilityNodeInfo hero = node(ui, "android.widget.TextView", "Kleine Widgets.");
            // a section title (ui.heading): a heading (TalkBack's heading navigation); a text is none
            AccessibilityNodeInfo heading = node(ui, "android.widget.TextView", "01  Interaktion");
            check(heading.isHeading() && !hero.isHeading(), "section title a heading, a text none");
            AccessibilityNodeInfo slider = node(ui, "android.widget.SeekBar", "Wert: 40 %");
            check(slider.getRangeInfo() != null && slider.getRangeInfo().getCurrent() == 40f
                && slider.getRangeInfo().getMax() == 100f, "slider range 0..100 at 40");
            AccessibilityNodeInfo progress = node(ui, "android.widget.ProgressBar", "");
            check(progress.getRangeInfo() != null && progress.getRangeInfo().getCurrent() == 40f, "progress at 40");
            AccessibilityNodeInfo edit = node(ui, "android.widget.EditText", "");
            check(edit.isEditable() && !edit.isMultiLine() && !edit.isPassword(), "text box: editable, one line");
            check(edit.getHintText() != null && edit.getHintText().toString().contains("Texteingabe"), "text box named by the text above it");

            // actions: click toggles, clicks count
            check(box.performAction(AccessibilityNodeInfo.ACTION_CLICK), "click accepted");
            until(() -> box.refresh() && box.isChecked(), "check box checked after the click");
            checks++;
            AccessibilityNodeInfo button = node(ui, "android.widget.Button", "Klick mich");
            button.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            node(ui, "android.widget.TextView", "Klicks: 1");
            checks++;
            // a value: set progress, scroll forward (one twentieth of the range)
            Bundle progressValue = new Bundle();
            progressValue.putFloat(AccessibilityNodeInfo.ACTION_ARGUMENT_PROGRESS_VALUE, 73f);
            check(slider.performAction(AccessibilityAction.ACTION_SET_PROGRESS.getId(), progressValue), "set progress accepted");
            node(ui, "android.widget.TextView", "Wert: 73 %");
            checks++;
            AccessibilityNodeInfo slider2 = node(ui, "android.widget.SeekBar", "Wert: 73 %");
            slider2.performAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
            node(ui, "android.widget.TextView", "Wert: 78 %");
            checks++;
            // text: set it, focus, select; events for text and selection
            Bundle text = new Bundle();
            text.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, "Robo 🙂");
            check(edit.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, text), "set text accepted");
            node(ui, "android.widget.TextView", "Hallo Robo 🙂!");
            checks++;
            until(() -> edit.refresh() && "Robo 🙂".contentEquals(edit.getText()), "the box shows the text");
            checks++;
            until(() -> sawEvent(AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED, "Robo"), "TYPE_VIEW_TEXT_CHANGED");
            checks++;
            check(edit.performAction(AccessibilityNodeInfo.ACTION_FOCUS), "focus accepted");
            until(() -> edit.refresh() && edit.isFocused(), "the box has the input focus");
            checks++;
            until(() -> sawEvent(AccessibilityEvent.TYPE_VIEW_FOCUSED, null), "TYPE_VIEW_FOCUSED");
            checks++;
            Bundle selection = new Bundle();
            selection.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 1);
            selection.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, 7); // UTF-16: after the emoji
            check(edit.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, selection), "set selection accepted");
            until(() -> edit.refresh() && edit.getTextSelectionStart() == 1 && edit.getTextSelectionEnd() == 7, "selection 1..7 (UTF-16)");
            checks++;
            until(() -> sawEvent(AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED, null), "TYPE_VIEW_TEXT_SELECTION_CHANGED");
            checks++;
            // the accessibility focus (TalkBack's cursor)
            AccessibilityNodeInfo sw2 = node(ui, "android.widget.Switch", "Regler aktiv");
            check(sw2.performAction(AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS), "accessibility focus accepted");
            until(() -> sw2.refresh() && sw2.isAccessibilityFocused(), "switch has the accessibility focus");
            checks++;
            AccessibilityNodeInfo focused = ui.findFocus(AccessibilityNodeInfo.FOCUS_ACCESSIBILITY);
            check(focused != null && label(focused).startsWith("Regler aktiv"), "findFocus(accessibility)");
            until(() -> sawEvent(AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED, "Regler aktiv"), "TYPE_VIEW_ACCESSIBILITY_FOCUSED");
            checks++;
            // a disabled slider: off with the switch, no value action
            sw2.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            until(() -> {
                AccessibilityNodeInfo s = find(ui.getRootInActiveWindow(), "android.widget.SeekBar", "Wert: 78 %");
                return s != null && !s.isEnabled() && !s.getActionList().contains(AccessibilityAction.ACTION_SET_PROGRESS);
            }, "slider disabled without value actions");
            checks++;

            // explore by touch: the hit test finds the node under the check box's centre
            Object provider = field(view, "a11y");
            final String[] hit = new String[1];
            runOnMainSync(() -> {
                try {
                    AccessibilityNodeInfo b = find(ui.getRootInActiveWindow(), "android.widget.CheckBox", "Option aktivieren");
                    Rect c = new Rect();
                    b.getBoundsInScreen(c);
                    int vid = (Integer)provider.getClass().getDeclaredMethod("virtualAt", float.class, float.class)
                        .invoke(provider, (float)(c.centerX() - at[0]), (float)(c.centerY() - at[1]));
                    AccessibilityNodeInfo n = ((android.view.accessibility.AccessibilityNodeProvider)provider).createAccessibilityNodeInfo(vid);
                    hit[0] = n == null ? "none" : label(n);
                } catch (Exception error) { hit[0] = String.valueOf(error); }
            });
            check(hit[0].startsWith("Option aktivieren"), "hit test at the check box: " + hit[0]);
            // with touch exploration (requested by this client) hovering sends HOVER_ENTER
            AccessibilityServiceInfo info = ui.getServiceInfo();
            info.flags |= AccessibilityServiceInfo.FLAG_REQUEST_TOUCH_EXPLORATION_MODE;
            ui.setServiceInfo(info);
            android.view.accessibility.AccessibilityManager manager = (android.view.accessibility.AccessibilityManager)
                getTargetContext().getSystemService(android.content.Context.ACCESSIBILITY_SERVICE);
            long end = SystemClock.uptimeMillis() + 3000;
            while (!manager.isTouchExplorationEnabled() && SystemClock.uptimeMillis() < end) SystemClock.sleep(50);
            String hover = "touch exploration not available";
            if (manager.isTouchExplorationEnabled()) {
                runOnMainSync(() -> {
                    Rect c = new Rect();
                    box.refresh();
                    box.getBoundsInScreen(c);
                    long now = SystemClock.uptimeMillis();
                    android.view.MotionEvent e = android.view.MotionEvent.obtain(now, now, android.view.MotionEvent.ACTION_HOVER_ENTER,
                        c.centerX() - at[0], c.centerY() - at[1], 0);
                    e.setSource(android.view.InputDevice.SOURCE_TOUCHSCREEN);
                    view.dispatchGenericMotionEvent(e); // a pointer source: dispatchHoverEvent
                    e.recycle();
                });
                until(() -> sawEvent(AccessibilityEvent.TYPE_VIEW_HOVER_ENTER, "Option aktivieren"), "TYPE_VIEW_HOVER_ENTER on the check box");
                checks++;
                hover = "hover checked";
            }
            result.putInt("checks", checks);
            result.putString("stream", "PASS: Android accessibility tree, actions, events, focus, selection, hit test (" + checks
                + " checks; " + hover + ").\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putInt("checks", checks);
            result.putString("stream", "FAIL after " + checks + " checks: " + error + "\n");
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
