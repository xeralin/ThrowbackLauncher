#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <iphlpapi.h>
#include <string.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>

#define TAP_DESC     L"radminvpn0"
#define RADMIN_DESC  L"Famatech Radmin VPN Ethernet Adapter"
#define RADMIN_FRIENDLY L"Radmin VPN"

static DWORD g_jmp_tls = TLS_OUT_OF_INDEXES;
static void longjmp_tramp(void);

static LONG CALLBACK crash_handler(PEXCEPTION_POINTERS ep)
{
    DWORD code = ep->ExceptionRecord->ExceptionCode;

    if ((code & 0xC0000000) != 0xC0000000)
        return EXCEPTION_CONTINUE_SEARCH;
    if (code == 0xE06D7363u)
        return EXCEPTION_CONTINUE_SEARCH;

    if (code == 0xC0000005u && g_jmp_tls != TLS_OUT_OF_INDEXES &&
        TlsGetValue(g_jmp_tls)) {
        ep->ContextRecord->Eip = (DWORD)(ULONG_PTR)&longjmp_tramp;
        return EXCEPTION_CONTINUE_EXECUTION;
    }

    return EXCEPTION_CONTINUE_SEARCH;
}

#define DISPATCH_RVA 0x636A0
#define FREE_RVA     0xC9001
#define COUNTER_RVA  0x13F1D8

static BYTE *g_rvbase = NULL;

static void longjmp_tramp(void)
{
    if (g_jmp_tls != TLS_OUT_OF_INDEXES) {
        void **e = (void **)TlsGetValue(g_jmp_tls);
        if (e)
            __builtin_longjmp(e, 1);
    }
    for (;;)
        Sleep(1000);
}

static unsigned __stdcall hook_dispatch(void **Block)
{
    if (Block) {
        void *fn  = Block[0];
        void *arg = Block[1];

        ((void (__cdecl *)(void *, unsigned))(g_rvbase + FREE_RVA))(Block, 8);
        if (fn) {
            void *env[5];
            void **prev;
            if (g_jmp_tls != TLS_OUT_OF_INDEXES) {
                prev = (void **)TlsGetValue(g_jmp_tls);
                TlsSetValue(g_jmp_tls, env);
                if (__builtin_setjmp(env) == 0)
                    ((void (__stdcall *)(void *))fn)(arg);
                TlsSetValue(g_jmp_tls, prev);
            } else {
                ((void (__stdcall *)(void *))fn)(arg);
            }
        }

        _InterlockedDecrement((long *)(g_rvbase + COUNTER_RVA));
    }
    return 0;
}

#define GETINETWORK_RVA 0x5BA70

static void install_release_guard(void)
{

    static const BYTE expect[5] = { 0x55, 0x8B, 0xEC, 0x6A, 0xFF };
    HMODULE base = GetModuleHandleA(NULL);
    BYTE *t;
    DWORD old;

    if (!base)
        return;
    g_rvbase = (BYTE *)base;

    {
        static const BYTE gi_expect[3] = { 0x56, 0x57, 0x8B };
        BYTE *g = (BYTE *)base + GETINETWORK_RVA;
        DWORD go;
        if (memcmp(g, gi_expect, sizeof(gi_expect)) == 0 &&
            VirtualProtect(g, 3, PAGE_EXECUTE_READWRITE, &go)) {
            g[0] = 0x30; g[1] = 0xC0;
            g[2] = 0xC3;
            VirtualProtect(g, 3, go, &go);
            FlushInstructionCache(GetCurrentProcess(), g, 3);
        }
    }

    t = (BYTE *)base + DISPATCH_RVA;
    if (memcmp(t, expect, sizeof(expect)) != 0)
        return;
    if (!VirtualProtect(t, 6, PAGE_EXECUTE_READWRITE, &old))
        return;

    t[0] = 0x68;
    *(DWORD *)(t + 1) = (DWORD)(ULONG_PTR)hook_dispatch;
    t[5] = 0xC3;
    VirtualProtect(t, 6, old, &old);
    FlushInstructionCache(GetCurrentProcess(), t, 6);
}

