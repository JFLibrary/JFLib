/* The system services plat asks for (lib/plat/android_host.rs, web.rs; bridge.c):
 * call(kind, bytes) answers at once on the UI thread -- the ContentResolver behind content://
 * names, the ClipboardManager, the persisted grants, the secret store (GallerySecrets: the
 * Android Keystore); a ContentObserver per provider whose tree is listed reports changes
 * (nativeChanged); request(kind, id, bytes) starts the
 * system's picker (Storage Access Framework) and answers later through nativeResult.
 * Wire: little endian; strings and byte blocks as u32 length + bytes (android_host.rs). */
package org.jflib.gallery;

import android.app.Activity;
import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ClipboardManager;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.content.UriPermission;
import android.database.ContentObserver;
import android.database.Cursor;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
import android.util.Log;
import android.util.SparseArray;
import android.webkit.MimeTypeMap;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;

final class GalleryHost {
    static native void nativeResult(int id, int status, byte[] uri);
    static native void nativeChanged();

    // android_host.rs
    static final int CALL_QUERY = 1, CALL_CHILDREN = 2, CALL_CREATE = 3, CALL_DELETE = 4, CALL_RENAME = 5,
        CALL_OPEN = 6, CALL_CLIP_GET = 10, CALL_CLIP_SET = 11, CALL_CLIP_HAS = 12, CALL_CLIP_STAMP = 13, CALL_CLIP_OURS = 14,
        CALL_GRANTS = 20, CALL_RELEASE = 21;
    // web.rs requests and plat consts
    static final int REQUEST_DOWNLOAD = 2, REQUEST_OPEN = 3, REQUEST_FILE_DIALOG = 12;
    static final int FD_SAVE = 1, FD_FOLDER = 2, FO_READ = 0, FO_WRITE = 1, FO_RW = 2;
    static final int CLIP_TEXT = 1, CLIP_IMAGE = 3, CLIP_HTML = 4;
    static final int DOC_WRITE = 1, DOC_DELETE = 2, DOC_RENAME = 4, DOC_CREATE = 8;
    static final int DONE = 2, FAILED = -5, CANCELLED = -4, EXPORTED = 1;
    private static final String TAG = "jfgui";
    private static final long CLIP_MAX = 64L << 20;
    private static final int REQUEST_BASE = 0x4A00;

    private static Activity activity;
    // the application (the secret store needs no window)
    private static Context app;
    // request code -> {id, kind, payload}
    private static final SparseArray<Object[]> pending = new SparseArray<>();
    private static int nextCode;

    static void attach(Activity a) { activity = a; app = a.getApplicationContext(); }
    static void detach(Activity a) { if (activity == a) activity = null; }

    // ------------------------------------------------------------ wire

    private static final class Out {
        final ByteArrayOutputStream b = new ByteArrayOutputStream();
        Out u32(long v) { for (int i = 0; i < 4; i++) b.write((int)(v >>> (8 * i))); return this; }
        Out u64(long v) { for (int i = 0; i < 8; i++) b.write((int)(v >>> (8 * i))); return this; }
        Out bytes(byte[] v) { u32(v.length); b.write(v, 0, v.length); return this; }
        Out str(String v) { return bytes((v == null ? "" : v).getBytes(StandardCharsets.UTF_8)); }
        byte[] done() { return b.toByteArray(); }
    }
    private static final class In {
        final ByteBuffer b;
        In(byte[] v) { b = ByteBuffer.wrap(v).order(ByteOrder.LITTLE_ENDIAN); }
        int u32() { return b.getInt(); }
        byte[] bytes() { byte[] v = new byte[b.getInt()]; b.get(v); return v; }
        String str() { return new String(bytes(), StandardCharsets.UTF_8); }
    }

    // ------------------------------------------------------------ synchronous calls

