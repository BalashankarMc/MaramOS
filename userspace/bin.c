#include "syscalls.h"

int main() {
    char str[22] = "Hello, World! From C!\0";

    return sys_print(str);
}