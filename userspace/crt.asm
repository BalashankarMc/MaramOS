bits 64

section .text

extern main
extern __preinit_array_start, __preinit_array_end
extern __fini_array_start, __fini_array_end
extern __init_array_start, __init_array_end

global _start
_start:
    ; Preinit Array
    mov r15, __preinit_array_start
    mov r14, __preinit_array_end
    mov r13, init_array_loop
    call init_array

    ; Init Array
    mov r15, __init_array_start
    mov r14, __init_array_end
    mov r13, init_array_loop
    call init_array

    call main

    mov r12, rax

    ; Fini Array
    mov r15, __fini_array_start
    mov r14, __fini_array_end
    mov r13, fini_array_loop
    call init_array

    ; Terminate process
    mov rax, 1
    mov rbx, 2
    mov rdi, 0x7FFFFC000000
    mov [rdi], r12
    syscall

loop:
    nop
    jmp loop

init_array:
    cmp r15, r14
    je return
    call r13
    ret

init_array_loop:
    call [r15]
    add r15, 8
    cmp r15, r14
    jne init_array_loop
    ret

fini_array_loop:
    sub r14, 8
    call [r14]
    cmp r15, r14
    jne fini_array_loop
    ret

return:
    ret