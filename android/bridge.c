/* Native Android host boundary. All callbacks run on the Java UI thread.
 * Rust owns widgets, layout, fonts, state and pixels; Java owns the Android View/IME.
 * Rust asks Java synchronously through jfgui_host_call (GalleryHost.call: the ContentResolver,
 * the clipboard, the secret store; lib/plat/android_host.rs) and starts the system's picker through
 * jfgui_host_request (GalleryHost.request), whose answer comes back in GalleryHost.nativeResult.
 */
#include <jni.h>
#include <android/bitmap.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <time.h>

extern uintptr_t jfgui_buffer(uint32_t length);
extern int32_t jfgui_start(int32_t width, int32_t height, int32_t dpi, uint32_t args_len);
extern void jfgui_tick(uint32_t time_ms);
extern void jfgui_input(uint32_t time_ms);
extern uint32_t jfgui_event(uint32_t type, uint32_t button, uint32_t mods, uint32_t window,
                             int32_t x, int32_t y, uint32_t data);
extern uint32_t jfgui_text(uint32_t length);
extern uint32_t jfgui_env(uint32_t length);
extern uint32_t jfgui_complete(uint32_t id, int32_t status);
extern uint32_t jfgui_frame_delay(void);
extern uint32_t jfgui_editor_snapshot(void);
extern uintptr_t jfgui_editor_data(void);
extern uint32_t jfgui_editor_replace(uint32_t id, uint32_t length, uint32_t anchor, uint32_t caret);
extern uint32_t jfgui_a11y(uint32_t on);
extern uint32_t jfgui_a11y_act(uint32_t win, uint32_t key_lo, uint32_t key_hi, uint32_t act, int32_t a, int32_t b,
                               double value, uint32_t length);
extern uint32_t jfgui_a11y_take(void);
extern uintptr_t jfgui_a11y_data(void);
extern uintptr_t jfgui_reply(uint32_t length);
extern uint32_t jfgui_file_result(uint32_t id, int32_t status, uint32_t length);
extern void jfgui_content_changed(void);

static JNIEnv *target_env;
static jobject target_bitmap;
static AndroidBitmapInfo target_info;
static int started, last_width, last_height, last_dpi, ime_visible, presented;
static int a11y_wanted = -1; /* the accessibility mirror asked for before the app started */
static uint64_t epoch_ms;
/* GalleryHost: call(int, byte[]) -> byte[], request(int, int, byte[]) -> boolean */
static JavaVM *java_vm;
static jclass host_class;
static jmethodID host_call, host_request;

JNIEXPORT jint JNI_OnLoad(JavaVM *vm, void *reserved) {
    (void)reserved;
    JNIEnv *env;
    java_vm = vm;
    if ((*vm)->GetEnv(vm, (void **)&env, JNI_VERSION_1_6) != JNI_OK) return JNI_ERR;
    /* FindClass in JNI_OnLoad sees the app's classes (the loader of System.loadLibrary's caller) */
    jclass found = (*env)->FindClass(env, "org/jflib/gallery/GalleryHost");
    if (!found) {
        (*env)->ExceptionClear(env); /* a host without services: every request is Unsupported */
        return JNI_VERSION_1_6;
    }
    host_class = (jclass)(*env)->NewGlobalRef(env, found);
    host_call = (*env)->GetStaticMethodID(env, found, "call", "(I[B)[B");
    host_request = (*env)->GetStaticMethodID(env, found, "request", "(II[B)Z");
    if ((*env)->ExceptionCheck(env) || !host_call || !host_request) {
        (*env)->ExceptionClear(env);
        host_class = NULL;
    }
    (*env)->DeleteLocalRef(env, found);
    return JNI_VERSION_1_6;
}
static JNIEnv *host_env(void) {
    JNIEnv *env;
    if (!java_vm || !host_class || (*java_vm)->GetEnv(java_vm, (void **)&env, JNI_VERSION_1_6) != JNI_OK) return NULL;
    return env;
}
static jbyteArray host_bytes(JNIEnv *env, const uint8_t *bytes, size_t length) {
    if (length > (256u << 20)) return NULL;
    jbyteArray array = (*env)->NewByteArray(env, (jsize)length);
    if (array && length) (*env)->SetByteArrayRegion(env, array, 0, (jsize)length, (const jbyte *)bytes);
    return array;
}
/* One synchronous question to GalleryHost.call -> the answer's length (its bytes in the buffer of
 * jfgui_reply), or -1 (no host, null, an exception). */
