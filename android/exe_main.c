/* A program without a UI on Android (apps/gputest, coretest, plattest, asynctest): a plain executable
 * for `adb shell`, no APK, no Java (tools/android.py build_exe).  This stands in for what bridge.c does
 * for an app: the jfgui_* variables of the environment and the arguments go in before the start, the
 * output goes to stdout (rt android.rs), the exit code is jfgui_main's.  Nothing here can ask the
 * system's services; a surface needs the app's SurfaceView, so what wants a window SKIPs
 * (apps/androidGallery has that part: tests/android_gpu.py).
 *
 * Built with -DJFGUI_CONSOLE_APK it is the same program in an app's process instead (docs/ANDROID.md,
 * AN0): tools/android_console/ConsoleInstrumentation.java calls run() below on a thread of its own, in
 * the SELinux domain untrusted_app and under Zygote's seccomp filter, which `adb shell` has neither of.
 * The output goes to a file (it survives a SIGSYS) and then to logcat, the exit code after it. */
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
/* no sound output (audio.c is the app's): plat::audio answers "no device" */
uintptr_t jfgui_host_audio_open(void *feed, int32_t rate, int32_t channels, int32_t buffer_frames) {
    (void)feed; (void)rate; (void)channels; (void)buffer_frames;
    return 0;
}
int32_t jfgui_host_audio_ctl(uintptr_t h, int32_t op) {
    (void)h; (void)op;
    return -1;
}
int64_t jfgui_host_audio_info(uintptr_t h, int32_t what) {
    (void)h; (void)what;
    return -1;
}

/* one NAME=VALUE of the environment, when it is one of the library's (jfgui_*) */
static void put_env(const char *entry, size_t n) {
    if (n < 6 || strncmp(entry, "jfgui_", 6) != 0) return;
    uintptr_t buffer = n < 4096 ? jfgui_buffer((uint32_t)n) : 0;
    if (!buffer) return;
    memcpy((void *)buffer, entry, n);
    (void)jfgui_env((uint32_t)n);
}

/* the arguments, one per line (without the program's name) -> jfgui_main's exit code */
static int start(const char *lines, size_t total) {
    uintptr_t buffer = jfgui_buffer((uint32_t)total);
    if (total && !buffer) return 2;
    if (total && (const char *)buffer != lines) memcpy((void *)buffer, lines, total);
    int code = jfgui_start(1, 1, 96, (uint32_t)total);
    fflush(stdout);
    return code;
}

#ifndef JFGUI_CONSOLE_APK
int main(int argc, char **argv) {
    jfgui_console();
    for (char **e = environ; e && *e; e++) put_env(*e, strlen(*e));
    size_t total = 0;
    for (int i = 1; i < argc; i++) total += strlen(argv[i]) + 1;
    char *lines = (char *)jfgui_buffer((uint32_t)total);    /* reused by start: no allocation */
    if (total && !lines) return 2;
    char *p = lines;
    for (int i = 1; i < argc; i++) {
        size_t n = strlen(argv[i]);
        memcpy(p, argv[i], n);
        p[n] = '\n';
        p += n + 1;
    }
    return start(lines, total);
}
#else
#include <jni.h>
#include <android/log.h>
#include <fcntl.h>
#include <stdlib.h>
#include <unistd.h>

static void put(const char *text) { (void)!write(1, text, strlen(text)); }

/* the first line of the output: where this runs (the runner checks for untrusted_app and seccomp) */
static void where(void) {
    char context[256] = "?", status[4096], line[400];
    int fd = open("/proc/self/attr/current", O_RDONLY | O_CLOEXEC);
    if (fd >= 0) {
        ssize_t n = read(fd, context, sizeof context - 1);
        context[n > 0 ? n : 0] = 0;
        close(fd);
        context[strcspn(context, "\n")] = 0;
    }
    const char *seccomp = "?";
    fd = open("/proc/self/status", O_RDONLY | O_CLOEXEC);
    if (fd >= 0) {
        ssize_t n = read(fd, status, sizeof status - 1);
        status[n > 0 ? n : 0] = 0;
        close(fd);
        char *s = strstr(status, "\nSeccomp:");
        if (s) {
            s += 9;
            while (*s == ' ' || *s == '\t') s++;
            s[strcspn(s, "\n")] = 0;
            seccomp = s;
        }
    }
    snprintf(line, sizeof line, "jfgui-console: context %s, seccomp %s, pid %d\n", context, seccomp, (int)getpid());
    put(line);
}

/* the output file, line by line, to logcat (tag jfgui-console) */
static void to_logcat(const char *path) {
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return;
    char chunk[4096], line[1024];
    size_t used = 0;
    ssize_t n;
    while ((n = read(fd, chunk, sizeof chunk)) > 0) {
        for (ssize_t i = 0; i < n; i++) {
            if (chunk[i] == '\n' || used == sizeof line - 1) {
                line[used] = 0;
                __android_log_write(ANDROID_LOG_INFO, "jfgui-console", line);
                used = 0;
                if (chunk[i] == '\n') continue;
            }
            line[used++] = chunk[i];
        }
    }
    if (used) {
        line[used] = 0;
        __android_log_write(ANDROID_LOG_INFO, "jfgui-console", line);
    }
    close(fd);
}

/* ConsoleInstrumentation.run(args, env, dir, out): args and env one per line; dir the working
 * directory (the app's files), out the file the output goes to -> the exit code */
JNIEXPORT jint JNICALL Java_org_jflib_console_ConsoleInstrumentation_run(
        JNIEnv *env, jclass cls, jstring args, jstring vars, jstring dir, jstring out) {
    (void)cls;
    const char *a = (*env)->GetStringUTFChars(env, args, NULL);
    const char *v = (*env)->GetStringUTFChars(env, vars, NULL);
    const char *d = (*env)->GetStringUTFChars(env, dir, NULL);
    const char *o = (*env)->GetStringUTFChars(env, out, NULL);
    int code = 2;
    char *path = o ? strdup(o) : NULL;
    int fd = path ? open(path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0600) : -1;
    if (a && v && d && fd >= 0 && chdir(d) == 0) {
        /* stdout and stderr of an app are /dev/null and belong to nobody (fdsan): the file instead */
        dup2(fd, 1);
        dup2(fd, 2);
        jfgui_console();
        where();
        for (const char *e = v; *e;) {
            size_t n = strcspn(e, "\n");
            put_env(e, n);
            e += n + (e[n] == '\n');
        }
        code = start(a, strlen(a));
        char line[32];
        snprintf(line, sizeof line, "exit=%d\n", code);
        put(line);
        int null = open("/dev/null", O_RDWR | O_CLOEXEC);
        if (null >= 0) {
            dup2(null, 1);
            dup2(null, 2);
            close(null);
        }
    }
    if (fd >= 0) close(fd);
    if (path) {
        to_logcat(path);
        free(path);
    }
    if (a) (*env)->ReleaseStringUTFChars(env, args, a);
    if (v) (*env)->ReleaseStringUTFChars(env, vars, v);
    if (d) (*env)->ReleaseStringUTFChars(env, dir, d);
    if (o) (*env)->ReleaseStringUTFChars(env, out, o);
    return code;
}
#endif
