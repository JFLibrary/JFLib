/* Sound output of the Android host (lib/plat/android_audio.rs): one AAudio stream per
 * plat::audio stream, f32 at the stream's rate and channels (AAudio converts to the device),
 * shared mode, low latency.  Its data callback renders from the stream's Feed in Rust
 * (jfgui_audio_render, on AAudio's own thread; 0 back: the stream has played its end, AAudio
 * stops it).  A disconnected device (headphones out, a new default output) reopens the stream
 * on a thread of its own -- the default device then -- and counts a device change.  The handle
 * Rust keeps is the struct below; jfgui_host_audio_ctl(CLOSE) frees it once no reopening
 * thread holds it, and after that no callback reaches the Feed. */
#include <aaudio/AAudio.h>
#include <pthread.h>
#include <stdint.h>
#include <stdlib.h>
#include <time.h>

extern int32_t jfgui_audio_render(void *feed, float *out, int32_t frames, int32_t rate);

enum { OP_START = 1, OP_PAUSE = 2, OP_FLUSH = 3, OP_CLOSE = 4 };
enum { INFO_RATE = 1, INFO_BUFFER = 2, INFO_READ = 3, INFO_XRUNS = 4, INFO_CHANGES = 5, INFO_DEVICE = 6, INFO_STATE = 7 };

struct jf_audio {
    AAudioStream *stream;
    void *feed;
    int32_t rate, channels, buffer, playing, closing, changes, refs, xruns;
    pthread_mutex_t lock;
};

static aaudio_data_callback_result_t audio_data(AAudioStream *s, void *user, void *data, int32_t frames) {
    struct jf_audio *a = user;
    return jfgui_audio_render(a->feed, data, frames, AAudioStream_getSampleRate(s))
        ? AAUDIO_CALLBACK_RESULT_CONTINUE : AAUDIO_CALLBACK_RESULT_STOP;
}
static void audio_error(AAudioStream *s, void *user, aaudio_result_t error);

/* a stream for a (lock held) -> 0 or an AAudio error */
static aaudio_result_t audio_make(struct jf_audio *a) {
    AAudioStreamBuilder *b = NULL;
    aaudio_result_t r = AAudio_createStreamBuilder(&b);
    if (r != AAUDIO_OK) return r;
    AAudioStreamBuilder_setDirection(b, AAUDIO_DIRECTION_OUTPUT);
    AAudioStreamBuilder_setSharingMode(b, AAUDIO_SHARING_MODE_SHARED);
    AAudioStreamBuilder_setPerformanceMode(b, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
    AAudioStreamBuilder_setFormat(b, AAUDIO_FORMAT_PCM_FLOAT);
    AAudioStreamBuilder_setSampleRate(b, a->rate);
    AAudioStreamBuilder_setChannelCount(b, a->channels);
    AAudioStreamBuilder_setDataCallback(b, audio_data, a);
    AAudioStreamBuilder_setErrorCallback(b, audio_error, a);
    AAudioStream *s = NULL;
    r = AAudioStreamBuilder_openStream(b, &s);
    AAudioStreamBuilder_delete(b);
    if (r != AAUDIO_OK) return r;
    /* the latency aimed for, at least two bursts */
    int32_t burst = AAudioStream_getFramesPerBurst(s);
    int32_t want = a->buffer > 2 * burst ? a->buffer : 2 * burst;
    AAudioStream_setBufferSizeInFrames(s, want);
    a->stream = s;
    if (a->playing) AAudioStream_requestStart(s);
    return AAUDIO_OK;
}

static void audio_release(struct jf_audio *a) {
    /* frees a when nobody holds it any more (the lock is not held) */
    if (__atomic_sub_fetch(&a->refs, 1, __ATOMIC_ACQ_REL) == 0) {
        pthread_mutex_destroy(&a->lock);
        free(a);
    }
}

/* the device went away: close the old stream and open one on the default device, retried
 * every second until it works or the stream is closed */
static void *audio_reopen(void *user) {
    struct jf_audio *a = user;
    pthread_mutex_lock(&a->lock);
    if (a->stream) {
        a->xruns += AAudioStream_getXRunCount(a->stream);
        AAudioStream_close(a->stream);
        a->stream = NULL;
    }
    while (!__atomic_load_n(&a->closing, __ATOMIC_ACQUIRE) && audio_make(a) != AAUDIO_OK) {
        pthread_mutex_unlock(&a->lock);
        struct timespec t = {1, 0};
        nanosleep(&t, NULL);
        pthread_mutex_lock(&a->lock);
    }
    if (!__atomic_load_n(&a->closing, __ATOMIC_ACQUIRE)) a->changes++;
    pthread_mutex_unlock(&a->lock);
    audio_release(a);
    return NULL;
}

static void audio_error(AAudioStream *s, void *user, aaudio_result_t error) {
    struct jf_audio *a = user;
    (void)s;
    if (error != AAUDIO_ERROR_DISCONNECTED || __atomic_load_n(&a->closing, __ATOMIC_ACQUIRE)) return;
    /* not on this thread, and without the lock here: AAudio forbids closing a stream from its
     * own callbacks, and a close waiting for this callback may hold the lock */
    pthread_t t;
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED);
    __atomic_add_fetch(&a->refs, 1, __ATOMIC_ACQ_REL);
    if (pthread_create(&t, &attr, audio_reopen, a) != 0) __atomic_sub_fetch(&a->refs, 1, __ATOMIC_ACQ_REL);
    pthread_attr_destroy(&attr);
}

