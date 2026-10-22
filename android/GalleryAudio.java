/* The system's sounds for plat::audio::sound (lib/plat/android_audio.rs; the streams themselves
 * run on AAudio, android/audio.c): call(u32 kind) plays the default notification sound
 * (SOUND_NOTIFY) or a ToneGenerator beep on STREAM_SYSTEM (the others: it follows the system's
 * volume and mute) and answers one byte, 1 played / 0 not. */
package org.jflib.gallery;

import android.content.Context;
import android.media.AudioManager;
import android.media.Ringtone;
import android.media.RingtoneManager;
import android.media.ToneGenerator;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

final class GalleryAudio {
    static final int CALL_SOUND = 40;
    // plat::audio::SOUND_*
    static final int SOUND_DEFAULT = 0, SOUND_INFO = 1, SOUND_WARNING = 2, SOUND_ERROR = 3, SOUND_QUESTION = 4, SOUND_NOTIFY = 5;
    private static final Handler main = new Handler(Looper.getMainLooper());
    // the notification playing now (kept: a collected Ringtone may stop)
    private static Ringtone ringing;

    static byte[] call(Context context, byte[] in) {
        int kind = in != null && in.length >= 4 ? (in[0] & 255) | (in[1] & 255) << 8 | (in[2] & 255) << 16 | (in[3] & 255) << 24 : -1;
        boolean ok;
        try {
            ok = play(context, kind);
        } catch (RuntimeException e) {
            Log.i("jfgui", "sound " + kind + ": " + e);
            ok = false;
        }
        return new byte[] {(byte) (ok ? 1 : 0)};
    }

    private static boolean play(Context context, int kind) {
        if (kind == SOUND_NOTIFY) {
            Ringtone r = RingtoneManager.getRingtone(context, RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION));
            if (r == null) return false;
            if (ringing != null) ringing.stop();
            ringing = r;
            r.play();
            return true;
        }
        int tone, ms;
        switch (kind) {
            case SOUND_INFO: tone = ToneGenerator.TONE_PROP_BEEP; ms = 150; break;
            case SOUND_WARNING: tone = ToneGenerator.TONE_PROP_BEEP2; ms = 300; break;
            case SOUND_ERROR: tone = ToneGenerator.TONE_PROP_NACK; ms = 300; break;
            case SOUND_QUESTION: tone = ToneGenerator.TONE_PROP_PROMPT; ms = 300; break;
            case SOUND_DEFAULT: tone = ToneGenerator.TONE_PROP_BEEP; ms = 150; break;
            default: return false;
        }
        final ToneGenerator g = new ToneGenerator(AudioManager.STREAM_SYSTEM, 80);
        boolean started = g.startTone(tone, ms);
        main.postDelayed(g::release, ms + 200);
        return started;
    }
}
