#define _GNU_SOURCE
#include <dlfcn.h>
#include <netdb.h>
#include <netinet/in.h>
#include <stddef.h>
#include <sys/socket.h>

#include "private_v4.h"

static int private_v6(const struct in6_addr *addr)
{
    return IN6_IS_ADDR_LOOPBACK(addr) || IN6_IS_ADDR_LINKLOCAL(addr) ||
           IN6_IS_ADDR_SITELOCAL(addr) || (addr->s6_addr[0] & 0xFE) == 0xFC;
}

static int private_sockaddr(const struct sockaddr *sa, socklen_t len)
{
    if (sa == NULL)
        return 0;
    if (sa->sa_family == AF_INET && len >= (socklen_t)sizeof(struct sockaddr_in))
        return private_v4(ntohl(((const struct sockaddr_in *)sa)->sin_addr.s_addr));
    if (sa->sa_family == AF_INET6 && len >= (socklen_t)sizeof(struct sockaddr_in6))
        return private_v6(&((const struct sockaddr_in6 *)sa)->sin6_addr);
    return 0;
}

int getnameinfo(const struct sockaddr *sa, socklen_t salen, char *host,
                socklen_t hostlen, char *serv, socklen_t servlen, int flags)
{
    static int (*real)(const struct sockaddr *, socklen_t, char *, socklen_t,
                       char *, socklen_t, int);

    if (real == NULL)
        real = dlsym(RTLD_NEXT, "getnameinfo");
    if (real == NULL)
        return EAI_FAIL;
    if (private_sockaddr(sa, salen))
        flags = (flags | NI_NUMERICHOST) & ~NI_NAMEREQD;
    return real(sa, salen, host, hostlen, serv, servlen, flags);
}
