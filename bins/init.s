global _start

_start:
	mov rax, 1
	syscall

loop:
	nop
	jmp loop