    static byte[] call(int kind, byte[] in) {
        if (kind >= GallerySecrets.CALL_SECRET_AVAILABLE && kind <= GallerySecrets.CALL_SECRET_DELETE)
            return app == null ? null : GallerySecrets.call(app, kind, in);
        if (kind == GalleryAudio.CALL_SOUND) return app == null ? null : GalleryAudio.call(app, in);
        Context context = activity;
        if (context == null) return null;
        try {
            switch (kind) {
                case CALL_QUERY: return query(context, new String(in, StandardCharsets.UTF_8));
                case CALL_CHILDREN: return children(context, new String(in, StandardCharsets.UTF_8));
                case CALL_CREATE: { In r = new In(in); boolean dir = r.u32() != 0; return create(context, r.str(), r.str(), dir); }
                case CALL_DELETE: return new Out().u32(DocumentsContract.deleteDocument(context.getContentResolver(),
                    Uri.parse(new String(in, StandardCharsets.UTF_8))) ? 1 : 0).done();
                case CALL_RENAME: { In r = new In(in); return rename(context, r.str(), r.str()); }
                case CALL_OPEN: { In r = new In(in); int mode = r.u32(); return open(context, r.str(), mode); }
                case CALL_CLIP_GET: return clipGet(context, new In(in).u32());
                case CALL_CLIP_SET: { In r = new In(in); return clipSet(context, r.bytes(), r.bytes(), r.bytes()); }
                case CALL_CLIP_HAS: return new Out().u32(clipHas(context, new In(in).u32()) ? 1 : 0).done();
                case CALL_CLIP_STAMP: return new Out().u64(clipStamp(context)).done();
                case CALL_CLIP_OURS: return new Out().u32(clipOurs(context) ? 1 : 0).done();
                case CALL_GRANTS: return grants(context);
                case CALL_RELEASE: return new Out().u32(release(context, new String(in, StandardCharsets.UTF_8)) ? 1 : 0).done();
                default: return null;
            }
        } catch (Exception | LinkageError e) {
            // missing documents, revoked grants, providers that refuse: an empty answer
            Log.i(TAG, "host call " + kind + ": " + e);
            return null;
        }
    }

    // the document behind a URI (a tree: its root folder)
    static Uri document(Uri u) {
        if (DocumentsContract.isTreeUri(u) && u.getPathSegments().size() == 2)
            return DocumentsContract.buildDocumentUriUsingTree(u, DocumentsContract.getTreeDocumentId(u));
        return u;
    }
    private static final String[] DOC_COLUMNS = {DocumentsContract.Document.COLUMN_DISPLAY_NAME,
        DocumentsContract.Document.COLUMN_MIME_TYPE, DocumentsContract.Document.COLUMN_SIZE,
        DocumentsContract.Document.COLUMN_LAST_MODIFIED, DocumentsContract.Document.COLUMN_FLAGS,
        DocumentsContract.Document.COLUMN_DOCUMENT_ID};
    private static final String[] OPENABLE = {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};

