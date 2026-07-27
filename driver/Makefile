obj-m := legion_hwmon.o

KERNEL_DIR ?= /lib/modules/$(shell uname -r)/build
PWD := $(shell pwd)

all:
	$(MAKE) -C $(KERNEL_DIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KERNEL_DIR) M=$(PWD) clean

install: all
	$(MAKE) -C $(KERNEL_DIR) M=$(PWD) modules_install
	depmod -a

dkms-install:
	cp -r $(PWD)/.. /usr/src/legion-hwmon-0.1
	dkms add legion-hwmon/0.1
	dkms build legion-hwmon/0.1
	dkms install legion-hwmon/0.1

.PHONY: all clean install dkms-install
