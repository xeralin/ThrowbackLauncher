#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <netinet/in.h>
#include <stddef.h>
#include <sys/socket.h>

int socket(int domain, int type, int protocol)
{
    static int (*real)(int, int, int);
    int fd, on = 1;

    if (real == NULL)
        real = dlsym(RTLD_NEXT, "socket");
    if (real == NULL) {
        errno = ENOSYS;
        return -1;
    }
    fd = real(domain, type, protocol);
    if (fd >= 0 && (domain == AF_INET || domain == AF_INET6) &&
        (type & ~(SOCK_NONBLOCK | SOCK_CLOEXEC)) == SOCK_STREAM &&
        (protocol == 0 || protocol == IPPROTO_TCP))
        setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &on, sizeof(on));
    return fd;
}
