/* The motion sensors and the location for plat (lib/plat/sensors.rs, web_sensors.rs):
 * - CALL_SENSORS: u32 SENSOR_* union -> u32 the bits this device has; SensorManager's accelerometer,
 *   gravity, gyroscope, magnetic field and rotation vector (else the game rotation vector, without
 *   north) at about 30 Hz on the UI thread, every reading through nativeSensor (system.c ->
 *   jfgui_sensor), at most one E_WAKE and frame per display frame;
 * - CALL_LOCATION: u32 0 stop, 1 start, 2 start again after the background -> u32 LocationState.
 *   LocationManager's GPS and network providers (no Google Play services), a fix a second at
 *   most through nativeLocation, the last known fix first; a network fix does not replace a better
 *   GPS fix of the last ten seconds.  Op 1 asks ACCESS_FINE_LOCATION / ACCESS_COARSE_LOCATION
 *   (GalleryActivity.onRequestPermissionsResult; "approximate" gives the network provider only),
 *   op 2 asks nobody: a permission taken back shows as Denied.  The system's location switch:
 *   Disabled, followed through the providers' enabled / disabled calls.
 * Rust stops everything in the background (runtime.rs: E_POWER -> plat::sensors::suspend).
 * All of it on the UI thread, like the library. */
package org.jflib.gallery;

import android.Manifest;
import android.app.Activity;
import android.content.Context;
import android.content.pm.PackageManager;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.List;

final class GallerySensors {
    // android_host.rs: the calls of web_sensors.rs
    static final int CALL_SENSORS = 53, CALL_LOCATION = 54;
    // plat::sensors::SENSOR_*
    private static final int ACCEL = 1, GRAVITY = 2, GYRO = 4, MAGNET = 8, ROTATION = 16;
    // plat::sensors::LocationState
    private static final int SEARCHING = 1, FIX = 2, DENIED = 4, DISABLED = 5, UNAVAILABLE = 6;
    private static final int OP_STOP = 0, OP_START = 1;
    private static final int PERMISSION_CODE = 0x4A47; // not GalleryNotify's
    private static final int PERIOD_US = 33000, FIX_MS = 1000;
    private static final long GPS_WINDOW_MS = 10000;
    private static final String TAG = "jfgui";

    static native void nativeSensor(int bit, float x, float y, float z, float w, int flags);
    static native void nativeLocation(int state, double lat, double lon, double accuracy, double altitude,
                                      double speed, double bearing, double ageMs, int source);

    private static Activity activity;
    private static Context app;
    private static final Handler main = new Handler(Looper.getMainLooper());
    private static boolean pumpPending;
    private static int running;

    static void attach(Activity a) { activity = a; app = a.getApplicationContext(); }
    static void detach(Activity a) { if (activity == a) activity = null; }

    // E_WAKE and one frame for what arrived in this display frame
    private static void pump() {
        if (pumpPending) return;
        pumpPending = true;
        main.postDelayed(() -> {
            pumpPending = false;
            GalleryActivity.nativeEvent(GallerySystem.E_WAKE, 0, 0, 0, 0, 0);
            GalleryActivity.frameNow();
        }, 16);
    }

    static byte[] call(int kind, byte[] in) {
        if (app == null || in == null || in.length < 4) return null;
        int arg = ByteBuffer.wrap(in).order(ByteOrder.LITTLE_ENDIAN).getInt();
        int answer;
        try {
            answer = kind == CALL_SENSORS ? sensors(arg) : location(arg);
        } catch (RuntimeException e) {
            Log.w(TAG, "sensors: " + e);
            return null;
        }
        return ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(answer).array();
    }

    // ------------------------------------------------------------ sensors

