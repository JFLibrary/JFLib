/* System notifications for plat::tray::notify (lib/plat/web_notify.rs): CALL_NOTIFY over
 * GalleryHost.call (u32 op 1 show / 2 close, u32 id; show: title, body as u32 length + UTF-8,
 * u32 w, u32 h, w * h 0xAARRGGBB words) -> one byte, 1 shown (or waiting for the permission) /
 * 0 not possible.  NotificationManager with the channel "jfgui"; on API 33+ POST_NOTIFICATIONS
 * is asked for at the first note (the system's dialog over the activity; the notes wait for the
 * answer, refused they are gone).  A tap opens the activity (a PendingIntent with SINGLE_TOP |
 * CLEAR_TOP: onNewIntent of the running one, else onCreate) and reports the id as clicked,
 * swiping it away (the delete intent to a receiver of the app's own) as gone: nativeNote
 * (system.c -> jfgui_note), then nativeEvent(E_WAKE) so the app hears it at once.  The intents
 * carry this process's session: a tap on a note of an earlier process only opens the app. */
package org.jflib.gallery;

import android.Manifest;
import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.os.Build;
import android.util.Log;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;

final class GalleryNotify {
    static final int CALL_NOTIFY = 50, OP_SHOW = 1, OP_CLOSE = 2;
    static final String CHANNEL = "jfgui", TAG = "jfgui-note";
    static final String EXTRA_ID = "org.jflib.gallery.jfgui.NOTE", EXTRA_SESSION = "org.jflib.gallery.jfgui.SESSION";
    private static final String LOG = "jfgui";
    private static final int PERMISSION_CODE = 0x4B00;
    // this process (a note of an earlier one is not this app's id any more)
    private static final long SESSION = System.nanoTime() ^ (long) android.os.Process.myPid() << 40;

    static native void nativeNote(int id, boolean clicked);

    private static Activity activity;
    // the notes waiting for the permission: {Integer id, Notification}
    private static final ArrayList<Object[]> waiting = new ArrayList<>();
    private static boolean asking, receiving, channel;

    static void attach(Activity a) {
        activity = a;
        intent(a.getIntent()); // started from a note while the process lived
    }
    static void detach(Activity a) { if (activity == a) activity = null; }

    private static String action(Context app) { return app.getPackageName() + ".jfgui.NOTE_GONE"; }
    private static void pump() { GalleryActivity.nativeEvent(GallerySystem.E_WAKE, 0, 0, 0, 0, 0); }

