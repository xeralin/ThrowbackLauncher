CC       = gcc
MINGW32  = i686-w64-mingw32-gcc
MINGW64  = x86_64-w64-mingw32-gcc
CFLAGS   = -Wall -O2
BUILD    = build

STRIP    = strip
STRIP32  = i686-w64-mingw32-strip
STRIP64  = x86_64-w64-mingw32-strip

DDK_INC := $(firstword $(wildcard \
    /usr/x86_64-w64-mingw32/include/ddk \
    /usr/x86_64-w64-mingw32/sys-root/mingw/include/ddk \
    $(HOME)/.linuxbrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/include/ddk \
    /home/linuxbrew/.linuxbrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/include/ddk \
))
ifeq ($(DDK_INC),)
DDK_INC = /usr/x86_64-w64-mingw32/include/ddk
$(warning DDK headers not found in standard paths; rvpnnetmp.sys compile may fail)
endif

BINS = $(BUILD)/tap_bridge $(BUILD)/dns_shim.so $(BUILD)/rvpnnetmp.sys $(BUILD)/adapter_hook.dll \
       $(BUILD)/rvpn_launcher.exe $(BUILD)/netsh.exe $(BUILD)/netsh64.exe \
       $(BUILD)/drvinst.exe

all: check-deps $(BINS)

$(BUILD)/tap_bridge: src/tap_bridge.c | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< -lpthread
	$(STRIP) $@

$(BUILD)/dns_shim.so: src/dns_shim.c src/private_v4.h | $(BUILD)
	$(CC) $(CFLAGS) -fPIC -shared -o $@ $< -ldl
	$(STRIP) $@

$(BUILD)/rvpnnetmp.sys: src/rvpnnetmp.c | $(BUILD)
	$(MINGW64) -Wno-multichar -shared -o $@ $< \
		-I$(DDK_INC) \
		-lntoskrnl -lhal -nostdlib -lgcc \
		-Wl,--subsystem,native -Wl,--entry,DriverEntry
	$(STRIP64) $@

$(BUILD)/adapter_hook.dll: src/adapter_hook.c src/private_v4.h | $(BUILD)
	$(MINGW32) $(CFLAGS) -shared -o $@ $< \
		-liphlpapi -lws2_32
	$(STRIP32) $@

$(BUILD)/rvpn_launcher.exe: src/rvpn_launcher.c | $(BUILD)
	$(MINGW32) $(CFLAGS) -o $@ $<
	$(STRIP32) $@

$(BUILD)/netsh.exe: src/netsh_wrapper.c | $(BUILD)
	$(MINGW32) $(CFLAGS) -o $@ $< -municode
	$(STRIP32) $@

$(BUILD)/netsh64.exe: src/netsh_wrapper.c | $(BUILD)
	$(MINGW64) $(CFLAGS) -o $@ $< -municode
	$(STRIP64) $@

$(BUILD)/drvinst.exe: src/drvinst.c | $(BUILD)
	$(MINGW32) $(CFLAGS) -o $@ $<
	$(STRIP32) $@

$(BUILD):
	mkdir -p $(BUILD)

check-deps:
	@command -v $(MINGW32) >/dev/null || { echo "Missing: $(MINGW32)"; exit 1; }
	@command -v $(MINGW64) >/dev/null || { echo "Missing: $(MINGW64)"; exit 1; }

clean:
	rm -rf $(BUILD)

.PHONY: all check-deps clean
