/* plat::secret on Android (lib/plat/android_secret.rs), asked through GalleryHost.call: one
 * AES-256-GCM key of the Android Keystore per app (alias jfgui-secrets, made on the first set;
 * not exportable, no user authentication, usable while the device is locked), each value
 * encrypted with it into noBackupFilesDir/jfgui-secrets/<SHA-256 of "app\0service\0account",
 * hex>.bin: "JFS1", the 12-byte IV the Keystore chose, the ciphertext with its 16-byte tag.  The
 * name is the additional data: a file copied to another entry's name does not decrypt.
 * Answers: u32 status (0 done, 1 missing, 2 failed), then u32 length + the value of get or the
 * reason of a failure. */
package org.jflib.gallery;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;
import java.io.File;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.Key;
import java.security.KeyStore;
import java.security.MessageDigest;
import java.util.Arrays;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

final class GallerySecrets {
    // android_host.rs
    static final int CALL_SECRET_AVAILABLE = 30, CALL_SECRET_GET = 31, CALL_SECRET_SET = 32, CALL_SECRET_DELETE = 33;
    static final int DONE = 0, MISSING = 1, FAILED = 2;
    static final String ALIAS = "jfgui-secrets", DIR = "jfgui-secrets";
    private static final byte[] MAGIC = {'J', 'F', 'S', '1'};
    private static final int IV = 12, TAG = 16;
    private static KeyStore keystore;

    private GallerySecrets() {}

    static synchronized byte[] call(Context context, int kind, byte[] in) {
        try {
            if (kind == CALL_SECRET_AVAILABLE) {
                keystore();
                return answer(DONE, new byte[0]);
            }
            ByteBuffer b = ByteBuffer.wrap(in).order(ByteOrder.LITTLE_ENDIAN);
            byte[] name = name(text(b), text(b), text(b));
            File f = file(context, name);
            if (kind == CALL_SECRET_GET) return get(f, name);
            if (kind == CALL_SECRET_SET) {
                byte[] value = new byte[b.getInt()];
                b.get(value);
                try { return set(f, name, value); } finally { Arrays.fill(value, (byte)0); }
            }
            if (kind == CALL_SECRET_DELETE) {
                boolean had = f.exists();
                new AtomicFile(f).delete();
                if (f.exists()) return failed("the file was not deleted");
                return answer(had ? DONE : MISSING, new byte[0]);
            }
            return null;
        } catch (Exception | LinkageError e) {
            return failed(e.getClass().getSimpleName() + (e.getMessage() != null ? ": " + e.getMessage() : ""));
        } finally {
            Arrays.fill(in, (byte)0);
        }
    }

    private static String text(ByteBuffer b) {
        byte[] v = new byte[b.getInt()];
        b.get(v);
        return new String(v, StandardCharsets.UTF_8);
    }
    private static byte[] answer(int status, byte[] body) {
        return ByteBuffer.allocate(8 + body.length).order(ByteOrder.LITTLE_ENDIAN).putInt(status).putInt(body.length).put(body).array();
    }
    private static byte[] failed(String why) {
        return answer(FAILED, why.getBytes(StandardCharsets.UTF_8));
    }

    static KeyStore keystore() throws Exception {
        if (keystore == null) {
            KeyStore k = KeyStore.getInstance("AndroidKeyStore");
            k.load(null);
            keystore = k;
        }
        return keystore;
    }
    /** The app's key; null when there is none and create is false. */
    static SecretKey key(boolean create) throws Exception {
        Key k = keystore().getKey(ALIAS, null);
        if (k instanceof SecretKey) return (SecretKey)k;
        if (!create) return null;
        KeyGenerator g = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        g.init(new KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256).build());
        return g.generateKey();
    }
    static byte[] name(String app, String service, String account) {
        return (app + "\0" + service + "\0" + account).getBytes(StandardCharsets.UTF_8);
    }
    /** The entry's file: <noBackupFilesDir>/jfgui-secrets/<SHA-256 of the name>.bin */
    static File file(Context context, byte[] name) throws Exception {
        byte[] h = MessageDigest.getInstance("SHA-256").digest(name);
        StringBuilder s = new StringBuilder(64);
        for (byte x : h) s.append(Character.forDigit((x >> 4) & 15, 16)).append(Character.forDigit(x & 15, 16));
        return new File(new File(context.getNoBackupFilesDir(), DIR), s + ".bin");
    }

    private static byte[] get(File f, byte[] name) throws Exception {
        byte[] blob;
        try { blob = new AtomicFile(f).readFully(); }
        catch (FileNotFoundException e) { return answer(MISSING, new byte[0]); }
        SecretKey key = key(false);
        if (key == null) return failed("the app's key is gone from the Keystore");
        if (blob.length < MAGIC.length + IV + TAG || !Arrays.equals(Arrays.copyOf(blob, MAGIC.length), MAGIC))
            return failed("the stored value is damaged");
        Cipher c = Cipher.getInstance("AES/GCM/NoPadding");
        c.init(Cipher.DECRYPT_MODE, key, new GCMParameterSpec(TAG * 8, blob, MAGIC.length, IV));
        c.updateAAD(name);
        byte[] value;
        try { value = c.doFinal(blob, MAGIC.length + IV, blob.length - MAGIC.length - IV); }
        catch (GeneralSecurityException e) { return failed("the stored value does not decrypt (" + e.getClass().getSimpleName() + ")"); }
        try { return answer(DONE, value); } finally { Arrays.fill(value, (byte)0); }
    }

    private static byte[] set(File f, byte[] name, byte[] value) throws Exception {
        SecretKey key = key(true);
        Cipher c = Cipher.getInstance("AES/GCM/NoPadding");
        c.init(Cipher.ENCRYPT_MODE, key); // the Keystore picks a random IV
        c.updateAAD(name);
        byte[] sealed = c.doFinal(value);
        byte[] iv = c.getIV();
        if (iv == null || iv.length != IV) return failed("the Keystore chose an IV of an unexpected size");
        File dir = f.getParentFile();
        if (!dir.isDirectory() && !dir.mkdirs()) return failed("no folder " + dir);
        AtomicFile file = new AtomicFile(f);
        FileOutputStream out = file.startWrite();
        try {
            out.write(MAGIC);
            out.write(iv);
            out.write(sealed);
            file.finishWrite(out); // fsync, then rename over the old file
        } catch (Exception e) {
            file.failWrite(out);
            throw e;
        }
        return answer(DONE, new byte[0]);
    }
}