static ULONG (WINAPI *real_GetAdaptersAddresses)(
    ULONG, ULONG, PVOID, PIP_ADAPTER_ADDRESSES, PULONG) = NULL;
static ULONG (WINAPI *real_GetAdaptersInfo)(PIP_ADAPTER_INFO, PULONG) = NULL;

static DWORD g_tap_ifindex = 0;

static BOOL is_uplink_v4(DWORD s_addr_net)
{
    DWORD h = ntohl(s_addr_net);
    if (h == 0) return FALSE;
    if ((h & 0xFF000000u) == 0x7F000000u) return FALSE;
    if ((h & 0xFFFF0000u) == 0xA9FE0000u) return FALSE;
    if ((h & 0xFFC00000u) == 0x64400000u) return FALSE;
    return TRUE;
}

static DWORD default_route_ifindex(void)
{
    if (!real_GetAdaptersAddresses) {
        DWORD idx = 0;
        if (GetBestInterface(inet_addr("8.8.8.8"), &idx) == NO_ERROR && idx)
            return idx;
        return 0;
    }

    ULONG size = 15 * 1024;
    IP_ADAPTER_ADDRESSES *buf = (IP_ADAPTER_ADDRESSES *)malloc(size);
    if (!buf) return 0;
    ULONG r = real_GetAdaptersAddresses(AF_INET, GAA_FLAG_INCLUDE_GATEWAYS,
                                        NULL, buf, &size);
    if (r == ERROR_BUFFER_OVERFLOW) {
        IP_ADAPTER_ADDRESSES *nb = (IP_ADAPTER_ADDRESSES *)realloc(buf, size);
        if (!nb) { free(buf); return 0; }
        buf = nb;
        r = real_GetAdaptersAddresses(AF_INET, GAA_FLAG_INCLUDE_GATEWAYS,
                                      NULL, buf, &size);
    }
    if (r != ERROR_SUCCESS) { free(buf); return 0; }

    DWORD best = 0;
    if (GetBestInterface(inet_addr("8.8.8.8"), &best) == NO_ERROR && best) {
        for (IP_ADAPTER_ADDRESSES *a = buf; a; a = a->Next) {
            if (a->IfIndex == best) { free(buf); return best; }
        }
    }

    DWORD found = 0;
    int n = 0;
    for (IP_ADAPTER_ADDRESSES *a = buf; a; a = a->Next) {
        if (a->OperStatus != IfOperStatusUp) continue;
        if (!a->FirstGatewayAddress) continue;
        BOOL has_v4 = FALSE;
        for (PIP_ADAPTER_UNICAST_ADDRESS u = a->FirstUnicastAddress; u; u = u->Next) {
            if (u->Address.lpSockaddr &&
                u->Address.lpSockaddr->sa_family == AF_INET &&
                is_uplink_v4(((struct sockaddr_in *)
                              u->Address.lpSockaddr)->sin_addr.s_addr)) {
                has_v4 = TRUE;
                break;
            }
        }
        if (!has_v4) continue;
        n++;
        found = a->IfIndex;
    }
    free(buf);
    return n == 1 ? found : 0;
}

static ULONG WINAPI hook_GetAdaptersAddresses(
    ULONG Family, ULONG Flags, PVOID Rsvd,
    PIP_ADAPTER_ADDRESSES Addrs, PULONG Size)
{
    if (!real_GetAdaptersAddresses) return ERROR_NOT_SUPPORTED;

    ULONG ret = real_GetAdaptersAddresses(Family, Flags, Rsvd, Addrs, Size);
    if (ret != ERROR_SUCCESS || !Addrs) return ret;

    DWORD best = default_route_ifindex();
    BOOL  best_ok = FALSE;

    for (PIP_ADAPTER_ADDRESSES cur = Addrs; cur; cur = cur->Next) {
        if (cur->Description && wcscmp(cur->Description, TAP_DESC) == 0) {

            cur->Description  = (WCHAR *)RADMIN_DESC;
            cur->FriendlyName = (WCHAR *)RADMIN_FRIENDLY;
            g_tap_ifindex = cur->IfIndex;
            continue;
        }
        if (best && cur->IfIndex == best)
            best_ok = TRUE;
    }

    if (!best || !best_ok)
        return ret;

    for (PIP_ADAPTER_ADDRESSES cur = Addrs; cur; cur = cur->Next) {
        if (cur->IfIndex == best || cur->IfIndex == g_tap_ifindex)
            continue;
        cur->FirstUnicastAddress = NULL;
    }
    return ret;
}

