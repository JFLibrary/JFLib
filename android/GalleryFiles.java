/* Files the app hands to other apps: an image on the clipboard (GalleryHost.clipSet).
 * Declared in the app's manifest with the authority PACKAGE.jfgui.files, not exported, with
 * grantUriPermissions -- the clipboard grants the app that pastes read access to the one file.
 * Without the declaration images stay inside the app. */
package org.jflib.gallery;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileNotFoundException;
import java.util.UUID;

public final class GalleryFiles extends ContentProvider {
    static String authority(Context context) { return context.getPackageName() + ".jfgui.files"; }

    /** bytes as a fresh file named after name (clip.png -> clip-RANDOM.png) -> its content:// URI,
     *  or null when the manifest declares no provider.  A new file each time, never the old one
     *  rewritten: a descriptor another app kept of an earlier copy reads that copy, not what the user
     *  copies later.  The earlier copies are deleted. */
    static Uri share(Context context, String name, byte[] bytes) {
        if (context.getPackageManager().resolveContentProvider(authority(context), 0) == null) return null;
        File dir = GalleryHost.shareDir(context);
        if (dir == null) return null;
        int dot = name.lastIndexOf('.');
        String stem = (dot > 0 ? name.substring(0, dot) : name) + "-", ext = dot > 0 ? name.substring(dot) : "";
        String fresh = stem + UUID.randomUUID() + ext;
        File f = new File(dir, fresh);
        if (!GalleryHost.write(f, bytes)) {
            f.delete();
            return null;
        }
        File[] old = dir.listFiles();
        for (File o : old != null ? old : new File[0]) {
            String n = o.getName();
            if (!n.equals(fresh) && (n.equals(name) || n.startsWith(stem) && n.endsWith(ext))) o.delete();
        }
        return new Uri.Builder().scheme("content").authority(authority(context)).appendPath(fresh)
            .appendQueryParameter("t", Long.toString(System.currentTimeMillis())).build();
    }

    private File file(Uri uri) throws FileNotFoundException {
        String name = uri.getLastPathSegment();
        File dir = GalleryHost.shareDir(getContext());
        if (name == null || dir == null || name.contains("/") || name.startsWith(".")) throw new FileNotFoundException();
        File f = new File(dir, name);
        if (!f.isFile()) throw new FileNotFoundException(name);
        return f;
    }
    @Override public boolean onCreate() { return true; }
    @Override public String getType(Uri uri) {
        String name = uri.getLastPathSegment();
        return name != null ? GalleryHost.mimeOf(name) : null;
    }
    @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) throw new FileNotFoundException("read only");
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }
    @Override public Cursor query(Uri uri, String[] projection, String selection, String[] args, String order) {
        File f;
        try { f = file(uri); } catch (FileNotFoundException e) { return null; }
        String[] cols = projection != null ? projection : new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        MatrixCursor c = new MatrixCursor(cols, 1);
        Object[] row = new Object[cols.length];
        for (int i = 0; i < cols.length; i++) {
            if (OpenableColumns.DISPLAY_NAME.equals(cols[i])) row[i] = f.getName();
            else if (OpenableColumns.SIZE.equals(cols[i])) row[i] = f.length();
        }
        c.addRow(row);
        return c;
    }
    @Override public Uri insert(Uri uri, ContentValues values) { return null; }
    @Override public int delete(Uri uri, String selection, String[] args) { return 0; }
    @Override public int update(Uri uri, ContentValues values, String selection, String[] args) { return 0; }
}
