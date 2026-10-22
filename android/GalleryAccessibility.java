/* The Rust UI's accessibility tree as virtual views of the host view: an
 * AccessibilityNodeProvider kept current from the differences the library sends after its
 * frames (lib/plat/a11y_wire.rs: the format, lib/plat/web_a11y.rs: the bridge), explore by
 * touch (hover), the accessibility focus, events for TalkBack, and its actions back to Rust
 * (click, focus, value, text, selection). All calls run on the UI thread, like the library.
 */
package org.jflib.gallery;

import android.content.Context;
import android.graphics.Rect;
import android.os.Build;
import android.os.Bundle;
import android.text.InputType;
import android.text.SpannableString;
import android.text.Spanned;
import android.text.style.URLSpan;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewParent;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityManager;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction;
import android.view.accessibility.AccessibilityNodeInfo.CollectionItemInfo;
import android.view.accessibility.AccessibilityNodeInfo.RangeInfo;
import android.view.accessibility.AccessibilityNodeProvider;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.function.DoubleSupplier;

final class GalleryAccessibility extends AccessibilityNodeProvider {
    // plat::a11y::Role in declaration order
    static final int GROUP = 0, LABEL = 1, BUTTON = 2, CHECKBOX = 3, RADIO = 4, SWITCH = 5, SLIDER = 6,
        PROGRESS = 7, TEXTBOX = 8, LINK = 9, LIST = 10, LISTITEM = 11, MENUBAR = 12, MENU = 13, MENUITEM = 14,
        TABLIST = 15, TAB = 16, COMBO = 17, IMAGE = 18, CANVAS = 19, DIALOG = 20, HEADING = 21, WINDOW = -1;
    // plat::a11y::S_* and A_*
    static final int S_FOCUSED = 1, S_FOCUSABLE = 2, S_DISABLED = 4, S_CHECKED = 8, S_MIXED = 16, S_SELECTED = 32,
        S_EXPANDED = 64, S_PRESSED = 128, S_MULTILINE = 256, S_PROTECTED = 512, S_READONLY = 1024;
    static final int A_CLICK = 1, A_FOCUS = 2, A_VALUE = 4, A_TEXT = 8;
    // jfgui_a11y_act kinds and the update events (a11y_wire.rs E_*)
    static final int ACT_CLICK = 1, ACT_FOCUS = 2, ACT_VALUE = 3, ACT_TEXT = 4, ACT_SELECT = 5;
    static final int E_WINDOW = 1, E_ACTIVE = 2, E_FOCUS = 3, E_STATE = 4, E_NAME = 5, E_VALUE = 6, E_TEXT = 7, E_CARET = 8;

    /** A node of the tree, or a window (role WINDOW). */
    static final class Node {
        int vid, win, role = GROUP, state, actions, caret = -1, anchor = -1;
        long key;
        int[] rect = new int[4], caretRect = new int[4];
        String name = "", text = "";
        double value, min, max, step;
        Node parent;
        final ArrayList<Node> children = new ArrayList<>();
    }
    static final class Win {
        Node node;
        int flags;
        int[] place = new int[4];
        long focus;
        String title = "";
        final LinkedHashMap<Long, Node> nodes = new LinkedHashMap<>();
    }

    private final View view;
    private final DoubleSupplier scale; // view px per library px
    private final Runnable wake;
    private final AccessibilityManager manager;
    private LinkedHashMap<Integer, Win> wins = new LinkedHashMap<>();
    private final HashMap<Integer, Node> byVid = new HashMap<>();
    private int nextVid = 1, focusVid = View.NO_ID, hoverVid = View.NO_ID;
    private boolean on;
    int updates; // updates applied (tests)

    GalleryAccessibility(View view, DoubleSupplier scale, Runnable wake) {
        this.view = view;
        this.scale = scale;
        this.wake = wake;
        manager = (AccessibilityManager)view.getContext().getSystemService(Context.ACCESSIBILITY_SERVICE);
        manager.addAccessibilityStateChangeListener(enabled -> enable(enabled));
    }