static ULONG WINAPI hook_GetAdaptersInfo(PIP_ADAPTER_INFO Info, PULONG Size)
{
    if (!real_GetAdaptersInfo) return ERROR_NOT_SUPPORTED;

    ULONG ret = real_GetAdaptersInfo(Info, Size);
    if (ret != ERROR_SUCCESS || !Info) return ret;

    DWORD best = default_route_ifindex();
    BOOL  best_ok = FALSE;
    for (PIP_ADAPTER_INFO cur = Info; cur; cur = cur->Next)
        if (best && cur->Index == best) { best_ok = TRUE; break; }

    if (!best || !best_ok)
        return ret;

    for (PIP_ADAPTER_INFO cur = Info; cur; cur = cur->Next) {
        if (cur->Index == best || (g_tap_ifindex && cur->Index == g_tap_ifindex))
            continue;

        strcpy(cur->IpAddressList.IpAddress.String, "0.0.0.0");
        cur->IpAddressList.Next = NULL;
    }
    return ret;
}

#define WS2_ORD_GETHOSTBYNAME 52
#define ALLOWED_MAX           32

static DWORD g_allowed_v4[ALLOWED_MAX];
static int   g_allowed_n     = 0;
static BOOL  g_allowed_valid = FALSE;

static struct hostent *(WINAPI *real_gethostbyname)(const char *) = NULL;
static int (WINAPI *real_getaddrinfo)(const char *, const char *,
    const struct addrinfo *, struct addrinfo **) = NULL;

static BOOL is_cgnat(DWORD s_addr_net)
{
    return (ntohl(s_addr_net) & 0xFFC00000u) == 0x64400000u;
}

static BOOL in_allowed_v4(DWORD s_addr_net)
{
    for (int i = 0; i < g_allowed_n; i++)
        if (g_allowed_v4[i] == s_addr_net) return TRUE;
    return FALSE;
}

static void build_allowed_v4(void)
{
    g_allowed_valid = FALSE;
    g_allowed_n = 0;

    if (!real_GetAdaptersAddresses) return;
    DWORD uplink = default_route_ifindex();
    if (!uplink) return;

    ULONG size = 15 * 1024;
    IP_ADAPTER_ADDRESSES *buf = (IP_ADAPTER_ADDRESSES *)malloc(size);
    if (!buf) return;
    ULONG r = real_GetAdaptersAddresses(AF_INET, 0, NULL, buf, &size);
    if (r == ERROR_BUFFER_OVERFLOW) {
        IP_ADAPTER_ADDRESSES *nb = (IP_ADAPTER_ADDRESSES *)realloc(buf, size);
        if (!nb) { free(buf); return; }
        buf = nb;
        r = real_GetAdaptersAddresses(AF_INET, 0, NULL, buf, &size);
    }
    if (r != ERROR_SUCCESS) { free(buf); return; }

    BOOL uplink_has_v4 = FALSE;
    for (IP_ADAPTER_ADDRESSES *a = buf; a; a = a->Next) {
        if (a->IfIndex != uplink && a->IfIndex != g_tap_ifindex) continue;
        for (PIP_ADAPTER_UNICAST_ADDRESS u = a->FirstUnicastAddress; u; u = u->Next) {
            if (!u->Address.lpSockaddr ||
                u->Address.lpSockaddr->sa_family != AF_INET) continue;
            DWORD ip = ((struct sockaddr_in *)u->Address.lpSockaddr)->sin_addr.s_addr;
            if (a->IfIndex == uplink) {
                uplink_has_v4 = TRUE;
                if (is_cgnat(ip)) { free(buf); return; }
            }
            if (g_allowed_n < ALLOWED_MAX) g_allowed_v4[g_allowed_n++] = ip;
        }
    }
    free(buf);
    if (!uplink_has_v4) return;
    g_allowed_valid = TRUE;
}