    // DocInfo of the cursor's row: uri, name, dir, size, mtime ms, DOC_*
    private static void info(Out out, Uri uri, Cursor c, boolean document, String type) {
        String name = c.isNull(0) ? uri.getLastPathSegment() : c.getString(0);
        String mime = document ? c.getString(1) : type;
        int sizeColumn = document ? 2 : 1;
        long size = c.isNull(sizeColumn) ? -1 : c.getLong(sizeColumn);
        long mtime = document && !c.isNull(3) ? c.getLong(3) : 0;
        int flags = document ? c.getInt(4) : 0, doc = 0;
        if ((flags & DocumentsContract.Document.FLAG_SUPPORTS_WRITE) != 0) doc |= DOC_WRITE;
        if ((flags & DocumentsContract.Document.FLAG_SUPPORTS_DELETE) != 0) doc |= DOC_DELETE;
        if ((flags & DocumentsContract.Document.FLAG_SUPPORTS_RENAME) != 0) doc |= DOC_RENAME;
        if ((flags & DocumentsContract.Document.FLAG_DIR_SUPPORTS_CREATE) != 0) doc |= DOC_CREATE;
        boolean dir = DocumentsContract.Document.MIME_TYPE_DIR.equals(mime);
        out.str(uri.toString()).str(name).u32(dir ? 1 : 0).u64(dir ? 0 : size).u64(mtime).u32(doc);
    }
    static byte[] query(Context context, String text) {
        Uri u = document(Uri.parse(text));
        ContentResolver resolver = context.getContentResolver();
        boolean document = DocumentsContract.isDocumentUri(context, u);
        try (Cursor c = resolver.query(u, document ? DOC_COLUMNS : OPENABLE, null, null, null)) {
            if (c == null || !c.moveToFirst()) return new byte[0];
            Out out = new Out();
            info(out, u, c, document, document ? null : resolver.getType(u));
            return out.done();
        }
    }
    static byte[] children(Context context, String text) {
        Uri u = Uri.parse(text);
        if (!DocumentsContract.isTreeUri(u)) return null; // only below a folder the user chose
        watch(u);
        u = document(u);
        Uri kids = DocumentsContract.buildChildDocumentsUriUsingTree(u, DocumentsContract.getDocumentId(u));
        try (Cursor c = context.getContentResolver().query(kids, DOC_COLUMNS, null, null, null)) {
            if (c == null) return null;
            Out out = new Out().u32(c.getCount());
            while (c.moveToNext())
                info(out, DocumentsContract.buildDocumentUriUsingTree(u, c.getString(5)), c, true, null);
            return out.done();
        }
    }
    // the providers of the trees the app lists: a change they report (notifyChange) empties
    // plat's name cache of content:// (content.rs; nativeChanged -> jfgui_content_changed)
    private static final HashSet<String> watched = new HashSet<>();
    private static void watch(Uri u) {
        String auth = u.getAuthority();
        if (app == null || auth == null || !watched.add(auth)) return;
        try {
            app.getContentResolver().registerContentObserver(new Uri.Builder().scheme("content").authority(auth).build(), true,
                new ContentObserver(new Handler(Looper.getMainLooper())) {
                    @Override public void onChange(boolean self) { nativeChanged(); }
                });
        } catch (Exception e) {
            Log.i(TAG, "no change reports of " + auth + ": " + e);
        }
    }
    static String mimeOf(String name) {
        int dot = name.lastIndexOf('.');
        String mime = dot < 0 ? null
            : MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substring(dot + 1).toLowerCase(Locale.ROOT));
        return mime != null ? mime : "application/octet-stream";
    }
    static byte[] create(Context context, String parent, String name, boolean dir) throws Exception {
        Uri made = DocumentsContract.createDocument(context.getContentResolver(), document(Uri.parse(parent)),
            dir ? DocumentsContract.Document.MIME_TYPE_DIR : mimeOf(name), name);
        return made == null ? null : query(context, made.toString());
    }
    static byte[] rename(Context context, String uri, String name) throws Exception {
        Uri u = Uri.parse(uri);
        Uri renamed = DocumentsContract.renameDocument(context.getContentResolver(), u, name);
        return query(context, (renamed != null ? renamed : u).toString());
    }
    static byte[] open(Context context, String uri, int mode) throws Exception {
        ContentResolver resolver = context.getContentResolver();
        Uri u = Uri.parse(uri);
        ParcelFileDescriptor fd;
        if (mode == FO_READ) fd = resolver.openFileDescriptor(u, "r");
        else if (mode == FO_RW) fd = resolver.openFileDescriptor(u, "rw");
        else {
            try { fd = resolver.openFileDescriptor(u, "rwt"); }
            catch (IllegalArgumentException | UnsupportedOperationException e) { fd = resolver.openFileDescriptor(u, "wt"); }
        }
        if (fd == null) return null;
        return new Out().u32(fd.detachFd()).done(); // Rust closes it
    }
    static byte[] grants(Context context) {
        List<UriPermission> list = context.getContentResolver().getPersistedUriPermissions();
        Out out = new Out().u32(list.size());
        for (UriPermission p : list) out.str(p.getUri().toString()).u32(p.isReadPermission() ? 1 : 0).u32(p.isWritePermission() ? 1 : 0);
        return out.done();
    }
    static boolean release(Context context, String uri) {
        Uri u = Uri.parse(uri);
        for (UriPermission p : context.getContentResolver().getPersistedUriPermissions()) {
            if (!p.getUri().equals(u)) continue;
            int flags = (p.isReadPermission() ? Intent.FLAG_GRANT_READ_URI_PERMISSION : 0)
                | (p.isWritePermission() ? Intent.FLAG_GRANT_WRITE_URI_PERMISSION : 0);
            context.getContentResolver().releasePersistableUriPermission(u, flags);
            return true;
        }
        return false;
    }

    // ------------------------------------------------------------ the clipboard

    private static ClipboardManager clipboard(Context context) {
        return (ClipboardManager)context.getSystemService(Context.CLIPBOARD_SERVICE);
    }
    static long clipStamp(Context context) {
        ClipDescription d = clipboard(context).getPrimaryClipDescription();
        return d == null ? 0 : d.getTimestamp();
    }
    // the first image of the clip: (uri, its type)
    private static Uri clipImage(Context context, ClipData clip) {
        ContentResolver resolver = context.getContentResolver();
        for (int i = 0; i < clip.getItemCount(); i++) {
            Uri u = clip.getItemAt(i).getUri();
            if (u == null) continue;
            String type = resolver.getType(u);
            if (type != null && type.startsWith("image/")) return u;
        }
        return null;
    }
    // the clip's first item is a file the app shared (GalleryFiles)
    static boolean clipOurs(Context context) {
        ClipData clip = clipboard(context).getPrimaryClip();
        Uri u = clip != null && clip.getItemCount() > 0 ? clip.getItemAt(0).getUri() : null;
        return u != null && GalleryFiles.authority(context).equals(u.getAuthority());
    }
    static boolean clipHas(Context context, int format) {
        ClipboardManager cm = clipboard(context);
        ClipDescription d = cm.getPrimaryClipDescription();
        if (d == null) return false;
        if (format == CLIP_TEXT) return d.hasMimeType("text/*");
        if (format == CLIP_HTML) return d.hasMimeType(ClipDescription.MIMETYPE_TEXT_HTML);
        if (format == CLIP_IMAGE) return d.hasMimeType("image/*");
        return false;
    }
    static byte[] clipGet(Context context, int format) throws Exception {
        ClipData clip = clipboard(context).getPrimaryClip();
        Out out = new Out();
        if (clip == null || clip.getItemCount() == 0) return out.u32(0).done();
        ClipData.Item item = clip.getItemAt(0);
        if (format == CLIP_TEXT) {
            // an image's URI is no text to paste
            if (item.getText() == null && item.getHtmlText() == null && item.getUri() != null && clipImage(context, clip) != null)
                return out.u32(0).done();
            CharSequence text = item.coerceToText(context);
            return text == null ? out.u32(0).done() : out.u32(1).str(text.toString()).done();
        }
        if (format == CLIP_HTML) {
            String html = item.getHtmlText();
            return html == null ? out.u32(0).done() : out.u32(1).str(html).done();
        }
        if (format == CLIP_IMAGE) {
            Uri u = clipImage(context, clip);
            if (u == null) return out.u32(0).done();
            try (InputStream s = context.getContentResolver().openInputStream(u)) {
                ByteArrayOutputStream all = new ByteArrayOutputStream();
                byte[] buf = new byte[65536];
                for (int n; s != null && (n = s.read(buf)) > 0; ) {
                    all.write(buf, 0, n);
                    if (all.size() > CLIP_MAX) return out.u32(0).done();
                }
                return out.u32(1).bytes(all.toByteArray()).done();
            }
        }
        return out.u32(0).done();
    }
    static byte[] clipSet(Context context, byte[] text, byte[] html, byte[] png) throws Exception {
        ClipboardManager cm = clipboard(context);
        ClipData clip;
        if (png.length > 0) {
            Uri u = GalleryFiles.share(context, "clip.png", png);
            if (u == null) return null; // no GalleryFiles provider in the manifest: in the app only
            clip = ClipData.newUri(context.getContentResolver(), "Bild", u);
        } else {
            String t = new String(text, StandardCharsets.UTF_8);
            clip = html.length > 0 ? ClipData.newHtmlText("jfgui", t, new String(html, StandardCharsets.UTF_8))
                : ClipData.newPlainText("jfgui", t);
        }
        cm.setPrimaryClip(clip);
        return new Out().u64(clipStamp(context)).done();
    }

    // ------------------------------------------------------------ the picker

    static boolean request(int kind, int id, byte[] payload) {
        Activity a = activity;
        if (a == null) return false;
        try {
            Intent intent;
            if (kind == REQUEST_FILE_DIALOG) intent = dialog(new In(payload));
            else if (kind == REQUEST_OPEN) intent = openIntent(extensions(new String(payload, StandardCharsets.UTF_8)));
            else if (kind == REQUEST_DOWNLOAD) {
                In r = new In(payload);
                String path = new String(r.bytes(), StandardCharsets.UTF_8); // u32 length + path, the rest the bytes
                intent = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                    .setType(mimeOf(path)).putExtra(Intent.EXTRA_TITLE, path.substring(path.lastIndexOf('/') + 1));
            } else return false;
            int code = REQUEST_BASE + (nextCode++ & 0xFF);
            pending.put(code, new Object[] {id, kind, payload});
            a.startActivityForResult(intent, code);
            return true;
        } catch (Exception e) {
            Log.w(TAG, "picker: " + e);
            return false;
        }
    }
    private static List<String> extensions(String filter) {
        List<String> out = new ArrayList<>();
        for (String e : filter.split("[,;\\s]+")) if (!e.isEmpty()) out.add(e.startsWith(".") ? e.substring(1) : e);
        return out;
    }
    private static Intent openIntent(List<String> exts) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE);
        List<String> types = new ArrayList<>();
        for (String e : exts) {
            String m = MimeTypeMap.getSingleton().getMimeTypeFromExtension(e.toLowerCase(Locale.ROOT));
            if (m == null) { types.clear(); break; } // an unknown type: every document
            if (!types.contains(m)) types.add(m);
        }
        if (types.size() == 1) intent.setType(types.get(0));
        else {
            intent.setType("*/*");
            if (!types.isEmpty()) intent.putExtra(Intent.EXTRA_MIME_TYPES, types.toArray(new String[0]));
        }
        return intent;
    }
    // file_dialog: u32 flags, the suggested name, the URI to start at, u32 count, extensions
    private static Intent dialog(In r) {
        int flags = r.u32();
        String name = r.str(), start = r.str();
        List<String> exts = new ArrayList<>();
        for (int n = r.u32(); n > 0; n--) exts.add(r.str());
        Intent intent;
        if ((flags & FD_FOLDER) != 0) {
            intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        } else if ((flags & FD_SAVE) != 0) {
            if (name.isEmpty()) name = "Dokument";
            if (name.indexOf('.') < 0 && !exts.isEmpty()) name += "." + exts.get(0);
            intent = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                .setType(mimeOf(name)).putExtra(Intent.EXTRA_TITLE, name);
        } else {
            intent = openIntent(exts);
        }
        if (start.startsWith("content:")) intent.putExtra(DocumentsContract.EXTRA_INITIAL_URI, Uri.parse(start));
        return intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
            | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
    }

    /** GalleryActivity.onActivityResult: true when it was one of ours. */
    static boolean onResult(Activity a, int code, int result, Intent data) {
        Object[] p = pending.get(code);
        if (p == null) return false;
        pending.remove(code);
        int id = (Integer)p[0], kind = (Integer)p[1];
        Uri uri = result == Activity.RESULT_OK && data != null ? data.getData() : null;
        if (uri == null) {
            nativeResult(id, CANCELLED, null);
            return true;
        }
        try {
            if (kind == REQUEST_DOWNLOAD) {
                byte[] payload = (byte[])p[2];
                int n = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN).getInt();
                try (OutputStream out = a.getContentResolver().openOutputStream(uri, "wt")) {
                    out.write(payload, 4 + n, payload.length - 4 - n);
                }
                nativeResult(id, EXPORTED, null);
                return true;
            }
            // the user's document stays reachable after a restart (what the picker granted)
            int grant = data.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            try { if (grant != 0) a.getContentResolver().takePersistableUriPermission(uri, grant); }
            catch (SecurityException e) { Log.i(TAG, "no lasting grant for " + uri + ": " + e); }
            nativeResult(id, DONE, uri.toString().getBytes(StandardCharsets.UTF_8));
        } catch (Exception e) {
            Log.w(TAG, "picker result: " + e);
            nativeResult(id, FAILED, null);
        }
        return true;
    }

    /** A file of the app's cache shared through GalleryFiles (images on the clipboard). */
    static File shareDir(Context context) {
        File d = new File(context.getCacheDir(), "jfgui-share");
        if (!d.isDirectory() && !d.mkdirs()) return null;
        return d;
    }
    static boolean write(File f, byte[] bytes) {
        try (FileOutputStream out = new FileOutputStream(f)) { out.write(bytes); return true; }
        catch (Exception e) { return false; }
    }
}
