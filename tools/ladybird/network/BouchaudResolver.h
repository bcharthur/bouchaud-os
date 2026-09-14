#pragma once
#include <cstdio>
#include <cstring>

namespace BouchaudResolver {
// Bounded parser: no hostname resolution, allocation, or network access.
inline bool parse_ipv4(char const* text, char (&out)[16])
{
    unsigned parts[4] {};
    auto* p = text;
    for (int i = 0; i < 4; ++i) {
        int digits = 0;
        while (*p >= '0' && *p <= '9') {
            if (++digits > 3) return false;
            parts[i] = parts[i] * 10 + static_cast<unsigned>(*p++ - '0');
        }
        if (!digits || parts[i] > 255) return false;
        if (i < 3 && *p++ != '.') return false;
    }
    if (*p && *p != ' ' && *p != '\t' && *p != '\r' && *p != '\n' && *p != '#') return false;
    if (parts[0] == 0 || parts[0] >= 224) return false;
    std::snprintf(out, sizeof(out), "%u.%u.%u.%u", parts[0], parts[1], parts[2], parts[3]);
    return true;
}
inline bool read(char (&out)[16], char const* path = "/etc/resolv.conf")
{
    auto* file = std::fopen(path, "r");
    if (!file) return false;
    char line[256];
    bool found = false;
    for (int n = 0; n < 16 && std::fgets(line, sizeof(line), file); ++n) {
        char const* p = line;
        while (*p == ' ' || *p == '\t') ++p;
        if (std::strncmp(p, "nameserver", 10) != 0 || (p[10] != ' ' && p[10] != '\t')) continue;
        p += 10;
        while (*p == ' ' || *p == '\t') ++p;
        if (parse_ipv4(p, out)) { found = true; break; }
    }
    std::fclose(file);
    return found;
}
}
