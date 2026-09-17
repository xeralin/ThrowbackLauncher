#include <windows.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

#define NETSH_CMD_DELAY_MS    1500

static int valid_ipv4(const char *s) {
    int octets = 0;
    while (octets < 4) {
        int val = 0, digits = 0;
        while (*s >= '0' && *s <= '9') { val = val * 10 + (*s - '0'); digits++; s++; if (val > 255) return 0; }
        if (digits == 0) return 0;
        octets++;
        if (octets < 4) { if (*s != '.') return 0; s++; }
    }
    return *s == '\0';
}

static char *to_narrow(const WCHAR *w) {
    int len = WideCharToMultiByte(CP_UTF8, 0, w, -1, NULL, 0, NULL, NULL);
    char *buf = malloc(len + 1);
    if (!buf) return NULL;
    WideCharToMultiByte(CP_UTF8, 0, w, -1, buf, len + 1, NULL, NULL);
    return buf;
}

int wmain(int argc, WCHAR *argv[])
{
    char cmdline[2048] = "";
    int i;
    char *addr = NULL, *mask = NULL;

    for (i = 1; i < argc; i++) {
        char *a = to_narrow(argv[i]);
        if (!a) continue;

        size_t current_len = strlen(cmdline);
        size_t arg_len = strlen(a);
        size_t space_needed = arg_len + 1;

        if (i > 1) space_needed += 1;

        if (current_len + space_needed >= sizeof(cmdline)) {
            free(a);
            break;
        }

        if (i > 1) strcat(cmdline, " ");
        strcat(cmdline, a);
        free(a);
    }

    if (strstr(cmdline, "interface") &&
        (strstr(cmdline, "add address") || strstr(cmdline, "set address"))) {
        char *p;

        if (strstr(cmdline, "set address")) {
            p = strstr(cmdline, "address=");
            if (p) { p = strchr(p, '=') + 1; addr = p; }
        } else {
            p = strstr(cmdline, "addr=");
            if (!p) p = strstr(cmdline, "address=");
            if (p) { p = strchr(p, '=') + 1; addr = p; }
        }
        if (addr) {
            char *end = strchr(addr, ' ');
            if (end) *end = '\0';
        }

        p = strstr(cmdline, "mask=");
        if (p) {
            p = strchr(p, '=') + 1;
            mask = p;
            char *end = strchr(p, ' ');
            if (end) *end = '\0';
        }

        if (addr) {

            const char *cidr = "8";
            if (mask) {
                if (strcmp(mask, "255.0.0.0") == 0) cidr = "8";
                else if (strcmp(mask, "255.255.0.0") == 0) cidr = "16";
                else if (strcmp(mask, "255.255.255.0") == 0) cidr = "24";
            }

            if (strstr(addr, "fe80")) {
                return 0;
            }

            if (!valid_ipv4(addr)) {
                return 0;
            }

            {
                FILE *f = _wfopen(L"Z:\\tmp\\rvpn_netsh_cmd", L"a");
                if (f) {
                    fprintf(f, "ip addr add %s/%s dev radminvpn0\n", addr, cidr);
                    fclose(f);
                    Sleep(NETSH_CMD_DELAY_MS);
                }
            }
            return 0;
        }
    }

    return 0;
}
