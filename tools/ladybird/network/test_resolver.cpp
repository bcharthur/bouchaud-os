#include <cstdlib>
#include <initializer_list>
#include "BouchaudResolver.h"
#include <cassert>
#include <unistd.h>
int main()
{
    char out[16] {};
    assert(BouchaudResolver::parse_ipv4("192.168.1.254\r\n", out));
    assert(std::strcmp(out, "192.168.1.254") == 0);
    for (auto const* bad : {"", "1.2.3", "999.2.3.4", "0.0.0.0", "1.2.3.4evil", "224.1.1.1"})
        assert(!BouchaudResolver::parse_ipv4(bad, out));
    char path[] = "/tmp/bouchaud-dns-test-XXXXXX";
    int fd = mkstemp(path);
    assert(fd >= 0); close(fd);
    auto save = [&](char const* contents) {
        auto* f = std::fopen(path, "w"); assert(f);
        std::fputs(contents, f); std::fclose(f);
    };
    save("# network not configured\n");
    assert(!BouchaudResolver::read(out, path));
    save("nameserver 10.0.2.3\n");
    assert(BouchaudResolver::read(out, path));
    assert(std::strcmp(out, "10.0.2.3") == 0);
    save("  nameserver\t192.168.1.254\r\n");
    assert(BouchaudResolver::read(out, path));
    assert(std::strcmp(out, "192.168.1.254") == 0);
    save("# disconnected\n");
    assert(!BouchaudResolver::read(out, path));
    unlink(path);
}