int32_t jfgui_host_call(uint32_t kind, const uint8_t *bytes, size_t length) {
    JNIEnv *env = host_env();
    if (!env) return -1;
    jbyteArray in = host_bytes(env, bytes, length);
    if (!in) { (*env)->ExceptionClear(env); return -1; }
    jbyteArray out = (jbyteArray)(*env)->CallStaticObjectMethod(env, host_class, host_call, (jint)kind, in);
    (*env)->DeleteLocalRef(env, in);
    if ((*env)->ExceptionCheck(env)) {
        (*env)->ExceptionClear(env);
        return -1;
    }
    if (!out) return -1;
    jsize n = (*env)->GetArrayLength(env, out);
    uintptr_t buffer = n >= 0 && n <= (jsize)(256u << 20) ? jfgui_reply((uint32_t)n) : 0;
    if (!buffer && n) { (*env)->DeleteLocalRef(env, out); return -1; }
    if (n) (*env)->GetByteArrayRegion(env, out, 0, n, (jbyte *)buffer);
    (*env)->DeleteLocalRef(env, out);
    if ((*env)->ExceptionCheck(env)) { (*env)->ExceptionClear(env); return -1; }
    return (int32_t)n;
}

double jfgui_host_time_us(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return 0;
    return (double)now.tv_sec * 1000000.0 + (double)now.tv_nsec / 1000.0;
}
static uint32_t time_ms(void) {
    uint64_t now = (uint64_t)jfgui_host_time_us() / 1000;
    if (!epoch_ms) epoch_ms = now;
    return (uint32_t)(now - epoch_ms);
}

void jfgui_host_present(const uint8_t *source, size_t length, int32_t width, int32_t height,
                         int32_t stride, const int32_t (*rects)[4], size_t count) {
    if (!target_bitmap || !source || !rects || !count || count > 128
        || width <= 0 || height <= 0 || width > 4096 || height > 4096
        || stride < width * 4 || (size_t)stride * (size_t)height > length
        || target_info.width != (uint32_t)width || target_info.height != (uint32_t)height
        || target_info.stride < (uint32_t)width * 4) return;
    uint8_t *pixels = NULL;
    // Locking even an unchanged Bitmap changes its generation and triggers a GPU upload.
    // Only touch it when Rust reports damage, and only copy those rectangles.
    if (AndroidBitmap_lockPixels(target_env, target_bitmap, (void **)&pixels) != ANDROID_BITMAP_RESULT_SUCCESS) {
        presented = -3;
        return;
    }
    for (size_t i = 0; i < count; ++i) {
        int64_t right = (int64_t)rects[i][0] + rects[i][2];
        int64_t bottom = (int64_t)rects[i][1] + rects[i][3];
        int left = rects[i][0] < 0 ? 0 : rects[i][0];
        int top = rects[i][1] < 0 ? 0 : rects[i][1];
        int end_x = right > width ? width : (int)right;
        int end_y = bottom > height ? height : (int)bottom;
        if (end_x <= left || end_y <= top) continue;
        for (int y = top; y < end_y; ++y) {
            const uint8_t *s = source + (size_t)y * (size_t)stride + (size_t)left * 4;
            uint8_t *d = pixels + (size_t)y * target_info.stride + (size_t)left * 4;
            for (int x = left; x < end_x; ++x) {
                d[0] = s[2]; d[1] = s[1]; d[2] = s[0]; d[3] = 255;
                d += 4; s += 4;
            }
        }
    }
    AndroidBitmap_unlockPixels(target_env, target_bitmap);
    presented = 1;
}
void jfgui_host_request(uint32_t kind, uint32_t id, const uint8_t *bytes, size_t length) {
    if (kind == 6 && bytes && length == 12) {
        int32_t height;
        memcpy(&height, bytes + 8, sizeof(height));
        ime_visible = height > 0;
    }
    /* the system's picker: 2 download (save a copy), 3 pick_file, 12 file_dialog */
    if (id && (kind == 2 || kind == 3 || kind == 12)) {
        JNIEnv *env = host_env();
        jbyteArray payload = env ? host_bytes(env, bytes, length) : NULL;
        if (payload) {
            jboolean taken = (*env)->CallStaticBooleanMethod(env, host_class, host_request, (jint)kind, (jint)id, payload);
            (*env)->DeleteLocalRef(env, payload);
            if ((*env)->ExceptionCheck(env)) { (*env)->ExceptionClear(env); taken = JNI_FALSE; }
            if (taken) return;
        } else if (env) {
            (*env)->ExceptionClear(env);
        }
    }
    /* Resolve every unsupported asynchronous service instead of leaving it pending. */
    if (id) (void)jfgui_complete(id, -3);
}

