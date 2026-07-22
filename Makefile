KERNEL_DIR := kernel
BOOT_DIR := limine-files
ISO_DIR := iso-root
KERNEL_ELF := $(KERNEL_DIR)/target/x86_64-unknown-none/release/kernel
ISO := maram.iso
QEMU := qemu-system-x86_64
OVMF := /usr/share/OVMF/x64/OVMF.4m.fd

# Disk image configuration
DISK_IMG    := disk.img
DISK_SIZE   := 256M
ESP_SIZE    := 100M
# LemonFS GUID: 4D415241-4D46-5300-8000-000000000001
LEMON_FS_GUID := 4D415241-4D46-5300-8000-000000000001

# External drive deployment (override with: make deploy DEVICE=/dev/sdX)
DEVICE     ?= /dev/sdb
BOOT_PART  := $(DEVICE)1
DATA_PART  := $(DEVICE)2
MNT        := /tmp/maram-deploy

LEMONCC := tools/lemoncc/target/release/lemoncc
USER_DIR := userspace
USER_C_SRCS := $(wildcard $(USER_DIR)/*.c)
USER_ASM_SRCS := $(USER_DIR)/crt.asm
USER_ELFS := $(patsubst $(USER_DIR)/%.c,$(USER_DIR)/%.elf,$(USER_C_SRCS))

CC := gcc
CFLAGS := -ffreestanding -nostdlib -nostartfiles -static -fno-stack-protector -Wall -Wextra -O2 -I$(USER_DIR)
LD := ld
LDFLAGS := -T $(USER_DIR)/user.ld -nostdlib
NASM := nasm

.PHONY: all build disk-img populate-img run test test-unit test-gdb clean FORCE \
	deploy partition-drive format-drive install-boot write-data undeploy user-progs

all build: disk-img

# === Disk Image (GPT: ESP + LemonFS) ===

$(KERNEL_ELF): FORCE
	$(MAKE) -C $(KERNEL_DIR) release

$(LEMONCC): FORCE
	cargo build --release --manifest-path tools/lemoncc/Cargo.toml 2>&1 | tail -1

# Create the GPT disk image with ESP + blank LemonFS partition.
# lemoncc auto-formats the partition during populate-img.
disk-img: $(KERNEL_ELF) $(BOOT_DIR)/limine.conf $(BOOT_DIR)/BOOTX64.EFI
	@echo "==> Creating GPT disk image ($(DISK_SIZE))"
	@rm -f $(DISK_IMG)
	@truncate -s $(DISK_SIZE) $(DISK_IMG)
	@sudo parted -s $(DISK_IMG) mklabel gpt
	@sudo parted -s $(DISK_IMG) mkpart ESP fat32 1MiB 101MiB
	@sudo parted -s $(DISK_IMG) set 1 esp on
	@sudo parted -s $(DISK_IMG) mkpart LemonFS 101MiB 100%
	@sudo sgdisk --typecode=2:4F4D454C-204E-4944-534B-000000000000 --change-name=2:LemonFS $(DISK_IMG)
	@# Format ESP using mtools (no loopback/mount needed)
	@ESP_START=$$(sudo parted -s $(DISK_IMG) unit s print | grep '^ *1' | awk '{print $$2}' | tr -d 's') && \
	ESP_SECTORS=$$(sudo parted -s $(DISK_IMG) unit s print | grep '^ *1' | awk '{print $$4}' | tr -d 's') && \
	truncate -s $$(( ESP_SECTORS * 512 )) /tmp/maram_esp.img && \
	mformat -F -n MARAMBOOT -i /tmp/maram_esp.img && \
	mmd -i /tmp/maram_esp.img ::EFI && \
	mmd -i /tmp/maram_esp.img ::EFI/BOOT && \
	mmd -i /tmp/maram_esp.img ::boot && \
	mcopy -i /tmp/maram_esp.img $(BOOT_DIR)/BOOTX64.EFI ::EFI/BOOT/ && \
	mcopy -i /tmp/maram_esp.img $(BOOT_DIR)/limine.conf ::boot/ && \
	mcopy -i /tmp/maram_esp.img $(KERNEL_ELF) ::boot/kernel.elf && \
	dd if=/tmp/maram_esp.img of=$(DISK_IMG) bs=512 seek=$$ESP_START conv=notrunc status=none && \
	rm -f /tmp/maram_esp.img
	@echo "==> Disk image ready: $(DISK_IMG)"

# Build userspace C programs into ELF binaries.
$(USER_DIR)/%.elf: $(USER_DIR)/%.c $(USER_ASM_SRCS) $(USER_DIR)/user.ld $(USER_DIR)/syscalls.h
	@echo "  CC $<"
	@$(NASM) -f elf64 $(USER_ASM_SRCS) -o /tmp/crt.o
	@$(CC) $(CFLAGS) -c $< -o /tmp/$(notdir $*).o
	@$(LD) $(LDFLAGS) /tmp/crt.o /tmp/$(notdir $*).o -o $@
	@rm -f /tmp/crt.o /tmp/$(notdir $*).o

user-progs: $(USER_ELFS)

# Inject compiled userspace ELFs into the LemonFS partition of disk.img.
# lemoncc auto-formats the partition if it's blank.
populate-img: $(LEMONCC) user-progs disk-img
	@echo "==> Injecting userspace programs into LemonFS partition"
	@DATA_START=$$(sudo parted -s $(DISK_IMG) unit s print | grep '^ *2' | awk '{print $$2}' | tr -d 's') && \
	DATA_END=$$(sudo parted -s $(DISK_IMG) unit s print | grep '^ *2' | awk '{print $$3}' | tr -d 's') && \
	DATA_SECTORS=$$(( DATA_END - DATA_START + 1 )) && \
	dd if=$(DISK_IMG) of=/tmp/maram_lemonfs.img bs=512 skip=$$DATA_START count=$$DATA_SECTORS status=none && \
	for elf in $(USER_ELFS); do \
		stem=$$(basename $$elf .elf); \
		echo "  injecting $$elf -> /$$stem.elf"; \
		$(LEMONCC) $$elf -o /$$stem.elf -d /tmp/maram_lemonfs.img; \
	done && \
	dd if=/tmp/maram_lemonfs.img of=$(DISK_IMG) bs=512 seek=$$DATA_START conv=notrunc status=none && \
	rm -f /tmp/maram_lemonfs.img && \
	echo "==> Injection complete"

FORCE:

# === Run ===

ifeq ($(DISK),nvme)
QEMU_DRIVE := -drive if=none,id=nvm0,file=$(DISK_IMG),format=raw \
		-device nvme,id=nvme0,serial=deadbeef \
		-device nvme-ns,drive=nvm0,bus=nvme0
else
QEMU_DRIVE := -drive if=none,id=sata0,file=$(DISK_IMG),format=raw \
		-device ich9-ahci,id=ahci \
		-device ide-hd,bus=ahci.0,drive=sata0
endif

run: populate-img
	$(QEMU) -bios $(OVMF) -machine q35 -m 512M -smp 4 \
		$(QEMU_DRIVE) &

# === Tests (use ISO for serial output capture) ===

test-unit:
	$(MAKE) -C $(KERNEL_DIR) test-unit

test:
	rm -f $(ISO) serial.out
	rm -rf $(ISO_DIR)
	mkdir -p $(ISO_DIR)/boot $(ISO_DIR)/EFI/BOOT
	cp $(BOOT_DIR)/limine.conf $(ISO_DIR)/boot/
	cp $(BOOT_DIR)/limine-uefi-cd.bin $(ISO_DIR)/boot/
	cp $(BOOT_DIR)/BOOTX64.EFI $(ISO_DIR)/EFI/BOOT/
	$(MAKE) -C $(KERNEL_DIR) test
	cp $(KERNEL_ELF) $(ISO_DIR)/boot/kernel.elf
	xorriso -as mkisofs -e boot/limine-uefi-cd.bin -no-emul-boot \
		-isohybrid-gpt-basdat -V MARAMOS $(ISO_DIR) -o $(ISO) >/dev/null 2>&1
	$(QEMU) -bios $(OVMF) -cdrom $(ISO) -machine q35 -m 512M -smp 2 \
		$(QEMU_DRIVE) \
		-serial file:serial.out -no-reboot &
	@echo ""
	@echo "=== Waiting for output ==="
	@for i in $$(seq 1 30); do \
		if [ -s serial.out ] && grep -q "Results:" serial.out 2>/dev/null; then \
			break; \
		fi; \
		sleep 1; \
	done
	@echo ""
	@echo "=== Test Results ==="
	@sed 's/\x1b\[[0-9;]*[a-zA-Z]//g; s/\x1b[=][0-9]*[a-zA-Z]//g; s/\x1b[[0-9;]*[H:]//g; s/\r//g' serial.out 2>/dev/null | tr -d '\0' > serial.clean 2>/dev/null || true
	@grep -E "(FAIL|Results:)" serial.clean 2>/dev/null || echo "(no test output in serial)"
	@if grep -q "FAIL" serial.clean 2>/dev/null; then \
		echo ""; \
		echo "SOME TESTS FAILED"; \
		exit 1; \
	elif grep -q "Results:" serial.clean 2>/dev/null; then \
		echo ""; \
		echo "ALL TESTS PASSED"; \
		exit 0; \
	else \
		echo "NO TEST OUTPUT FOUND"; \
		exit 1; \
	fi

test-gdb:
	rm -f serial.out
	rm -rf $(ISO_DIR)
	mkdir -p $(ISO_DIR)/boot $(ISO_DIR)/EFI/BOOT
	cp $(BOOT_DIR)/limine.conf $(ISO_DIR)/boot/
	cp $(BOOT_DIR)/limine-uefi-cd.bin $(ISO_DIR)/boot/
	cp $(BOOT_DIR)/BOOTX64.EFI $(ISO_DIR)/EFI/BOOT/
	$(MAKE) -C $(KERNEL_DIR) test-gdb
	cp $(KERNEL_ELF) $(ISO_DIR)/boot/kernel.elf
	xorriso -as mkisofs -e boot/limine-uefi-cd.bin -no-emul-boot \
		-isohybrid-gpt-basdat -V MARAMOS $(ISO_DIR) -o $(ISO) >/dev/null 2>&1
	@echo "Starting QEMU (background) and GDB (foreground)..."; \
	$(QEMU) -bios $(OVMF) -cdrom $(ISO) -machine q35 -m 512M -smp 2 \
		$(QEMU_DRIVE) \
		-serial file:serial.out -no-reboot -s -S & \
	PID=$$!; \
	sleep 0.2; \
	gdb $(KERNEL_ELF) -ex "target remote :1234"; \
	kill $$PID 2>/dev/null; \
	wait $$PID 2>/dev/null

clean:
	rm -f $(ISO) $(DISK_IMG) kernel.elf serial.out serial.clean
	rm -f $(USER_ELFS)
	rm -rf $(ISO_DIR)
	$(MAKE) -C $(KERNEL_DIR) clean

# === External Drive Deployment ===

deploy: $(LEMONCC) populate-img
	@echo "==> Deploying Maram OS to $(DEVICE)"
	@echo "    WARNING: This will DESTROY all data on $(DEVICE)!"
	@printf "    Continue? [y/N] "; read confirm; \
		[ "$$confirm" = "y" ] || [ "$$confirm" = "Y" ] || { echo "Aborted."; exit 1; }
	$(MAKE) partition-drive
	$(MAKE) format-drive
	$(MAKE) install-boot
	$(MAKE) write-data
	@echo "==> Deploy complete"

partition-drive:
	sudo parted -s $(DEVICE) mklabel gpt
	sudo parted -s $(DEVICE) mkpart primary fat32 1MiB 1025MiB
	sudo parted -s $(DEVICE) set 1 esp on
	sudo parted -s $(DEVICE) mkpart primary 1025MiB 1153MiB
	sudo sgdisk --typecode=2:4F4D454C-204E-4944-534B-000000000000 --change-name=2:LemonFS $(DEVICE)

format-drive:
	sudo mkfs.fat -F32 -n MARAMBOOT $(BOOT_PART)

install-boot: $(KERNEL_ELF)
	sudo mkdir -p $(MNT)
	sudo mount $(BOOT_PART) $(MNT)
	sudo mkdir -p $(MNT)/EFI/BOOT $(MNT)/boot
	sudo cp $(BOOT_DIR)/BOOTX64.EFI $(MNT)/EFI/BOOT/
	sudo cp $(BOOT_DIR)/limine.conf $(MNT)/boot/
	sudo cp $(KERNEL_ELF) $(MNT)/boot/kernel.elf
	sudo umount $(MNT)

write-data:
	@if [ ! -f $(DISK_IMG) ]; then \
		echo "Error: $(DISK_IMG) not found."; \
		echo "Run 'make disk-img' first."; \
		exit 1; \
	fi
	@echo "==> Writing LemonFS partition to $(DATA_PART)"
	@DATA_START=$$(sudo parted -s $(DISK_IMG) unit s print | grep '^ *2' | awk '{print $$2}' | tr -d 's') && \
	DATA_END=$$(sudo parted -s $(DISK_IMG) unit s print | grep '^ *2' | awk '{print $$3}' | tr -d 's') && \
	DATA_SECTORS=$$(( DATA_END - DATA_START + 1 )) && \
	TARGET_SECTORS=$$(( $$(sudo blockdev --getsize64 $(DATA_PART)) / 512 )) && \
	if [ $$DATA_SECTORS -gt $$TARGET_SECTORS ]; then \
		echo "  (truncating from $$DATA_SECTORS to $$TARGET_SECTORS sectors to fit target)"; \
		DATA_SECTORS=$$TARGET_SECTORS; \
	fi && \
	dd if=$(DISK_IMG) of=/tmp/maram_lemonfs.img bs=512 skip=$$DATA_START count=$$DATA_SECTORS status=none && \
	sudo dd if=/tmp/maram_lemonfs.img of=$(DATA_PART) bs=4M status=progress conv=fsync && \
	rm -f /tmp/maram_lemonfs.img

undeploy:
	@echo "WARNING: This will DESTROY all data on $(DEVICE)!"
	@printf "    Continue? [y/N] "; read confirm; \
		[ "$$confirm" = "y" ] || [ "$$confirm" = "Y" ] || { echo "Aborted."; exit 1; }
	sudo parted -s $(DEVICE) mklabel gpt