    private static final SensorEventListener listener = new SensorEventListener() {
        @Override public void onSensorChanged(SensorEvent e) {
            int bit = bit(e.sensor.getType());
            if (bit == 0 || e.values.length < 3) return;
            float[] v = e.values;
            int accuracy = e.accuracy < 0 ? 0 : Math.min(e.accuracy, 3);
            if (bit == ROTATION) {
                // the fourth value is optional on old systems
                float w = v.length >= 4 ? v[3] : (float) Math.sqrt(Math.max(0, 1 - v[0] * v[0] - v[1] * v[1] - v[2] * v[2]));
                boolean north = e.sensor.getType() == Sensor.TYPE_ROTATION_VECTOR;
                nativeSensor(bit, v[0], v[1], v[2], w, 3 | (north ? 4 : 0));
            } else {
                nativeSensor(bit, v[0], v[1], v[2], 0, accuracy);
            }
            pump();
        }
        @Override public void onAccuracyChanged(Sensor sensor, int accuracy) {}
    };

    private static int bit(int type) {
        switch (type) {
            case Sensor.TYPE_ACCELEROMETER: return ACCEL;
            case Sensor.TYPE_GRAVITY: return GRAVITY;
            case Sensor.TYPE_GYROSCOPE: return GYRO;
            case Sensor.TYPE_MAGNETIC_FIELD: return MAGNET;
            case Sensor.TYPE_ROTATION_VECTOR:
            case Sensor.TYPE_GAME_ROTATION_VECTOR: return ROTATION;
            default: return 0;
        }
    }

    private static Sensor sensor(SensorManager sm, int bit) {
        switch (bit) {
            case ACCEL: return sm.getDefaultSensor(Sensor.TYPE_ACCELEROMETER);
            case GRAVITY: return sm.getDefaultSensor(Sensor.TYPE_GRAVITY);
            case GYRO: return sm.getDefaultSensor(Sensor.TYPE_GYROSCOPE);
            case MAGNET: return sm.getDefaultSensor(Sensor.TYPE_MAGNETIC_FIELD);
            default: {
                Sensor s = sm.getDefaultSensor(Sensor.TYPE_ROTATION_VECTOR);
                return s != null ? s : sm.getDefaultSensor(Sensor.TYPE_GAME_ROTATION_VECTOR);
            }
        }
    }

    // the union runs -> the bits this device has
    private static int sensors(int mask) {
        SensorManager sm = (SensorManager) app.getSystemService(Context.SENSOR_SERVICE);
        if (sm == null) return 0;
        int have = 0;
        for (int bit = ACCEL; bit <= ROTATION; bit <<= 1) {
            if ((mask & bit) == 0) {
                if ((running & bit) != 0) {
                    Sensor s = sensor(sm, bit);
                    if (s != null) sm.unregisterListener(listener, s);
                }
                continue;
            }
            Sensor s = sensor(sm, bit);
            if (s == null) continue;
            have |= bit;
            if ((running & bit) == 0) sm.registerListener(listener, s, PERIOD_US, main);
        }
        running = have;
        return have;
    }

    // ------------------------------------------------------------ location

    private static boolean locating, listening;
    private static Location last;

    private static final LocationListener locations = new LocationListener() {
        @Override public void onLocationChanged(Location l) { deliver(l); }
        @Override public void onProviderEnabled(String provider) { switchChanged(); }
        @Override public void onProviderDisabled(String provider) { switchChanged(); }
        // called up to API 28
        @Override public void onStatusChanged(String provider, int status, Bundle extras) {}
    };

    private static boolean has(String permission) {
        return app.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED;
    }
    private static boolean allowed() { return has(Manifest.permission.ACCESS_FINE_LOCATION) || has(Manifest.permission.ACCESS_COARSE_LOCATION); }

    private static boolean enabled(LocationManager lm) {
        if (Build.VERSION.SDK_INT >= 28) return lm.isLocationEnabled();
        for (String p : new String[] {LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER})
            if (lm.getAllProviders().contains(p) && lm.isProviderEnabled(p)) return true;
        return false;
    }

    private static void report(int state) {
        nativeLocation(state, Double.NaN, Double.NaN, Double.NaN, Double.NaN, Double.NaN, Double.NaN, 0, 0);
        pump();
    }

    private static void switchChanged() {
        LocationManager lm = (LocationManager) app.getSystemService(Context.LOCATION_SERVICE);
        if (!locating || lm == null) return;
        report(enabled(lm) ? (last != null ? FIX : SEARCHING) : DISABLED);
    }

