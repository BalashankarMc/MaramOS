OVMF := /usr/share/edk2/x64/OVMF.4m.fd
DISK_IMG := disk.img

DISK_SIZE := 128
ESP_SIZE  := 50
LEMONFS_GUID := 8CBD0D4E-FE62-49CC-A982-FBD6F69ED888

build:
	make -C kernel
	make disk

run:
	qemu-system-x86_64 -bios /usr/share/edk2/x64/OVMF.4m.fd -m 128M -cpu max -machine q35 \
	-drive if=none,id=sata0,file=$(DISK_IMG),format=raw -device ich9-ahci,id=ahci \
	-device ide-hd,bus=ahci.0,drive=sata0

clean:
	make -C kernel clean
	rm kernel.elf
	rm disk.img

disk:
	dd if=/dev/zero of=$(DISK_IMG) bs=4M count=$(DISK_SIZE)

	parted -s $(DISK_IMG) mklabel gpt
	parted -s $(DISK_IMG) mkpart ESP fat32 1MiB $(ESP_SIZE)MiB set 1 esp on
	parted -s $(DISK_IMG) mkpart LemonFS $(ESP_SIZE)MiB 100%
	sgdisk -t 2:$(LEMONFS_GUID) -c 2:LemonFS $(DISK_IMG)

	S=$$(parted -s $(DISK_IMG) unit s print | awk '/^ *1/ {print $$2}' | tr -d 's') && \
	N=$$(parted -s $(DISK_IMG) unit s print | awk '/^ *1/ {print $$4}' | tr -d 's') && \
	truncate -s $$((N * 512)) /tmp/esp.img && \
	mformat -F -i /tmp/esp.img && \
	mmd -i /tmp/esp.img ::/EFI ::/EFI/BOOT ::/boot && \
	mcopy -i /tmp/esp.img limine-files/BOOTX64.EFI ::/EFI/BOOT/ && \
	mcopy -i /tmp/esp.img limine-files/limine.conf ::/boot/ && \
	mcopy -i /tmp/esp.img kernel.elf ::/boot/ && \
	dd if=/tmp/esp.img of=$(DISK_IMG) bs=512 seek=$$S conv=notrunc && \
	rm -f /tmp/esp.img