#include "syscalls.h"

__attribute__((constructor))
static void ctor(void) {
    sys_print("Constructor: before main\n");
}

__attribute__((destructor))
static void dtor(void) {
    sys_print("Destructor: after main\n");
}

int main(void) {
    sys_print("Main: running\n");
    return 0;
}