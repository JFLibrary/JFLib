/* The system's side for plat that is no frame and no file (android/GallerySystem.java,
 * GalleryNotify.java; lib/plat/web_system.rs): the user's settings as "key=value" lines
 * (jfgui_system: before the start and after every change) and the answers of the notifications
 * (jfgui_note: clicked or gone).  The Java side then sends nativeEvent(E_WAKE) so the app hears
 * it at once.  All calls run on the Java UI thread. */
#include <jni.h>
#include <stdint.h>

extern uintptr_t jfgui_buffer(uint32_t length);
extern uint32_t jfgui_system(uint32_t length);
extern uint32_t jfgui_note(uint32_t id, uint32_t clicked);

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
