bits 64

section .text
extern main

global _start
_start:
    call main

    ; Terminate process
    mov rax, 1
    mov rbx, 2
    syscall

loop:
    nop
    jmp loop