    /** The mirror on while accessibility services listen (Rust then sends its tree). */
    void enable(boolean enabled) {
        if (enabled == on) return;
        on = enabled;
        if (!on) { wins.clear(); byVid.clear(); focusVid = hoverVid = View.NO_ID; }
        GalleryActivity.nativeA11y(on);
        wake.run();
    }
    void resume() { enable(manager.isEnabled()); }

    /** After a frame: the updates the library sent since the last call. */
    void pull() {
        if (!on) return;
        byte[] data = GalleryActivity.nativeA11yTake();
        if (data == null || data.length == 0) return;
        ByteBuffer b = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
        boolean structure = false;
        ArrayList<Node> changed = new ArrayList<>();
        ArrayList<int[]> events = new ArrayList<>();
        ArrayList<Object> extra = new ArrayList<>();
        try {
            while (b.remaining() >= 4) {
                int length = b.getInt(), end = b.position() + length;
                ByteBuffer u = (ByteBuffer)b.duplicate().order(ByteOrder.LITTLE_ENDIAN).limit(end);
                structure |= update(u, changed, events, extra);
                b.position(end);
            }
        } catch (RuntimeException broken) {
            // a mirror that went wrong: everything anew with the next frame
            wins.clear(); byVid.clear();
            GalleryActivity.nativeA11y(true);
            wake.run();
            return;
        }
        updates++;
        if (!manager.isEnabled()) return;
        if (structure || changed.size() > 16) sendHost(AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE);
        else for (Node n : changed) if (byVid.get(n.vid) == n) send(n, AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED, AccessibilityEvent.CONTENT_CHANGE_TYPE_UNDEFINED);
        for (int i = 0; i < events.size(); i++) event(events.get(i), (String)extra.get(i));
    }

    private static String str(ByteBuffer b) {
        int n = b.getInt();
        if (n < 0 || n > b.remaining()) throw new IllegalStateException("a11y: string beyond the update");
        byte[] bytes = new byte[n];
        b.get(bytes);
        return new String(bytes, StandardCharsets.UTF_8);
    }
    private static void rect(ByteBuffer b, int[] r) { for (int i = 0; i < 4; i++) r[i] = b.getInt(); }
    private Node fresh(int win, long key, int role) {
        Node n = new Node();
        n.vid = nextVid++;
        n.win = win; n.key = key; n.role = role;
        byVid.put(n.vid, n);
        return n;
    }
    private void drop(Node n) {
        byVid.remove(n.vid);
        if (focusVid == n.vid) focusVid = View.NO_ID;
        if (hoverVid == n.vid) hoverVid = View.NO_ID;
    }

    // one update -> the structure changed
    private boolean update(ByteBuffer b, ArrayList<Node> changed, ArrayList<int[]> events, ArrayList<Object> extra) {
        if ((b.get() & 255) != 1) throw new IllegalStateException("a11y: unknown update version");
        // a full update lists every window with its order: nodes that stay keep their ids
        boolean structure = (b.get() & 1) != 0;
        LinkedHashMap<Integer, Win> next = new LinkedHashMap<>();
        for (int count = b.get() & 255; count > 0; count--) {
            int win = b.get() & 255;
            Win w = wins.get(win);
            if (w == null) { w = new Win(); w.node = fresh(win, 0, WINDOW); structure = true; }
            w.flags = b.get() & 255;
            rect(b, w.place);
            w.focus = b.getLong();
            w.title = str(b);
            w.node.rect = new int[] {0, 0, w.place[2], w.place[3]};
            w.node.name = w.title;
            int order = b.getInt();
            if (order != -1) {
                structure = true;
                LinkedHashMap<Long, Node> nodes = new LinkedHashMap<>();
                Node[] list = new Node[order];
                for (int i = 0; i < order; i++) {
                    long key = b.getLong();
                    int parent = b.getInt();
                    Node n = w.nodes.get(key);
                    if (n == null) n = fresh(win, key, GROUP);
                    if (parent != -1 && (parent < 0 || parent >= i)) throw new IllegalStateException("a11y: parent after child");
                    n.parent = parent == -1 ? w.node : list[parent];
                    list[i] = n;
                    nodes.put(key, n);
                }
                for (Node n : w.nodes.values()) if (nodes.get(n.key) != n) drop(n);
                w.node.children.clear();
                for (Node n : list) n.children.clear();
                for (Node n : list) n.parent.children.add(n);
                w.nodes.clear();
                w.nodes.putAll(nodes);
            } else if (!wins.containsKey(win)) throw new IllegalStateException("a11y: a new window without its order");
            for (int count2 = b.getInt(); count2 > 0; count2--) {
                Node n = w.nodes.get(b.getLong());
                if (n == null) throw new IllegalStateException("a11y: a node outside the order");
                n.role = b.get();
                n.state = b.getInt();
                n.actions = b.getInt();
                rect(b, n.rect);
                n.name = str(b);
                n.text = str(b);
                n.caret = b.getInt();
                n.anchor = b.getInt();
                rect(b, n.caretRect);
                n.value = b.getDouble(); n.min = b.getDouble(); n.max = b.getDouble(); n.step = b.getDouble();
                changed.add(n);
            }
            next.put(win, w);
        }
        for (Win w : wins.values()) if (!next.containsKey(w.node.win)) {
            structure = true;
            drop(w.node);
            for (Node n : w.nodes.values()) drop(n);
        }
        wins = next;
        for (int count = b.getInt(); count > 0; count--) {
            int kind = b.get() & 255, win = b.get() & 255;
            long key = b.getLong();
            int a = b.getInt(), c = b.getInt();
            events.add(new int[] {kind, win, (int)key, (int)(key >>> 32), a, c});
            extra.add(kind == E_TEXT ? str(b) : null);
        }
        if (b.hasRemaining()) throw new IllegalStateException("a11y: bytes after the update");
        return structure;
    }

