/* The system's side for plat that is no frame and no file (android/GallerySystem.java,
 * GalleryNotify.java, GallerySensors.java; lib/plat/web_system.rs, web_sensors.rs): the user's
 * settings as "key=value" lines (jfgui_system: before the start and after every change), the
 * answers of the notifications (jfgui_note: clicked or gone), sensor readings and location fixes
 * (jfgui_sensor, jfgui_location).  The Java side then sends nativeEvent(E_WAKE) so the app hears
 * it at once.  All calls run on the Java UI thread. */
#include <jni.h>
#include <stdint.h>

extern uintptr_t jfgui_buffer(uint32_t length);
extern uint32_t jfgui_system(uint32_t length);
extern uint32_t jfgui_note(uint32_t id, uint32_t clicked);
extern uint32_t jfgui_sensor(uint32_t bit, float x, float y, float z, float w, uint32_t flags);
extern uint32_t jfgui_location(uint32_t state, double lat, double lon, double accuracy, double altitude,
                               double speed, double bearing, double age_ms, uint32_t source);

JNIEXPORT void JNICALL Java_org_jflib_gallery_GallerySystem_nativeSettings(JNIEnv *env, jclass cls, jbyteArray text) {
    (void)cls;
    if (!text) return;
    jsize length = (*env)->GetArrayLength(env, text);
    if (length <= 0 || length > 65536) return;
    uintptr_t buffer = jfgui_buffer((uint32_t)length);
    if (!buffer) return;
    (*env)->GetByteArrayRegion(env, text, 0, length, (jbyte *)buffer);
    if ((*env)->ExceptionCheck(env)) return;
    (void)jfgui_system((uint32_t)length);
}

JNIEXPORT void JNICALL Java_org_jflib_gallery_GalleryNotify_nativeNote(JNIEnv *env, jclass cls, jint id, jboolean clicked) {
    (void)env; (void)cls;
    (void)jfgui_note((uint32_t)id, clicked ? 1u : 0u);
}


JNIEXPORT void JNICALL Java_org_jflib_gallery_GallerySensors_nativeSensor(JNIEnv *env, jclass cls, jint bit, jfloat x, jfloat y,
        jfloat z, jfloat w, jint flags) {
    (void)env; (void)cls;
    (void)jfgui_sensor((uint32_t)bit, x, y, z, w, (uint32_t)flags);
}

JNIEXPORT void JNICALL Java_org_jflib_gallery_GallerySensors_nativeLocation(JNIEnv *env, jclass cls, jint state, jdouble lat,
        jdouble lon, jdouble accuracy, jdouble altitude, jdouble speed, jdouble bearing, jdouble age_ms, jint source) {
    (void)env; (void)cls;
    (void)jfgui_location((uint32_t)state, lat, lon, accuracy, altitude, speed, bearing, age_ms, (uint32_t)source);
}