static void norm_host(char *dst, size_t dstsz, const char *src)
{
    size_t i = 0;
    for (; src[i] && i + 1 < dstsz; i++)
        dst[i] = src[i];
    while (i && dst[i - 1] == '.') i--;
    dst[i] = 0;
}

static BOOL is_local_hostname(const char *name)
{
    if (!name || !*name) return FALSE;
    char host[256];
    if (gethostname(host, sizeof host) != 0) return FALSE;
    char a[256], b[256];
    norm_host(a, sizeof a, name);
    norm_host(b, sizeof b, host);
    if (_stricmp(a, b) == 0) return TRUE;
    char *da = strchr(a, '.'); if (da) *da = 0;
    char *db = strchr(b, '.'); if (db) *db = 0;
    return _stricmp(a, b) == 0;
}

static struct hostent *WINAPI hook_gethostbyname(const char *name)
{
    struct hostent *h = real_gethostbyname ? real_gethostbyname(name) : NULL;
    if (!h || h->h_addrtype != AF_INET || !h->h_addr_list) return h;
    if (!is_local_hostname(name)) return h;
    build_allowed_v4();
    if (!g_allowed_valid) {
        h->h_addr_list[0] = NULL;
        return h;
    }

    char **rd = h->h_addr_list, **wr = h->h_addr_list;
    for (; *rd; rd++) {
        if (in_allowed_v4(*(DWORD *)*rd)) *wr++ = *rd;
    }
    *wr = NULL;
    return h;
}

static int WINAPI hook_getaddrinfo(const char *node, const char *service,
    const struct addrinfo *hints, struct addrinfo **res)
{
    int rc = real_getaddrinfo ? real_getaddrinfo(node, service, hints, res)
                              : EAI_FAIL;
    if (rc != 0 || !res || !*res) return rc;
    if (!is_local_hostname(node)) return rc;
    build_allowed_v4();
    if (!g_allowed_valid) return rc;

    int keep = 0;
    for (struct addrinfo *c = *res; c; c = c->ai_next)
        if (c->ai_family == AF_INET && c->ai_addr &&
            in_allowed_v4(((struct sockaddr_in *)c->ai_addr)->sin_addr.s_addr))
            keep++;
    if (keep == 0) return rc;

    struct addrinfo *cur = *res, *prev = NULL, *drop = NULL;
    while (cur) {
        BOOL ok = (cur->ai_family == AF_INET && cur->ai_addr &&
                   in_allowed_v4(((struct sockaddr_in *)cur->ai_addr)->sin_addr.s_addr));
        struct addrinfo *next = cur->ai_next;
        if (ok) {
            prev = cur;
        } else {
            if (prev) prev->ai_next = next; else *res = next;
            cur->ai_next = drop;
            drop = cur;
        }
        cur = next;
    }
    if (drop) freeaddrinfo(drop);
    return rc;
}

static int (WSAAPI *real_getnameinfo)(const SOCKADDR *, socklen_t,
    PCHAR, DWORD, PCHAR, DWORD, INT) = NULL;

static BOOL is_private_v4(DWORD s_addr_net)
{
    DWORD h = ntohl(s_addr_net);
    if ((h & 0xFF000000u) == 0x7F000000u) return TRUE;
    if ((h & 0xFF000000u) == 0x0A000000u) return TRUE;
    if ((h & 0xFFF00000u) == 0xAC100000u) return TRUE;
    if ((h & 0xFFFF0000u) == 0xC0A80000u) return TRUE;
    if ((h & 0xFFFF0000u) == 0xA9FE0000u) return TRUE;
    if ((h & 0xFFC00000u) == 0x64400000u) return TRUE;
    return FALSE;
}

