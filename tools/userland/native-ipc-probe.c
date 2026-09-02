#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "include/bouchaud/native.h"

static int check(long result, const char *what)
{
    if (result < 0) {
        fprintf(stderr, "ECHEC %s: %ld\n", what, result);
        return 0;
    }
    return 1;
}

int main(void)
{
    long version = bo_version();
    if (!check(version, "version")) return 1;
    printf("[NATIVE-IPC] ABI=%ld.%ld\n", (version >> 16) & 0xffff, version & 0xffff);

    uint64_t pair[2] = {0, 0};
    if (!check(bo_channel_create(pair), "channel_create")) return 2;

    const char ping[] = "ping-bouchaud";
    if (!check(bo_channel_send(pair[0], ping, sizeof(ping), 0, 0), "channel_send")) return 3;

    char recv[64] = {0};
    struct bo_recv_meta meta = {0, 0};
    long got = bo_channel_recv(pair[1], recv, sizeof(recv), 0, 0, &meta);
    if (!check(got, "channel_recv")) return 4;
    if (meta.bytes != sizeof(ping) || memcmp(recv, ping, sizeof(ping)) != 0) return 5;

    long event = bo_event_create(0);
    if (!check(event, "event_create") || !check(bo_event_signal((uint64_t)event), "event_signal")) return 6;

    long shm = bo_shm_create(4096);
    if (!check(shm, "shm_create")) return 7;
    const char shared[] = "shared-region-ok";
    if (!check(bo_shm_write((uint64_t)shm, 128, shared, sizeof(shared)), "shm_write")) return 8;
    memset(recv, 0, sizeof(recv));
    if (!check(bo_shm_read((uint64_t)shm, 128, recv, sizeof(shared)), "shm_read")) return 9;
    if (memcmp(recv, shared, sizeof(shared)) != 0) return 10;

    puts("[NATIVE-IPC] OK");
    return 0;
}
