/* Local ASan host; all buffers belong to this process, no files or plugins loaded. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
extern int review_fx(uint8_t *, uint8_t *, unsigned);
int main(int argc, char **argv) {
    unsigned mode = argc > 1 ? (unsigned)strtoul(argv[1], NULL, 10) : 0;
    uint32_t *src = calloc(1, sizeof(*src)), *dst = calloc(1, sizeof(*dst));
    if (!src || !dst) return 2;
    *src = 0x55667788;
    int code = review_fx((uint8_t *)src, (uint8_t *)dst, mode);
    int valid = mode == 0 && code == 0 && *dst == *src;
    printf("mode=%u result=%d copied=%d\n", mode, code, valid);
    free(src);
    free(dst);
    return valid ? 0 : 1;
}
