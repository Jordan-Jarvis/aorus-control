obj-m += aorus-als.o

KDIR ?= /lib/modules/$(shell uname -r)/build

.PHONY: all check clean

all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules

check: all
	@test -f aorus-als.ko
	@modinfo ./aorus-als.ko >/dev/null
	@echo "built and inspected aorus-als.ko; nothing was installed or loaded"

clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) clean