    private Node node(int win, long key) {
        Win w = wins.get(win);
        return w == null ? null : key == 0 ? w.node : w.nodes.get(key);
    }
    private static int units(String text, int chars) {
        int i = 0;
        for (int c = 0; c < chars && i < text.length(); c++) i += Character.charCount(text.codePointAt(i));
        return i;
    }
    private static int chars(String text, int units) {
        return text.codePointCount(0, Math.max(0, Math.min(units, text.length())));
    }

    // an event of the update for TalkBack
    private void event(int[] e, String old) {
        Node n = node(e[1], (e[2] & 0xFFFFFFFFL) | ((long)e[3] << 32));
        if (n == null) return;
        switch (e[0]) {
            case E_WINDOW:
                if (e[4] != 0 && n.role == WINDOW) {
                    AccessibilityEvent ev = obtain(n, AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED);
                    if (Build.VERSION.SDK_INT >= 28) ev.setContentChangeTypes(AccessibilityEvent.CONTENT_CHANGE_TYPE_PANE_APPEARED);
                    ev.getText().add(n.name);
                    dispatch(ev);
                }
                break;
            case E_FOCUS:
                if (n.role != WINDOW && view.hasFocus()) send(n, AccessibilityEvent.TYPE_VIEW_FOCUSED, -1);
                break;
            case E_STATE: case E_VALUE:
                send(n, AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED, Build.VERSION.SDK_INT >= 30
                    ? AccessibilityEvent.CONTENT_CHANGE_TYPE_STATE_DESCRIPTION : AccessibilityEvent.CONTENT_CHANGE_TYPE_UNDEFINED);
                break;
            case E_NAME:
                send(n, AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED, AccessibilityEvent.CONTENT_CHANGE_TYPE_TEXT);
                break;
            case E_TEXT: {
                String now = n.text, before = old == null ? "" : old;
                int start = 0;
                while (start < now.length() && start < before.length() && now.charAt(start) == before.charAt(start)) start++;
                int endNow = now.length(), endOld = before.length();
                while (endNow > start && endOld > start && now.charAt(endNow - 1) == before.charAt(endOld - 1)) { endNow--; endOld--; }
                AccessibilityEvent ev = obtain(n, AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED);
                ev.setBeforeText(before);
                ev.setFromIndex(start);
                ev.setAddedCount(endNow - start);
                ev.setRemovedCount(endOld - start);
                ev.getText().add(now);
                if ((n.state & S_PROTECTED) != 0) ev.setPassword(true);
                dispatch(ev);
                break;
            }
            case E_CARET: {
                AccessibilityEvent ev = obtain(n, AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED);
                int a = units(n.text, n.anchor >= 0 ? n.anchor : n.caret), c = units(n.text, n.caret);
                ev.setFromIndex(Math.min(a, c));
                ev.setToIndex(Math.max(a, c));
                ev.setItemCount(n.text.length());
                ev.getText().add(n.text);
                if ((n.state & S_PROTECTED) != 0) ev.setPassword(true);
                dispatch(ev);
                break;
            }
            default: break;
        }
    }
    @SuppressWarnings("deprecation")
    private AccessibilityEvent obtain(Node n, int type) {
        AccessibilityEvent ev = AccessibilityEvent.obtain(type);
        ev.setPackageName(view.getContext().getPackageName());
        ev.setClassName(className(n));
        ev.setSource(view, n.vid);
        ev.setEnabled((n.state & S_DISABLED) == 0);
        if (n.role != TEXTBOX && !n.name.isEmpty()) ev.setContentDescription(n.name);
        return ev;
    }
    private void dispatch(AccessibilityEvent ev) {
        ViewParent parent = view.getParent();
        if (parent != null && manager.isEnabled()) parent.requestSendAccessibilityEvent(view, ev);
    }
    private void send(Node n, int type, int change) {
        AccessibilityEvent ev = obtain(n, type);
        if (change >= 0) ev.setContentChangeTypes(change);
        dispatch(ev);
    }
    @SuppressWarnings("deprecation")
    private void sendHost(int change) {
        AccessibilityEvent ev = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED);
        ev.setPackageName(view.getContext().getPackageName());
        ev.setSource(view);
        ev.setContentChangeTypes(change);
        dispatch(ev);
    }

    private static String className(Node n) {
        switch (n.role) {
            case LABEL: case HEADING: case LINK: case LISTITEM: case MENUITEM: case TAB: return "android.widget.TextView";
            case BUTTON: return (n.state & S_PRESSED) != 0 ? "android.widget.ToggleButton" : "android.widget.Button";
            case CHECKBOX: return "android.widget.CheckBox";
            case RADIO: return "android.widget.RadioButton";
            case SWITCH: return "android.widget.Switch";
            case SLIDER: return "android.widget.SeekBar";
            case PROGRESS: return "android.widget.ProgressBar";
            case TEXTBOX: return "android.widget.EditText";
            case LIST: return "android.widget.ListView";
            case TABLIST: return "android.widget.TabWidget";
            case MENU: return n.key < 0 ? "android.widget.ListView" : "android.widget.Button"; // a popup / a menu bar entry
            case COMBO: return "android.widget.Spinner";
            case IMAGE: case CANVAS: return "android.widget.ImageView";
            default: return "android.view.ViewGroup";
        }
    }

    // a node's rectangle in view px (windows: their client area)
    private Rect local(Node n) {
        Win w = wins.get(n.win);
        double k = scale.getAsDouble();
        int x = w.place[0] + (n.role == WINDOW ? 0 : n.rect[0]), y = w.place[1] + (n.role == WINDOW ? 0 : n.rect[1]);
        return new Rect((int)Math.floor(x * k), (int)Math.floor(y * k),
            (int)Math.ceil((x + Math.max(1, n.rect[2])) * k), (int)Math.ceil((y + Math.max(1, n.rect[3])) * k));
    }

    @Override public AccessibilityNodeInfo createAccessibilityNodeInfo(int vid) {
        if (vid == HOST_VIEW_ID) {
            AccessibilityNodeInfo info = AccessibilityNodeInfo.obtain(view);
            view.onInitializeAccessibilityNodeInfo(info);
            for (Win w : wins.values()) info.addChild(view, w.node.vid);
            if (!wins.isEmpty()) info.setContentDescription(null); // the nodes speak for themselves
            return info;
        }
        Node n = byVid.get(vid);
        if (n == null) return null;
        AccessibilityNodeInfo info = AccessibilityNodeInfo.obtain(view, vid);
        info.setPackageName(view.getContext().getPackageName());
        info.setClassName(className(n));
        info.setSource(view, vid);
        if (n.parent == null) info.setParent(view);
        else info.setParent(view, n.parent.vid);
        for (Node c : n.children) info.addChild(view, c.vid);
        Rect box = local(n), shown = new Rect(0, 0, view.getWidth(), view.getHeight());
        Rect parentBox = n.parent == null ? new Rect(0, 0, 0, 0) : local(n.parent);
        Rect inParent = new Rect(box);
        inParent.offset(-parentBox.left, -parentBox.top);
        setBoundsInParent(info, inParent);
        int[] at = new int[2];
        view.getLocationOnScreen(at);
        Rect screen = new Rect(box);
        boolean visible = screen.intersect(shown);
        screen.offset(at[0], at[1]);
        info.setBoundsInScreen(screen);
        info.setVisibleToUser(visible && view.isShown());
        info.setImportantForAccessibility(true);
        boolean enabled = (n.state & S_DISABLED) == 0;
        info.setEnabled(enabled);
        if (n.role == WINDOW) {
            Win w = wins.get(n.win);
            if (Build.VERSION.SDK_INT >= 28 && w != null && (w.flags & 1) != 0) info.setPaneTitle(n.name);
            addAccessibilityFocus(info, vid);
            return info;
        }
        info.setFocusable((n.state & S_FOCUSABLE) != 0);
        info.setFocused((n.state & S_FOCUSED) != 0 && view.hasFocus() && wins.get(n.win) != null && (wins.get(n.win).flags & 2) != 0);
        info.setSelected((n.state & S_SELECTED) != 0);
        boolean checkable = n.role == CHECKBOX || n.role == RADIO || n.role == SWITCH
            || (n.role == MENUITEM && (n.state & S_CHECKED) != 0) || (n.role == BUTTON && (n.state & S_PRESSED) != 0);
        info.setCheckable(checkable);
        info.setChecked((n.state & (S_CHECKED | S_PRESSED)) != 0 && (n.state & S_MIXED) == 0);
        switch (n.role) {
            case TEXTBOX: {
                boolean multi = (n.state & S_MULTILINE) != 0, password = (n.state & S_PROTECTED) != 0;
                info.setText(n.text);
                info.setHintText(n.name);
                info.setEditable((n.state & S_READONLY) == 0 && enabled);
                info.setMultiLine(multi);
                info.setPassword(password);
                info.setInputType(InputType.TYPE_CLASS_TEXT | (multi ? InputType.TYPE_TEXT_FLAG_MULTI_LINE : 0)
                    | (password ? InputType.TYPE_TEXT_VARIATION_PASSWORD : 0));
                if (n.caret >= 0) {
                    int a = units(n.text, n.anchor >= 0 ? n.anchor : n.caret), c = units(n.text, n.caret);
                    info.setTextSelection(Math.min(a, c), Math.max(a, c));
                }
                info.setMovementGranularities(AccessibilityNodeInfo.MOVEMENT_GRANULARITY_CHARACTER
                    | AccessibilityNodeInfo.MOVEMENT_GRANULARITY_WORD | AccessibilityNodeInfo.MOVEMENT_GRANULARITY_LINE);
                break;
            }
            case SLIDER: case PROGRESS:
                if (!n.name.isEmpty()) info.setContentDescription(n.name);
                if (n.value >= n.min && n.max > n.min)
                    info.setRangeInfo(RangeInfo.obtain(n.step >= 1 ? RangeInfo.RANGE_TYPE_INT : RangeInfo.RANGE_TYPE_FLOAT,
                        (float)n.min, (float)n.max, (float)n.value));
                break;
            case COMBO:
                info.setText(n.text);
                if (!n.name.isEmpty()) info.setContentDescription(n.name + ", " + n.text);
                break;
            case IMAGE: case CANVAS: case LIST: case TABLIST: case MENUBAR: case GROUP: case DIALOG:
                if (!n.name.isEmpty()) info.setContentDescription(n.name);
                if (n.role == DIALOG && Build.VERSION.SDK_INT >= 28) info.setPaneTitle(n.name);
                break;
            case LINK: {
                SpannableString s = new SpannableString(n.name);
                s.setSpan(new URLSpan(""), 0, s.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
                info.setText(s);
                break;
            }
            case HEADING:
                info.setText(n.name);
                setHeading(info);
                break;
            default:
                if (n.role != MENU || n.key >= 0) info.setText(n.name);
                else if (!n.name.isEmpty()) info.setContentDescription(n.name);
                break;
        }
        if (enabled) {
            if ((n.actions & A_CLICK) != 0) {
                info.setClickable(true);
                info.addAction(AccessibilityAction.ACTION_CLICK);
                if (n.role == COMBO || (n.role == MENU && n.key >= 0))
                    info.addAction((n.state & S_EXPANDED) != 0 ? AccessibilityAction.ACTION_COLLAPSE : AccessibilityAction.ACTION_EXPAND);
            }
            if ((n.actions & A_FOCUS) != 0 && (n.state & S_FOCUSED) == 0) info.addAction(AccessibilityAction.ACTION_FOCUS);
            if ((n.actions & A_VALUE) != 0) {
                info.addAction(AccessibilityAction.ACTION_SET_PROGRESS);
                if (n.value < n.max) info.addAction(AccessibilityAction.ACTION_SCROLL_FORWARD);
                if (n.value > n.min) info.addAction(AccessibilityAction.ACTION_SCROLL_BACKWARD);
            }
            if ((n.actions & A_TEXT) != 0 && (n.state & S_READONLY) == 0) info.addAction(AccessibilityAction.ACTION_SET_TEXT);
            if (n.role == TEXTBOX && (n.actions & A_FOCUS) != 0) info.addAction(AccessibilityAction.ACTION_SET_SELECTION);
        }
        addAccessibilityFocus(info, vid);
        return info;
    }
    @SuppressWarnings("deprecation")
    private static void setBoundsInParent(AccessibilityNodeInfo info, Rect r) { info.setBoundsInParent(r); }
    // a heading (TalkBack: "heading", its heading navigation; Android has no level): API 28+
    // setHeading; API 26 / 27 the heading flag of a CollectionItemInfo, which isHeading() (and
    // androidx's AccessibilityNodeInfoCompat, which TalkBack uses) reads there as well
    @SuppressWarnings("deprecation")
    private static void setHeading(AccessibilityNodeInfo info) {
        if (Build.VERSION.SDK_INT >= 28) info.setHeading(true);
        else info.setCollectionItemInfo(CollectionItemInfo.obtain(0, 1, 0, 1, true));
    }
    private void addAccessibilityFocus(AccessibilityNodeInfo info, int vid) {
        info.setAccessibilityFocused(focusVid == vid);
        info.addAction(focusVid == vid ? AccessibilityAction.ACTION_CLEAR_ACCESSIBILITY_FOCUS : AccessibilityAction.ACTION_ACCESSIBILITY_FOCUS);
    }

    @Override public AccessibilityNodeInfo findFocus(int focus) {
        if (focus == AccessibilityNodeInfo.FOCUS_ACCESSIBILITY) return focusVid == View.NO_ID ? null : createAccessibilityNodeInfo(focusVid);
        if (focus == AccessibilityNodeInfo.FOCUS_INPUT && view.hasFocus())
            for (Win w : wins.values()) if ((w.flags & 2) != 0 && w.focus != 0) {
                Node n = w.nodes.get(w.focus);
                if (n != null) return createAccessibilityNodeInfo(n.vid);
            }
        return null;
    }

    @Override public boolean performAction(int vid, int action, Bundle args) {
        if (vid == HOST_VIEW_ID) return view.performAccessibilityAction(action, args);
        Node n = byVid.get(vid);
        if (n == null) return false;
        if (action == AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS) {
            if (focusVid == vid) return false;
            Node old = byVid.get(focusVid);
            focusVid = vid;
            if (old != null) send(old, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED, -1);
            send(n, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED, -1);
            view.invalidate();
            return true;
        }
        if (action == AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS) {
            if (focusVid != vid) return false;
            focusVid = View.NO_ID;
            send(n, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED, -1);
            view.invalidate();
            return true;
        }
        if (n.role == WINDOW || (n.state & S_DISABLED) != 0) return false;
        double step = Math.max(n.step, (n.max - n.min) / 20);
        switch (action) {
            case AccessibilityNodeInfo.ACTION_CLICK: case AccessibilityNodeInfo.ACTION_EXPAND: case AccessibilityNodeInfo.ACTION_COLLAPSE:
                if ((n.actions & A_CLICK) != 0) return act(n, ACT_CLICK, 0, 0, 0, null);
                return (n.actions & A_FOCUS) != 0 && act(n, ACT_FOCUS, 0, 0, 0, null);
            case AccessibilityNodeInfo.ACTION_FOCUS:
                if ((n.actions & A_FOCUS) == 0) return false;
                view.requestFocus();
                return act(n, ACT_FOCUS, 0, 0, 0, null);
            case AccessibilityNodeInfo.ACTION_SCROLL_FORWARD: case AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD:
                if ((n.actions & A_VALUE) == 0) return false;
                double v = n.value + (action == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD ? step : -step);
                return act(n, ACT_VALUE, 0, 0, Math.max(n.min, Math.min(n.max, v)), null);
            case AccessibilityNodeInfo.ACTION_SET_TEXT: {
                if ((n.actions & A_TEXT) == 0 || (n.state & S_READONLY) != 0) return false;
                CharSequence t = args == null ? null : args.getCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE);
                return act(n, ACT_TEXT, 0, 0, 0, t == null ? "" : t.toString());
            }
            case AccessibilityNodeInfo.ACTION_SET_SELECTION: {
                if (n.role != TEXTBOX || args == null) return false;
                int s = args.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, -1);
                int e = args.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, s);
                if (s < 0) return false;
                return act(n, ACT_SELECT, chars(n.text, s), chars(n.text, e), 0, null);
            }
            default:
                if (action == android.R.id.accessibilityActionSetProgress && (n.actions & A_VALUE) != 0 && args != null)
                    return act(n, ACT_VALUE, 0, 0, args.getFloat(AccessibilityNodeInfo.ACTION_ARGUMENT_PROGRESS_VALUE, (float)n.value), null);
                return false;
        }
    }
    private boolean act(Node n, int kind, int a, int b, double value, String text) {
        byte[] bytes = text == null ? new byte[0] : text.getBytes(StandardCharsets.UTF_8);
        GalleryActivity.nativeA11yAct(n.win, (int)n.key, (int)(n.key >>> 32), kind, a, b, value, bytes);
        wake.run();
        return true;
    }

    /** The deepest node at view px x, y (later windows and siblings lie on top) or NO_ID. */
    int virtualAt(float x, float y) {
        double k = scale.getAsDouble();
        int px = (int)Math.floor(x / k), py = (int)Math.floor(y / k);
        ArrayList<Win> list = new ArrayList<>(wins.values());
        for (int i = list.size() - 1; i >= 0; i--) {
            Win w = list.get(i);
            int lx = px - w.place[0], ly = py - w.place[1];
            if (lx < 0 || ly < 0 || lx >= w.place[2] || ly >= w.place[3]) continue;
            Node found = null;
            ArrayList<Node> level = w.node.children;
            for (boolean deeper = true; deeper; ) {
                deeper = false;
                for (int j = level.size() - 1; j >= 0; j--) {
                    Node c = level.get(j);
                    if (lx >= c.rect[0] && ly >= c.rect[1] && lx < c.rect[0] + c.rect[2] && ly < c.rect[1] + c.rect[3]) {
                        found = c; level = c.children; deeper = true;
                        break;
                    }
                }
            }
            return found != null ? found.vid : w.node.vid;
        }
        return View.NO_ID;
    }

    /** Explore by touch: the hovered node gets HOVER_ENTER (TalkBack moves its focus there). */
    boolean hover(MotionEvent e) {
        if (!on || !manager.isEnabled() || !manager.isTouchExplorationEnabled()) return false;
        int vid = e.getActionMasked() == MotionEvent.ACTION_HOVER_EXIT ? View.NO_ID : virtualAt(e.getX(), e.getY());
        if (vid != hoverVid) {
            Node old = byVid.get(hoverVid), now = byVid.get(vid);
            hoverVid = vid;
            if (now != null) send(now, AccessibilityEvent.TYPE_VIEW_HOVER_ENTER, -1);
            if (old != null) send(old, AccessibilityEvent.TYPE_VIEW_HOVER_EXIT, -1);
        }
        return vid != View.NO_ID;
    }
}