JNIEXPORT jint JNICALL Java_org_jflib_gallery_GalleryActivity_nativeRender(
        JNIEnv *env, jclass cls, jobject bitmap, jint dpi, jboolean new_surface) {
    (void)cls;
    if (AndroidBitmap_getInfo(env, bitmap, &target_info) != ANDROID_BITMAP_RESULT_SUCCESS
        || target_info.format != ANDROID_BITMAP_FORMAT_RGBA_8888
        || target_info.width == 0 || target_info.height == 0
        || target_info.width > 4096 || target_info.height > 4096) return -2;
    target_env = env;
    target_bitmap = bitmap;
    presented = 0;
    int result = 0;
    if (!started) {
        (void)jfgui_buffer(0);
        result = jfgui_start((int32_t)target_info.width, (int32_t)target_info.height, dpi, 0);
        started = result == 0;
        if (started && a11y_wanted >= 0) (void)jfgui_a11y((uint32_t)a11y_wanted);
    } else {
        if (last_dpi != dpi) jfgui_event(9, 0, 0, 0, 0, 0, (uint32_t)dpi);
        if (new_surface || last_width != (int)target_info.width || last_height != (int)target_info.height)
            jfgui_event(8, 0, 0, 0, (int32_t)target_info.width, (int32_t)target_info.height, 0);
    }
    if (started) {
        last_width = (int)target_info.width;
        last_height = (int)target_info.height;
        last_dpi = dpi;
        jfgui_tick(time_ms());
    }
    target_bitmap = NULL;
    target_env = NULL;
    return result != 0 ? -1 : presented;
}

JNIEXPORT jint JNICALL Java_org_jflib_gallery_GalleryActivity_nativeFrameDelay(JNIEnv *env, jclass cls) {
    (void)env; (void)cls;
    return started ? (jint)jfgui_frame_delay() : 250;
}
JNIEXPORT jbyteArray JNICALL Java_org_jflib_gallery_GalleryActivity_nativeEditorState(JNIEnv *env, jclass cls) {
    (void)cls;
    uint32_t length = jfgui_editor_snapshot();
    if (length > 262160) return NULL;
    jbyteArray out = (*env)->NewByteArray(env, (jsize)length);
    if (out) (*env)->SetByteArrayRegion(env, out, 0, (jsize)length, (const jbyte *)jfgui_editor_data());
    return out;
}
JNIEXPORT void JNICALL Java_org_jflib_gallery_GalleryActivity_nativeEditorReplace(
        JNIEnv *env, jclass cls, jint id, jbyteArray text, jint anchor, jint caret) {
    (void)cls;
    if (!started || !text) return;
    jsize length = (*env)->GetArrayLength(env, text);
    if (length < 0 || length > 262144) return;
    uintptr_t buffer = jfgui_buffer((uint32_t)length);
    if (!buffer) return;
    (*env)->GetByteArrayRegion(env, text, 0, length, (jbyte *)buffer);
    if ((*env)->ExceptionCheck(env)) return;
    if (jfgui_editor_replace((uint32_t)id, (uint32_t)length, (uint32_t)anchor, (uint32_t)caret))
        jfgui_input(time_ms());
}

JNIEXPORT void JNICALL Java_org_jflib_gallery_GalleryActivity_nativeEvent(
        JNIEnv *env, jclass cls, jint type, jint button, jint mods, jint x, jint y, jint data) {
    (void)env; (void)cls;
    if (!started) return;
    jfgui_event((uint32_t)type, (uint32_t)button, (uint32_t)mods, 0, x, y, (uint32_t)data);
    jfgui_input(time_ms());
}
/* "NAME=value" into the app's environment before it starts (LANGUAGE: the system's locales). */
JNIEXPORT void JNICALL Java_org_jflib_gallery_GalleryActivity_nativeEnv(
        JNIEnv *env, jclass cls, jbyteArray entry) {
    (void)cls;
    if (started || !entry) return;
    jsize length = (*env)->GetArrayLength(env, entry);
    if (length <= 0 || length > 4096) return;
    uintptr_t buffer = jfgui_buffer((uint32_t)length);
    if (!buffer) return;
    (*env)->GetByteArrayRegion(env, entry, 0, length, (jbyte *)buffer);
    if ((*env)->ExceptionCheck(env)) return;
    jfgui_env((uint32_t)length);
}

