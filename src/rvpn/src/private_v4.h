#ifndef RVPN_PRIVATE_V4_H
#define RVPN_PRIVATE_V4_H

#include <stdint.h>

static inline int private_v4(uint32_t host)
{
    return (host & 0xFF000000u) == 0x7F000000u ||
           (host & 0xFF000000u) == 0x0A000000u ||
           (host & 0xFFF00000u) == 0xAC100000u ||
           (host & 0xFFFF0000u) == 0xC0A80000u ||
           (host & 0xFFFF0000u) == 0xA9FE0000u ||
           (host & 0xFFC00000u) == 0x64400000u;
}

#endif
