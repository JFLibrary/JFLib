/* The secret store on a device or emulator (lib/plat/android_secret.rs, android/GallerySecrets.java),
 * run by apps/androidGallery/tests/android_secrets.py (it clears the app's data before phase 1 and
 * stops the app between the phases):
 *   adb shell am instrument -w -r -e phase 1 org.jflib.gallery/.GallerySecretsInstrumentation
 * Phase 1 starts the gallery's self-test (jfgui_SELFTEST=secrets1, apps/androidGallery/secrets.rs:
 * set, get, overwrite, delete, names, another app id, values up to SECRET_MAX), then looks from
 * outside: the Keystore key (AES, 256 bits, GCM, not exportable, no user authentication), the
 * three files the self-test left below no_backup/jfgui-secrets (none holds its value in plain,
 * nor does any other file of the app), and damages the file of the entry "tamper".  Phase 2 (a
 * new process): the values survived the restart, the damaged one is refused, then everything is
 * deleted and no file is left. */
package org.jflib.gallery;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.security.keystore.KeyInfo;
import android.security.keystore.KeyProperties;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.RandomAccessFile;
import java.nio.charset.StandardCharsets;
import java.security.Key;
import java.security.KeyStore;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import javax.crypto.SecretKey;
import javax.crypto.SecretKeyFactory;