/* a started stream rendering from feed -> its handle, or 0 (no output) */
uintptr_t jfgui_host_audio_open(void *feed, int32_t rate, int32_t channels, int32_t buffer_frames) {
    struct jf_audio *a = calloc(1, sizeof *a);
    if (!a) return 0;
    a->feed = feed;
    a->rate = rate;
    a->channels = channels;
    a->buffer = buffer_frames;
    a->playing = 1;
    a->refs = 1;
    pthread_mutex_init(&a->lock, NULL);
    pthread_mutex_lock(&a->lock);
    aaudio_result_t r = audio_make(a);
    pthread_mutex_unlock(&a->lock);
    if (r != AAUDIO_OK) {
        pthread_mutex_destroy(&a->lock);
        free(a);
        return 0;
    }
    return (uintptr_t)a;
}

/* start, pause, flush (paused, the buffer dropped) or close (no callback after it returns) */
int32_t jfgui_host_audio_ctl(uintptr_t h, int32_t op) {
    struct jf_audio *a = (struct jf_audio *)h;
    if (!a) return -1;
    pthread_mutex_lock(&a->lock);
    aaudio_result_t r = AAUDIO_OK;
    AAudioStream *s = a->stream;
    if (op == OP_START) {
        a->playing = 1;
        if (s) r = AAudioStream_requestStart(s);
    } else if (op == OP_PAUSE || op == OP_FLUSH) {
        a->playing = 0;
        if (s) {
            r = AAudioStream_requestPause(s);
            if (op == OP_FLUSH) r = AAudioStream_requestFlush(s);
        }
    } else if (op == OP_CLOSE) {
        __atomic_store_n(&a->closing, 1, __ATOMIC_RELEASE);
        if (s) AAudioStream_close(s);
        a->stream = NULL;
        pthread_mutex_unlock(&a->lock);
        audio_release(a);
        return 0;
    }
    pthread_mutex_unlock(&a->lock);
    return r;
}

/* the stream's rate, buffer size, frames played, underruns, device changes, device id, state */
int64_t jfgui_host_audio_info(uintptr_t h, int32_t what) {
    struct jf_audio *a = (struct jf_audio *)h;
    if (!a) return -1;
    pthread_mutex_lock(&a->lock);
    AAudioStream *s = a->stream;
    int64_t v = -1;
    switch (what) {
        case INFO_RATE: v = s ? AAudioStream_getSampleRate(s) : 0; break;
        case INFO_BUFFER: v = s ? AAudioStream_getBufferSizeInFrames(s) : 0; break;
        case INFO_READ: v = s ? AAudioStream_getFramesRead(s) : 0; break;
        case INFO_XRUNS: v = a->xruns + (s ? AAudioStream_getXRunCount(s) : 0); break;
        case INFO_CHANGES: v = a->changes; break;
        case INFO_DEVICE: v = s ? AAudioStream_getDeviceId(s) : 0; break;
        case INFO_STATE: v = s ? AAudioStream_getState(s) : 0; break;
    }
    pthread_mutex_unlock(&a->lock);
    return v;
}
