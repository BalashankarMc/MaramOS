#include <stdint.h>

inline void memcpy(void *src, void* dst, uint64_t bytes);
inline uint64_t strlen(char *str);
inline uint64_t strcpy(char *src, char *dest);
inline void memset(void *dst, char value, uint64_t bytes);

inline void memcpy(void *src, void* dst, uint64_t bytes) {
    uint64_t bytes_copied = 0;

    char *source = (char*) src;
    char *dest = (char*) dst;

    while (bytes > bytes_copied) {
        *dest++ = *source++;
        bytes_copied++;
    }

    return;
}

inline uint64_t strlen(char *str) {
    uint64_t size = 0;

    while (*str++) { size ++; }

    return size;
}

inline uint64_t strcpy(char *src, char *dest) {
    uint64_t bytes_copied = 0;

    while (*src) {
        *dest++ = *src++;
        bytes_copied++;
    }

    *dest++ = 0;

    return bytes_copied + 1;
}

inline void memset(void *dst, char value, uint64_t bytes) {
    char *p = (char*) dst;

    uint64_t i = 0;
    while (i < bytes) {
        *p++ = value;
        i++;
    }

    return;
}