static int WSAAPI hook_getnameinfo(const SOCKADDR *sa, socklen_t salen,
    PCHAR host, DWORD hostlen, PCHAR serv, DWORD servlen, INT flags)
{
    if (sa && sa->sa_family == AF_INET &&
        salen >= (socklen_t)sizeof(struct sockaddr_in)) {
        const struct sockaddr_in *si = (const struct sockaddr_in *)sa;
        if (is_private_v4(si->sin_addr.s_addr)) {
            char ip[16];
            lstrcpynA(ip, inet_ntoa(si->sin_addr), sizeof ip);
            if (host && hostlen) lstrcpynA(host, ip, hostlen);
            if (serv && servlen) {
                char pb[8];
                snprintf(pb, sizeof pb, "%u", (unsigned)ntohs(si->sin_port));
                lstrcpynA(serv, pb, servlen);
            }
            return 0;
        }
    }
    return real_getnameinfo
        ? real_getnameinfo(sa, salen, host, hostlen, serv, servlen, flags)
        : EAI_FAIL;
}

static LONG (WINAPI *real_RegSetKeySecurity)(HKEY hKey, SECURITY_INFORMATION si,
    PSECURITY_DESCRIPTOR psd) = NULL;

static LONG WINAPI hook_RegSetKeySecurity(HKEY hKey, SECURITY_INFORMATION si,
    PSECURITY_DESCRIPTOR psd)
{
    (void)hKey; (void)si; (void)psd;
    return ERROR_SUCCESS;
}

static BOOL hook_import(HMODULE mod, const char *dll, const char *fn, WORD ord,
                        void *newfn, void **saved)
{
    if (!mod) return FALSE;
    PIMAGE_DOS_HEADER dos = (PIMAGE_DOS_HEADER)mod;
    if (dos->e_magic != IMAGE_DOS_SIGNATURE) return FALSE;
    PIMAGE_NT_HEADERS nt = (PIMAGE_NT_HEADERS)((BYTE *)mod + dos->e_lfanew);
    if (nt->Signature != IMAGE_NT_SIGNATURE) return FALSE;
    DWORD rva = nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].VirtualAddress;
    if (!rva) return FALSE;

    for (PIMAGE_IMPORT_DESCRIPTOR imp =
            (PIMAGE_IMPORT_DESCRIPTOR)((BYTE *)mod + rva); imp->Name; imp++) {
        if (_stricmp((char *)mod + imp->Name, dll) != 0)
            continue;
        PIMAGE_THUNK_DATA orig  = (PIMAGE_THUNK_DATA)((BYTE *)mod + imp->OriginalFirstThunk);
        PIMAGE_THUNK_DATA thunk = (PIMAGE_THUNK_DATA)((BYTE *)mod + imp->FirstThunk);
        for (; orig->u1.AddressOfData; orig++, thunk++) {
            BOOL match;
            if (orig->u1.Ordinal & IMAGE_ORDINAL_FLAG) {

                match = (fn == NULL) &&
                        (IMAGE_ORDINAL(orig->u1.Ordinal) == ord);
            } else {
                if (fn == NULL) continue;
                PIMAGE_IMPORT_BY_NAME bn =
                    (PIMAGE_IMPORT_BY_NAME)((BYTE *)mod + orig->u1.AddressOfData);
                match = (strcmp(bn->Name, fn) == 0);
            }
            if (!match) continue;
            if (saved) *saved = (void *)thunk->u1.Function;
            DWORD old;
            if (!VirtualProtect(&thunk->u1.Function, sizeof(DWORD_PTR), PAGE_READWRITE, &old))
                return FALSE;
            thunk->u1.Function = (DWORD_PTR)newfn;
            VirtualProtect(&thunk->u1.Function, sizeof(DWORD_PTR), old, &old);
            return TRUE;
        }
    }
    return FALSE;
}

