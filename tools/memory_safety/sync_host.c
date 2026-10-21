#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
extern unsigned review_sync(unsigned);
struct work_args { void (*work)(const void *); const void *data; pthread_barrier_t *barrier; };
static void *run(void *p) {
    struct work_args *a = p;
    pthread_barrier_wait(a->barrier);
    a->work(a->data);
    return NULL;
}
void host_two_threads(void (*work)(const void *), const void *data) {
    pthread_t threads[2];
    pthread_barrier_t barrier;
    if (pthread_barrier_init(&barrier, NULL, 2)) abort();
    struct work_args a = { work, data, &barrier };
    for (int i = 0; i < 2; ++i) if (pthread_create(&threads[i], NULL, run, &a)) abort();
    for (int i = 0; i < 2; ++i) if (pthread_join(threads[i], NULL)) abort();
    pthread_barrier_destroy(&barrier);
}
int main(int argc, char **argv) {
    unsigned mode = argc > 1 ? (unsigned)strtoul(argv[1], NULL, 10) : 0;
    printf("mode=%u result=%u\n", mode, review_sync(mode));
    return 0;
}
