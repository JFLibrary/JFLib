/* A program without a UI on Android (apps/gputest): a plain executable for `adb shell`, no APK, no
 * Java (tools/android.py build_exe).  This stands in for what bridge.c does for an app: the jfgui_*
 * variables of the environment and the arguments go in before the start, the output goes to stdout
 * (rt android.rs), the exit code is jfgui_main's.  Nothing here can ask the system's services; a
 * surface needs the app's SurfaceView, so what wants a window SKIPs (apps/androidGallery has that
 * part: tests/android_gpu.py). */
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

extern char **environ;
extern uintptr_t jfgui_buffer(uint32_t length);
extern int32_t jfgui_start(int32_t width, int32_t height, int32_t dpi, uint32_t args_len);
extern uint32_t jfgui_env(uint32_t length);
extern uint32_t jfgui_complete(uint32_t id, int32_t status);
extern void jfgui_console(void);

double jfgui_host_time_us(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return 0;
    return (double)now.tv_sec * 1000000.0 + (double)now.tv_nsec / 1000.0;
}
void jfgui_host_present(const uint8_t *source, size_t length, int32_t width, int32_t height, int32_t stride,
                        const int32_t (*rects)[4], size_t count, uint32_t order) {
    (void)source; (void)length; (void)width; (void)height; (void)stride; (void)rects; (void)count; (void)order;
}
void jfgui_host_request(uint32_t kind, uint32_t id, const uint8_t *bytes, size_t length) {
    (void)kind; (void)bytes; (void)length;
    if (id) (void)jfgui_complete(id, -3);
}
int32_t jfgui_host_call(uint32_t kind, const uint8_t *bytes, size_t length) {
    (void)kind; (void)bytes; (void)length;
    return -1;
}

int main(int argc, char **argv) {
    jfgui_console();
    for (char **e = environ; e && *e; e++) {
        if (strncmp(*e, "jfgui_", 6) != 0) continue;
        size_t n = strlen(*e);
        uintptr_t buffer = n < 4096 ? jfgui_buffer((uint32_t)n) : 0;
        if (!buffer) continue;
        memcpy((void *)buffer, *e, n);
        (void)jfgui_env((uint32_t)n);
    }
    size_t total = 0;
    for (int i = 1; i < argc; i++) total += strlen(argv[i]) + 1;
    uintptr_t buffer = jfgui_buffer((uint32_t)total);
    if (total && !buffer) return 2;
    char *p = (char *)buffer;
    for (int i = 1; i < argc; i++) {
        size_t n = strlen(argv[i]);
        memcpy(p, argv[i], n);
        p[n] = '\n';
        p += n + 1;
    }
    int code = jfgui_start(1, 1, 96, (uint32_t)total);
    fflush(stdout);
    return code;
}
