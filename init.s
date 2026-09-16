global .start

.start:
	nop

fault:
	ud2
	jmp fault