    private static int location(int op) {
        LocationManager lm = (LocationManager) app.getSystemService(Context.LOCATION_SERVICE);
        if (op == OP_STOP) {
            locating = false;
            stopUpdates(lm);
            return 0;
        }
        locating = true;
        if (lm == null) return UNAVAILABLE;
        if (allowed()) return startUpdates(lm);
        if (op != OP_START || activity == null) return DENIED;
        // no dialog for a permission that is not in the manifest: the answer comes at once, denied
        activity.requestPermissions(new String[] {Manifest.permission.ACCESS_FINE_LOCATION,
            Manifest.permission.ACCESS_COARSE_LOCATION}, PERMISSION_CODE);
        return SEARCHING;
    }

    /** GalleryActivity.onRequestPermissionsResult -> true when it was ours. */
    static boolean permissions(int code, int[] results) {
        if (code != PERMISSION_CODE) return false;
        if (!locating) return true;
        LocationManager lm = (LocationManager) app.getSystemService(Context.LOCATION_SERVICE);
        if (lm == null) report(UNAVAILABLE);
        else if (!allowed()) report(DENIED);
        else report(startUpdates(lm));
        return true;
    }

    private static int startUpdates(LocationManager lm) {
        last = null;
        boolean fine = has(Manifest.permission.ACCESS_FINE_LOCATION);
        List<String> all = lm.getAllProviders();
        boolean gps = fine && all.contains(LocationManager.GPS_PROVIDER);
        boolean network = all.contains(LocationManager.NETWORK_PROVIDER);
        if (!gps && !network) return UNAVAILABLE;
        if (listening) stopUpdates(lm);
        try {
            if (gps) lm.requestLocationUpdates(LocationManager.GPS_PROVIDER, FIX_MS, 0f, locations, Looper.getMainLooper());
            if (network) lm.requestLocationUpdates(LocationManager.NETWORK_PROVIDER, FIX_MS, 0f, locations, Looper.getMainLooper());
            listening = true;
            if (!enabled(lm)) return DISABLED;
            // what the system knows already, while the first fix of this run takes its time
            Location known = gps ? lm.getLastKnownLocation(LocationManager.GPS_PROVIDER) : null;
            if (known == null && network) known = lm.getLastKnownLocation(LocationManager.NETWORK_PROVIDER);
            if (known != null) {
                deliver(known);
                return FIX;
            }
        } catch (SecurityException e) {
            Log.i(TAG, "location: " + e);
            stopUpdates(lm);
            return DENIED;
        } catch (IllegalArgumentException e) {
            Log.i(TAG, "location: " + e);
            stopUpdates(lm);
            return UNAVAILABLE;
        }
        return SEARCHING;
    }

    private static void stopUpdates(LocationManager lm) {
        if (lm != null && listening) lm.removeUpdates(locations);
        listening = false;
    }

    private static boolean isGps(Location l) { return LocationManager.GPS_PROVIDER.equals(l.getProvider()); }
    private static long ageMs(Location l) { return (SystemClock.elapsedRealtimeNanos() - l.getElapsedRealtimeNanos()) / 1000000; }

    private static void deliver(Location l) {
        if (!locating || l == null) return;
        // a coarse network fix does not push aside what the satellites said a moment ago
        if (last != null && isGps(last) && !isGps(l) && ageMs(last) < GPS_WINDOW_MS
            && (!l.hasAccuracy() || (last.hasAccuracy() && l.getAccuracy() > last.getAccuracy()))) return;
        last = l;
        nativeLocation(FIX, l.getLatitude(), l.getLongitude(),
            l.hasAccuracy() ? l.getAccuracy() : Double.NaN, l.hasAltitude() ? l.getAltitude() : Double.NaN,
            l.hasSpeed() ? l.getSpeed() : Double.NaN, l.hasBearing() ? l.getBearing() : Double.NaN,
            Math.max(0, ageMs(l)), isGps(l) ? 1 : LocationManager.NETWORK_PROVIDER.equals(l.getProvider()) ? 2 : 0);
        pump();
    }
}
