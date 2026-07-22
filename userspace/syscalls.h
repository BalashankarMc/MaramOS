#include "memory.h"
#include <stdint.h>
#include <stdbool.h>

#define SYSCALL_BUFFER ((char*) 0x7FFFFC000000)

static inline uint64_t syscall(uint64_t number, uint64_t sub_arg);
static inline bool sys_print(char *str);

static inline uint64_t syscall(uint64_t number, uint64_t sub_arg) {
    uint64_t ret;
    __asm__ volatile (
        "syscall"
        : "=a" (ret)
        : "a" (number), "b" (sub_arg)
        : "rcx", "r11", "memory"
    );

    return ret;
}

static inline bool sys_print(char *str) {
    strcpy(str, SYSCALL_BUFFER);

    uint64_t res = syscall(0, 0);

    return res == 0;
}