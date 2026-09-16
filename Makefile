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
	test -f $(DISK_IMG) || truncate -s $$(( $(DISK_SIZE) * 4 * 1024 * 1024 )) $(DISK_IMG)

	@set -e; \
	if ! python3 tools/fat_tool.py --image $(DISK_IMG) info 2>/dev/null | grep -qi '$(LEMONFS_GUID)'; then \
		echo "re-creating partition table"; \
		parted -s $(DISK_IMG) mklabel gpt; \
		parted -s $(DISK_IMG) mkpart ESP fat32 1MiB $(ESP_SIZE)MiB set 1 esp on; \
		parted -s $(DISK_IMG) mkpart LemonFS $(ESP_SIZE)MiB 100%; \
		sgdisk -t 2:$(LEMONFS_GUID) -c 2:LemonFS $(DISK_IMG); \
	fi

	python3 tools/fat_tool.py --image $(DISK_IMG) is-formatted || \
	python3 tools/fat_tool.py --image $(DISK_IMG) format

	python3 tools/fat_tool.py --image $(DISK_IMG) mkdir /EFI /EFI/BOOT /boot
	python3 tools/fat_tool.py --image $(DISK_IMG) write limine-files/BOOTX64.EFI /EFI/BOOT/BOOTX64.EFI
	python3 tools/fat_tool.py --image $(DISK_IMG) write limine-files/limine.conf /boot/limine.conf
	python3 tools/fat_tool.py --image $(DISK_IMG) write kernel.elf /boot/kernel.elf