#include <windows.h>
#include <stdio.h>

int main(int argc, char *argv[])
{
    char exe_dir[MAX_PATH];
    char cmdline[1024];
    char dll_path_buf[MAX_PATH];
    int i;
    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi = {0};

    GetModuleFileNameA(NULL, exe_dir, MAX_PATH);
    { char *p = strrchr(exe_dir, '\\'); if (p) *p = '\0'; }

    snprintf(cmdline, sizeof(cmdline), "\"%s\\RvControlSvc.exe\"", exe_dir);

    for (i = 1; i < argc; i++) {
        size_t current_len = strlen(cmdline);
        size_t arg_len = strlen(argv[i]);
        size_t space_needed = arg_len + 1;

        if (current_len + space_needed >= sizeof(cmdline))
            return 1;

        strcat(cmdline, " ");
        strcat(cmdline, argv[i]);
    }

    if (!CreateProcessA(NULL, cmdline, NULL, NULL, TRUE,
                        CREATE_SUSPENDED, NULL, exe_dir, &si, &pi))
        return 1;

    {
        int dll_path_len = snprintf(dll_path_buf, sizeof(dll_path_buf), "%s\\adapter_hook.dll", exe_dir);
        if (dll_path_len < 0 || (size_t)dll_path_len >= sizeof(dll_path_buf))
            goto skip_inject;
        const char *dll_path = dll_path_buf;
        SIZE_T path_len = strlen(dll_path) + 1;
        LPVOID remote_str;
        HANDLE remote_thread;
        FARPROC loadlib;

        remote_str = VirtualAllocEx(pi.hProcess, NULL, path_len,
                                     MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if (!remote_str) goto skip_inject;

        WriteProcessMemory(pi.hProcess, remote_str, dll_path, path_len, NULL);

        loadlib = GetProcAddress(GetModuleHandleA("kernel32.dll"), "LoadLibraryA");
        if (!loadlib) { VirtualFreeEx(pi.hProcess, remote_str, 0, MEM_RELEASE); goto skip_inject; }

        remote_thread = CreateRemoteThread(pi.hProcess, NULL, 0,
                                           (LPTHREAD_START_ROUTINE)loadlib,
                                           remote_str, 0, NULL);
        if (remote_thread) {
            WaitForSingleObject(remote_thread, 5000);
            CloseHandle(remote_thread);
        }

        VirtualFreeEx(pi.hProcess, remote_str, 0, MEM_RELEASE);
    }
skip_inject:

    ResumeThread(pi.hThread);

    WaitForSingleObject(pi.hProcess, INFINITE);

    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return 0;
}
