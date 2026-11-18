/* A real Android editable mirror: composing text is visible immediately in Rust.
 * Android's BaseInputConnection handles replacement spans, UTF-16 selection and IME queries.
 */
package org.jflib.gallery;

import android.content.Context;
import android.text.Editable;
import android.text.InputType;
import android.text.Selection;
import android.text.SpannableStringBuilder;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.ExtractedText;
import android.view.inputmethod.ExtractedTextRequest;
import android.view.inputmethod.InputMethodManager;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;

final class GalleryInputConnection extends BaseInputConnection {
    // the text box's flags in the snapshot (lib/ui consts.rs): TB_PASSWORD, TB_MULTI
    private static final int TB_PASSWORD = 0x0100_0000, TB_MULTI = 0x0800_0000;
    private final View view;
    private final Runnable wake;
    private final Editable editable = new SpannableStringBuilder();
    private int editorId, batches;
    // the box's kind (TB_PASSWORD or TB_MULTI or 0) now, and the one the EditorInfo was made for
    private int editorKind, infoKind;
    private int notifiedStart = -2, notifiedEnd = -2, notifiedCompStart = -2, notifiedCompEnd = -2;
    private String nativeText = "";
    private int nativeStart, nativeEnd;
    private boolean syncing, closed;

    GalleryInputConnection(View view, Runnable wake) {
        super(view, true);
        this.view = view;
        this.wake = wake;
        syncFromNative();
    }
    @Override public Editable getEditable() { return editable; }
    // the keyboard for the box's kind: a password one (no learning, no suggestions), a multi-line
    // one whose Enter breaks the line, else one line with "Done" (Enter for the app).  A fixed text
    // field gave Gboard a password to learn and a multi-line box no line break.
    void fillEditorInfo(EditorInfo info) {
        infoKind = editorKind;
        int type = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
        int options = EditorInfo.IME_FLAG_NO_EXTRACT_UI;
        if (editorKind == TB_PASSWORD) {
            type = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD;
            options |= EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING;
        } else if (editorKind == TB_MULTI) {
            type |= InputType.TYPE_TEXT_FLAG_MULTI_LINE;
            options |= EditorInfo.IME_FLAG_NO_ENTER_ACTION;
        } else {
            options |= EditorInfo.IME_ACTION_DONE;
        }
        info.inputType = type;
        info.imeOptions = options;
        info.initialSelStart = Selection.getSelectionStart(editable);
        info.initialSelEnd = Selection.getSelectionEnd(editable);
    }
    // (frames) another kind of box has the keyboard than the EditorInfo was made for: the view
    // restarts the input (a new connection, a new EditorInfo)
    boolean needsRestart() { return !closed && editorId != 0 && editorKind != infoKind; }
    private InputMethodManager ime() {
        return (InputMethodManager)view.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
    }
    void syncFromNative() {
        if (closed || syncing || batches != 0) return;
        byte[] snapshot = GalleryActivity.nativeEditorState();
        if (snapshot == null || snapshot.length < 16) return;
        ByteBuffer header = ByteBuffer.wrap(snapshot).order(ByteOrder.LITTLE_ENDIAN);
        int id = header.getInt(), flags = header.getInt();
        int anchor = header.getInt(), caret = header.getInt();
        int length = snapshot.length - 16;
        if (anchor < 0 || caret < 0 || anchor > length || caret > length) return;
        String text = new String(snapshot, 16, length, StandardCharsets.UTF_8);
        int start = new String(snapshot, 16, anchor, StandardCharsets.UTF_8).length();
        int end = new String(snapshot, 16, caret, StandardCharsets.UTF_8).length();
        syncing = true;
        // (a password box is one line also with TB_MULTI: lib/ui edit.rs)
        editorKind = id == 0 ? 0 : (flags & TB_PASSWORD) != 0 ? TB_PASSWORD : (flags & TB_MULTI) != 0 ? TB_MULTI : 0;
        if (editorId != id || !editable.toString().equals(text)) {
            removeComposingSpans(editable);
            editable.replace(0, editable.length(), text);
        }
        editorId = id;
        nativeText = text;
        nativeStart = start;
        nativeEnd = end;
        Selection.setSelection(editable, start, end);
        syncing = false;
        int compStart = getComposingSpanStart(editable), compEnd = getComposingSpanEnd(editable);
        if (start != notifiedStart || end != notifiedEnd || compStart != notifiedCompStart || compEnd != notifiedCompEnd) {
            notifiedStart = start; notifiedEnd = end;
            notifiedCompStart = compStart; notifiedCompEnd = compEnd;
            ime().updateSelection(view, start, end, compStart, compEnd);
        }
    }
    private void publish() {
        if (syncing || closed || batches != 0 || editorId == 0) return;
        String text = editable.toString();
        int start = Math.max(0, Selection.getSelectionStart(editable));
        int end = Math.max(0, Selection.getSelectionEnd(editable));
        if (nativeText.equals(text) && nativeStart == start && nativeEnd == end) {
            syncFromNative();
            return;
        }
        GalleryActivity.nativeEditorReplace(editorId, text.getBytes(StandardCharsets.UTF_8),
            text.substring(0, start).getBytes(StandardCharsets.UTF_8).length,
            text.substring(0, end).getBytes(StandardCharsets.UTF_8).length);
        syncFromNative(); // Respect capacity, filters and the Rust editor's actual cursor.
        wake.run();
    }
    @Override public boolean beginBatchEdit() { batches++; return true; }
    @Override public boolean endBatchEdit() {
        if (batches > 0) batches--;
        if (batches == 0) publish();
        return batches > 0;
    }
    @Override public boolean setComposingText(CharSequence text, int cursor) {
        boolean ok = super.setComposingText(text, cursor); publish(); return ok;
    }
    @Override public boolean commitText(CharSequence text, int cursor) {
        boolean ok = super.commitText(text, cursor); publish(); return ok;
    }
    @Override public boolean finishComposingText() {
        boolean ok = super.finishComposingText(); publish(); return ok;
    }
    @Override public boolean setComposingRegion(int start, int end) {
        boolean ok = super.setComposingRegion(start, end); publish(); return ok;
    }
    @Override public boolean setSelection(int start, int end) {
        boolean ok = super.setSelection(start, end); publish(); return ok;
    }
    @Override public boolean deleteSurroundingText(int before, int after) {
        boolean ok = super.deleteSurroundingText(before, after); publish(); return ok;
    }
    @Override public boolean deleteSurroundingTextInCodePoints(int before, int after) {
        boolean ok = super.deleteSurroundingTextInCodePoints(before, after); publish(); return ok;
    }
    @Override public ExtractedText getExtractedText(ExtractedTextRequest request, int flags) {
        ExtractedText text = new ExtractedText();
        text.text = editable.toString();
        text.selectionStart = Selection.getSelectionStart(editable);
        text.selectionEnd = Selection.getSelectionEnd(editable);
        text.partialStartOffset = -1;
        text.partialEndOffset = -1;
        text.flags = editorKind == TB_MULTI ? 0 : ExtractedText.FLAG_SINGLE_LINE;
        return text;
    }
    @Override public boolean sendKeyEvent(KeyEvent event) {
        if (event.getAction() == KeyEvent.ACTION_DOWN) view.onKeyDown(event.getKeyCode(), event);
        syncFromNative();
        return true;
    }
    // "Done" of a one-line box: Enter for the app (EV_COMMIT; it never got one), then the keyboard
    // goes; a multi-line box has no action (its Enter is a key)
    @Override public boolean performEditorAction(int action) {
        finishComposingText();
        if (editorId != 0 && editorKind != TB_MULTI) {
            view.onKeyDown(KeyEvent.KEYCODE_ENTER, new KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER));
            syncFromNative();
        }
        ime().hideSoftInputFromWindow(view.getWindowToken(), 0);
        return true;
    }
    @Override public void closeConnection() {
        // An obsolete connection must never overwrite a newly focused editor.
        closed = true;
        super.closeConnection();
    }
}