static void hook_iphlpapi(HMODULE mod)
{
    hook_import(mod, "IPHLPAPI.DLL", "GetAdaptersAddresses", 0,
                hook_GetAdaptersAddresses, (void **)&real_GetAdaptersAddresses);
    hook_import(mod, "IPHLPAPI.DLL", "GetAdaptersInfo", 0,
                hook_GetAdaptersInfo, (void **)&real_GetAdaptersInfo);
}

static void hook_ws2_32(HMODULE mod)
{
    hook_import(mod, "WS2_32.DLL", NULL, WS2_ORD_GETHOSTBYNAME,
                hook_gethostbyname, (void **)&real_gethostbyname);
    hook_import(mod, "WS2_32.DLL", "getaddrinfo", 0,
                hook_getaddrinfo, (void **)&real_getaddrinfo);
    hook_import(mod, "WS2_32.DLL", "getnameinfo", 0,
                hook_getnameinfo, (void **)&real_getnameinfo);
}

static volatile LONG g_rol_patched = 0;

static void try_patch_rol(void)
{
    if (g_rol_patched) return;
    HMODULE rol = GetModuleHandleW(L"RvROLClient.dll");
    if (!rol) return;
    if (InterlockedExchange(&g_rol_patched, 1) != 0) return;
    hook_iphlpapi(rol);
    hook_ws2_32(rol);
}

static HMODULE (WINAPI *real_LoadLibraryW)(LPCWSTR) = NULL;
static HMODULE (WINAPI *real_LoadLibraryA)(LPCSTR) = NULL;
static HMODULE (WINAPI *real_LoadLibraryExW)(LPCWSTR, HANDLE, DWORD) = NULL;

static HMODULE WINAPI hook_LoadLibraryW(LPCWSTR name)
{
    HMODULE h = real_LoadLibraryW ? real_LoadLibraryW(name) : NULL;
    try_patch_rol();
    return h;
}
static HMODULE WINAPI hook_LoadLibraryA(LPCSTR name)
{
    HMODULE h = real_LoadLibraryA ? real_LoadLibraryA(name) : NULL;
    try_patch_rol();
    return h;
}
static HMODULE WINAPI hook_LoadLibraryExW(LPCWSTR name, HANDLE file, DWORD flags)
{
    HMODULE h = real_LoadLibraryExW ? real_LoadLibraryExW(name, file, flags) : NULL;
    try_patch_rol();
    return h;
}

static DWORD WINAPI rol_watch_thread(LPVOID unused)
{
    (void)unused;
    for (int i = 0; i < 600 && !g_rol_patched; i++) {
        try_patch_rol();
        Sleep(100);
    }
    return 0;
}

BOOL WINAPI DllMain(HINSTANCE inst, DWORD reason, LPVOID reserved)
{
    (void)inst; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH) {

        AddVectoredExceptionHandler(1, crash_handler);

        g_jmp_tls = TlsAlloc();
        install_release_guard();

        HMODULE exe = GetModuleHandle(NULL);
        if (!exe)
            return TRUE;

        hook_iphlpapi(exe);
        hook_ws2_32(exe);
        hook_import(exe, "ADVAPI32.DLL", "RegSetKeySecurity", 0,
                    hook_RegSetKeySecurity,  (void **)&real_RegSetKeySecurity);
        hook_import(exe, "KERNEL32.dll", "LoadLibraryW", 0,
                    hook_LoadLibraryW,       (void **)&real_LoadLibraryW);
        hook_import(exe, "KERNEL32.dll", "LoadLibraryA", 0,
                    hook_LoadLibraryA,       (void **)&real_LoadLibraryA);
        hook_import(exe, "KERNEL32.dll", "LoadLibraryExW", 0,
                    hook_LoadLibraryExW,     (void **)&real_LoadLibraryExW);

        try_patch_rol();
        { HANDLE t = CreateThread(NULL, 0, rol_watch_thread, NULL, 0, NULL);
          if (t) CloseHandle(t); }
    }
    return TRUE;
}