public final class GallerySecretsInstrumentation extends Instrumentation {
    static final String APP = "androidGallery", SVC = "selbsttest", KEPT = "überlebt den Neustart ✓";
    private String phase;
    private int checks;
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
    private static byte[] bytes(File f) {
        if (!f.isFile()) return null;
        try (InputStream in = new FileInputStream(f)) {
            byte[] b = new byte[(int)f.length()];
            int n = 0;
            while (n < b.length) { int k = in.read(b, n, b.length - n); if (k < 0) break; n += k; }
            return Arrays.copyOf(b, n);
        } catch (Exception e) { return null; }
    }
    // secrets.rs: big(seed, n)
    static byte[] big(int seed, int n) {
        byte[] b = new byte[n];
        for (int i = 0; i < n; i++) b[i] = (byte)((i * (int)2654435761L + seed) >>> 13);
        return b;
    }
    private static boolean contains(byte[] hay, byte[] needle) {
        outer:
        for (int i = 0; hay != null && i + needle.length <= hay.length; i++) {
            for (int j = 0; j < needle.length; j++) if (hay[i + j] != needle[j]) continue outer;
            return true;
        }
        return false;
    }
    private static void files(File dir, List<File> out) {
        File[] list = dir.listFiles();
        if (list == null) return;
        for (File f : list) { if (f.isDirectory()) files(f, out); else out.add(f); }
    }
    private static List<File> secretFiles(Context c) {
        List<File> out = new ArrayList<>();
        File[] list = new File(c.getNoBackupFilesDir(), GallerySecrets.DIR).listFiles();
        if (list != null) out.addAll(Arrays.asList(list));
        return out;
    }

    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            Context target = getTargetContext();
            File results = new File(target.getFilesDir(), "fs/data/" + APP + "/secrets-selftest.txt");
            if (results.exists() && !results.delete()) throw new AssertionError("old results stay");
            Intent intent = new Intent(target, GalleryActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                .putExtra("jfgui_SELFTEST", "secrets" + phase);
            startActivitySync(intent);
            waitForIdleSync();
            long end = SystemClock.uptimeMillis() + 60000;
            String text;
            while ((text = results.isFile() ? new String(bytes(results), StandardCharsets.UTF_8) : null) == null || !text.contains("done ")) {
                if (SystemClock.uptimeMillis() > end) throw new AssertionError("self-test timed out:\n" + text);
                SystemClock.sleep(200);
            }
            int ok = 0;
            StringBuilder fails = new StringBuilder();
            for (String line : text.split("\n")) {
                if (line.startsWith("ok ")) ok++;
                else if (line.startsWith("FAIL")) fails.append(line).append('\n');
            }
            check(fails.length() == 0, "self-test:\n" + fails);
            check(ok >= (phase.equals("1") ? 33 : 9), "self-test ran " + ok + " checks");
            result.putInt("app_checks", ok);
            if (phase.equals("1")) outside(target); else check(secretFiles(target).isEmpty(), "no file left: " + secretFiles(target));
            result.putString("stream", "PASS: phase " + phase + ": " + ok + " self-test checks, " + checks + " checks\n" + log);
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", "FAIL: phase " + phase + ": " + error + "\n" + log);
            finish(Activity.RESULT_CANCELED, result);
        }
    }

    // the key and the files as another component of the app sees them
    private void outside(Context target) throws Exception {
        KeyStore ks = KeyStore.getInstance("AndroidKeyStore");
        ks.load(null);
        Key key = ks.getKey(GallerySecrets.ALIAS, null);
        check(key instanceof SecretKey, "the app's key is in the AndroidKeyStore");
        check(key.getEncoded() == null, "the key is not exportable");
        KeyInfo info = (KeyInfo)SecretKeyFactory.getInstance(key.getAlgorithm(), "AndroidKeyStore").getKeySpec((SecretKey)key, KeyInfo.class);
        check(KeyProperties.KEY_ALGORITHM_AES.equals(key.getAlgorithm()) && info.getKeySize() == 256, "AES, 256 bits: " + key.getAlgorithm() + " " + info.getKeySize());
        check(Arrays.asList(info.getBlockModes()).contains(KeyProperties.BLOCK_MODE_GCM), "GCM");
        check(info.getPurposes() == (KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT), "encrypt and decrypt only");
        check(!info.isUserAuthenticationRequired(), "no user authentication");
        @SuppressWarnings("deprecation") boolean hw = info.isInsideSecureHardware();
        log.append("key: inside secure hardware ").append(hw);
        if (Build.VERSION.SDK_INT >= 31) log.append(", security level ").append(info.getSecurityLevel());
        log.append('\n');

        List<File> files = secretFiles(target);
        log.append("files: ");
        for (File f : files) log.append(f.getName()).append(' ').append(f.length()).append(" B  ");
        log.append('\n');
        check(files.size() == 3, "three files below no_backup/jfgui-secrets: " + files);
        File kept = GallerySecrets.file(target, GallerySecrets.name(APP, SVC, "kept"));
        File tamper = GallerySecrets.file(target, GallerySecrets.name(APP, SVC, "tamper"));
        check(kept.isFile() && tamper.isFile() && GallerySecrets.file(target, GallerySecrets.name(APP, SVC, "kept-big")).isFile(),
            "named by the SHA-256 of app, service, account");
        byte[] plain = KEPT.getBytes(StandardCharsets.UTF_8), large = Arrays.copyOf(big(7, 5000), 64);
        for (File f : files) {
            byte[] b = bytes(f);
            check(b != null && b.length > 4 + 12 + 16 && b[0] == 'J' && b[1] == 'F' && b[2] == 'S' && b[3] == '1', "JFS1 file " + f.getName());
            check(!contains(b, plain) && !contains(b, large), "no value in plain in " + f.getName());
        }
        // nowhere else either: files, cache, no_backup
        List<File> all = new ArrayList<>();
        files(target.getFilesDir(), all);
        files(target.getCacheDir(), all);
        files(target.getNoBackupFilesDir(), all);
        for (File f : all) check(!contains(bytes(f), plain), "the kept value in plain in " + f);
        log.append("searched ").append(all.size()).append(" files of the app for the value\n");
        // damage "tamper": its last byte (the tag)
        try (RandomAccessFile r = new RandomAccessFile(tamper, "rw")) {
            r.seek(r.length() - 1);
            int x = r.read();
            r.seek(r.length() - 1);
            r.write(x ^ 1);
        }
        checks++;
    }
}