    static byte[] call(Context app, Activity a, int kind, byte[] in) {
        if (kind != CALL_NOTIFY || app == null || in == null || in.length < 8) return null;
        try {
            ByteBuffer b = ByteBuffer.wrap(in).order(ByteOrder.LITTLE_ENDIAN);
            int op = b.getInt(), id = b.getInt();
            NotificationManager nm = (NotificationManager) app.getSystemService(Context.NOTIFICATION_SERVICE);
            if (nm == null) return new byte[] {0};
            if (op == OP_CLOSE) {
                nm.cancel(TAG, id);
                for (int i = waiting.size() - 1; i >= 0; i--) if ((Integer) waiting.get(i)[0] == id) waiting.remove(i);
                return new byte[] {1};
            }
            String title = str(b), body = str(b);
            int w = b.getInt(), h = b.getInt();
            Bitmap icon = null;
            if (w > 0 && h > 0 && w <= 256 && h <= 256) {
                int[] px = new int[w * h];
                b.asIntBuffer().get(px);
                icon = Bitmap.createBitmap(px, w, h, Bitmap.Config.ARGB_8888);
            }
            Notification n = build(app, nm, id, title, body, icon);
            if (Build.VERSION.SDK_INT >= 33
                    && app.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                if (a == null) return new byte[] {0};
                waiting.add(new Object[] {id, n});
                if (!asking) {
                    asking = true;
                    a.requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS}, PERMISSION_CODE);
                }
                return new byte[] {1};
            }
            if (!nm.areNotificationsEnabled()) return new byte[] {0};
            nm.notify(TAG, id, n);
            return new byte[] {1};
        } catch (RuntimeException e) {
            Log.w(LOG, "notification: " + e);
            return new byte[] {0};
        }
    }
    private static String str(ByteBuffer b) {
        byte[] v = new byte[b.getInt()];
        b.get(v);
        return new String(v, StandardCharsets.UTF_8);
    }
    private static Notification build(Context app, NotificationManager nm, int id, String title, String body, Bitmap icon) {
        if (!channel) {
            channel = true;
            CharSequence label = app.getApplicationInfo().loadLabel(app.getPackageManager());
            nm.createNotificationChannel(new NotificationChannel(CHANNEL, label, NotificationManager.IMPORTANCE_DEFAULT));
        }
        receive(app);
        int flags = PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT;
        Intent open = new Intent(app, GalleryActivity.class)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .putExtra(EXTRA_ID, id).putExtra(EXTRA_SESSION, SESSION);
        Intent gone = new Intent(action(app)).setPackage(app.getPackageName()).putExtra(EXTRA_ID, id).putExtra(EXTRA_SESSION, SESSION);
        int small = app.getApplicationInfo().icon;
        Notification.Builder nb = new Notification.Builder(app, CHANNEL)
            .setSmallIcon(small != 0 ? small : android.R.drawable.ic_dialog_info)
            .setContentTitle(title).setContentText(body)
            .setStyle(new Notification.BigTextStyle().bigText(body))
            .setAutoCancel(true)
            .setContentIntent(PendingIntent.getActivity(app, id, open, flags))
            .setDeleteIntent(PendingIntent.getBroadcast(app, id, gone, flags));
        if (icon != null) nb.setLargeIcon(icon);
        return nb.build();
    }
    // the delete intents: a receiver of the app's own (not exported)
    private static void receive(Context app) {
        if (receiving) return;
        receiving = true;
        BroadcastReceiver r = new BroadcastReceiver() {
            @Override public void onReceive(Context c, Intent i) {
                if (i.getLongExtra(EXTRA_SESSION, 0) != SESSION) return;
                int id = i.getIntExtra(EXTRA_ID, 0);
                if (id == 0) return;
                nativeNote(id, false);
                pump();
            }
        };
        IntentFilter f = new IntentFilter(action(app));
        if (Build.VERSION.SDK_INT >= 33) app.registerReceiver(r, f, Context.RECEIVER_NOT_EXPORTED);
        else app.registerReceiver(r, f);
    }

    /** GalleryActivity.onRequestPermissionsResult -> true when it was ours. */
    static boolean permissions(int code, int[] results) {
        if (code != PERMISSION_CODE) return false;
        asking = false;
        Activity a = activity;
        Context app = a != null ? a.getApplicationContext() : null;
        NotificationManager nm = app != null ? (NotificationManager) app.getSystemService(Context.NOTIFICATION_SERVICE) : null;
        boolean granted = results != null && results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED
            && nm != null && nm.areNotificationsEnabled();
        for (Object[] w : waiting) {
            int id = (Integer) w[0];
            if (granted) nm.notify(TAG, id, (Notification) w[1]);
            else nativeNote(id, false);
        }
        waiting.clear();
        pump();
        return true;
    }

    /** GalleryActivity.onNewIntent / onCreate: a tap on one of this process's notes. */
    static void intent(Intent i) {
        if (i == null || !i.hasExtra(EXTRA_ID)) return;
        int id = i.getIntExtra(EXTRA_ID, 0);
        long session = i.getLongExtra(EXTRA_SESSION, 0);
        i.removeExtra(EXTRA_ID); // once (the activity keeps its intent)
        if (session != SESSION || id == 0) return;
        nativeNote(id, true);
        pump();
    }
}