JNIEXPORT void JNICALL Java_org_jflib_gallery_GalleryActivity_nativeText(
        JNIEnv *env, jclass cls, jbyteArray text) {
    (void)cls;
    if (!started || !text) return;
    jsize length = (*env)->GetArrayLength(env, text);
    if (length <= 0 || length > 262144) return;
    uintptr_t buffer = jfgui_buffer((uint32_t)length);
    if (!buffer) return;
    (*env)->GetByteArrayRegion(env, text, 0, length, (jbyte *)buffer);
    if ((*env)->ExceptionCheck(env)) return;
    jfgui_text((uint32_t)length);
    jfgui_input(time_ms());
}
/* The accessibility tree (GalleryAccessibility.java, lib/plat/web_a11y.rs): the mirror on or
 * off, the updates since the last call (each: u32 length + update), an action for a node. */
JNIEXPORT void JNICALL Java_org_jflib_gallery_GalleryActivity_nativeA11y(JNIEnv *env, jclass cls, jboolean on) {
    (void)env; (void)cls;
    a11y_wanted = on ? 1 : 0;
    if (!started) return;
    (void)jfgui_a11y(on ? 1 : 0);
    jfgui_input(time_ms());
}
JNIEXPORT jbyteArray JNICALL Java_org_jflib_gallery_GalleryActivity_nativeA11yTake(JNIEnv *env, jclass cls) {
    (void)cls;
    if (!started) return NULL;
    uint32_t length = jfgui_a11y_take();
    if (length == 0 || length > (64u << 20)) return NULL;
    jbyteArray out = (*env)->NewByteArray(env, (jsize)length);
    if (out) (*env)->SetByteArrayRegion(env, out, 0, (jsize)length, (const jbyte *)jfgui_a11y_data());
    return out;
}
JNIEXPORT void JNICALL Java_org_jflib_gallery_GalleryActivity_nativeA11yAct(
        JNIEnv *env, jclass cls, jint win, jint key_lo, jint key_hi, jint act, jint a, jint b, jdouble value,
        jbyteArray text) {
    (void)cls;
    if (!started) return;
    jsize length = text ? (*env)->GetArrayLength(env, text) : 0;
    if (length < 0 || length > 262144) return;
    uintptr_t buffer = jfgui_buffer((uint32_t)length);
    if (!buffer && length) return;
    if (length) {
        (*env)->GetByteArrayRegion(env, text, 0, length, (jbyte *)buffer);
        if ((*env)->ExceptionCheck(env)) return;
    }
    if (jfgui_a11y_act((uint32_t)win, (uint32_t)key_lo, (uint32_t)key_hi, (uint32_t)act, a, b, value, (uint32_t)length))
        jfgui_input(time_ms());
}
/* The answer of the picker (GalleryHost): status as jfgui_complete (2 done, -4 cancelled ...);
 * uri: the chosen content:// document (file_dialog, pick_file), null for download's export. */
JNIEXPORT void JNICALL Java_org_jflib_gallery_GalleryHost_nativeResult(
        JNIEnv *env, jclass cls, jint id, jint status, jbyteArray uri) {
    (void)cls;
    if (!started) return;
    jsize length = uri ? (*env)->GetArrayLength(env, uri) : 0;
    if (uri && length >= 0 && length <= 65536) {
        uintptr_t buffer = jfgui_buffer((uint32_t)length);
        if (buffer || !length) {
            if (length) (*env)->GetByteArrayRegion(env, uri, 0, length, (jbyte *)buffer);
            if (!(*env)->ExceptionCheck(env)) (void)jfgui_file_result((uint32_t)id, status, (uint32_t)length);
        }
    } else {
        (void)jfgui_complete((uint32_t)id, status);
    }
    jfgui_input(time_ms());
}
/* A provider whose tree the app listed reported a change (GalleryHost's ContentObserver): the
 * name cache of content:// starts again.  Any thread; only a counter. */
JNIEXPORT void JNICALL Java_org_jflib_gallery_GalleryHost_nativeChanged(JNIEnv *env, jclass cls) {
    (void)env; (void)cls;
    jfgui_content_changed();
}
JNIEXPORT jboolean JNICALL Java_org_jflib_gallery_GalleryActivity_nativeIme(
        JNIEnv *env, jclass cls) {
    (void)env; (void)cls;
    return ime_visible ? JNI_TRUE : JNI_FALSE;
}
