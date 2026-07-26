OVMF := /usr/share/edk2/x64/OVMF.4m.fd
DISK_IMG := disk.img

build:
	make -C kernel

	dd if=/dev/zero of=$(DISK_IMG) bs=1M count=64

	parted -s $(DISK_IMG) mklabel gpt mkpart ESP fat32 2048s 100% set 1 esp on
	S=$$(parted -s $(DISK_IMG) unit s print | awk '/^ *1/ {print $$2}' | tr -d 's') && \
	N=$$(parted -s $(DISK_IMG) unit s print | awk '/^ *1/ {print $$4}' | tr -d 's') && \
	truncate -s $$((N * 512)) /tmp/esp.img
	mformat -F -i /tmp/esp.img
	mmd -i /tmp/esp.img ::/EFI ::/EFI/BOOT ::/boot 
	mcopy -i /tmp/esp.img limine-files/BOOTX64.EFI ::/EFI/BOOT/
	mcopy -i /tmp/esp.img limine-files/limine.conf ::/boot/
	mcopy -i /tmp/esp.img kernel.elf ::/boot/
	dd if=/tmp/esp.img of=$(DISK_IMG) bs=512 seek=2048 conv=notrunc
	rm -f /tmp/esp.img

run: build
	qemu-system-x86_64 -hda $(DISK_IMG) -bios /usr/share/edk2/x64/OVMF.4m.fd

clean:
	make -C kernel clean
	rm kernel.elf
