obj-m += legion_ec_wmi.o

KERNEL_DIR ?= /lib/modules/$(shell uname -r)/build
PWD := $(shell pwd)

all:
	$(MAKE) -C $(KERNEL_DIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KERNEL_DIR) M=$(PWD) clean

test:
	sudo dmesg -C
	sudo insmod legion_ec_wmi.ko
	sudo dmesg | tail -20
	sudo cat /sys/kernel/debug/legion_ec/sensors
	sudo rmmod legion_ec_wmi

.PHONY: all clean test
