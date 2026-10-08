/* One in-flight sendmsg: its single byte and SCM_RIGHTS must be delivered together.
 * The receiving thread races arrival on an initially empty nonblocking socket.
 * Neither endpoint closes until all rounds have completed. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#define ROUNDS 10000
static int sockets[2], attachment;
static atomic_int request, stop, send_error;
static long long deadline;
static long long now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}
static void *sender(void *unused) {
    (void)unused;
    for (int round = 1; round <= ROUNDS; ++round) {
        while (atomic_load(&request) != round) {
            if (atomic_load(&stop) || now_ms() >= deadline) return NULL;
            sched_yield();
        }
        char byte = (char)(round % 251);
        struct iovec iov = {.iov_base = &byte, .iov_len = 1};
        union { struct cmsghdr align; char bytes[CMSG_SPACE(sizeof(int))]; } control = {0};
        struct msghdr msg = {.msg_iov = &iov, .msg_iovlen = 1,
            .msg_control = control.bytes, .msg_controllen = sizeof(control.bytes)};
        struct cmsghdr *c = CMSG_FIRSTHDR(&msg);
        c->cmsg_level = SOL_SOCKET; c->cmsg_type = SCM_RIGHTS; c->cmsg_len = CMSG_LEN(sizeof(int));
        memcpy(CMSG_DATA(c), &attachment, sizeof(int));
        ssize_t n;
        do { n = sendmsg(sockets[0], &msg, MSG_NOSIGNAL); } while (n < 0 && errno == EINTR);
        if (n != 1) { atomic_store(&send_error, n < 0 ? errno : -1); return NULL; }
    }
    return NULL;
}
int main(void) {
    if (socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, sockets) != 0) return 2;
    attachment = open("/dev/null", O_RDONLY | O_CLOEXEC);
    if (attachment < 0) return 2;
    deadline = now_ms() + 30000;
    pthread_t thread;
    if (pthread_create(&thread, NULL, sender, NULL) != 0) return 2;
    int completed = 0, failed = 0;
    for (int round = 1; round <= ROUNDS; ++round) {
        atomic_store(&request, round);
        for (;;) {
            char byte = 0;
            struct iovec iov = {.iov_base = &byte, .iov_len = 1};
            union { struct cmsghdr align; char bytes[CMSG_SPACE(sizeof(int) * 4)]; } control = {0};
            struct msghdr msg = {.msg_iov = &iov, .msg_iovlen = 1,
                .msg_control = control.bytes, .msg_controllen = sizeof(control.bytes)};
            ssize_t n = recvmsg(sockets[1], &msg, MSG_DONTWAIT | MSG_CMSG_CLOEXEC);
            int error = n < 0 ? errno : 0, count = 0;
            if (n >= 0) {
                for (struct cmsghdr *c = CMSG_FIRSTHDR(&msg); c; c = CMSG_NXTHDR(&msg, c)) {
                    if (c->cmsg_level != SOL_SOCKET || c->cmsg_type != SCM_RIGHTS || c->cmsg_len < CMSG_LEN(0)) continue;
                    size_t nfds = (c->cmsg_len - CMSG_LEN(0)) / sizeof(int);
                    for (size_t j = 0; j < nfds; ++j) {
                        int fd; memcpy(&fd, (char *)CMSG_DATA(c) + j * sizeof(int), sizeof(int));
                        if (fcntl(fd, F_GETFD) < 0) failed = 1;
                        close(fd); ++count;
                    }
                }
                if (n != 1 || count != 1 || byte != (char)(round % 251) || (msg.msg_flags & MSG_CTRUNC) || failed) {
                    printf("SCM_RECEIVE_RACE_FAIL round=%d bytes=%ld rights=%d errno=%d flags=%d peer_open=1 END\n", round, (long)n, count, error, msg.msg_flags);
                    failed = 1;
                } else ++completed;
                break;
            }
            if ((error != EAGAIN && error != EINTR) || now_ms() >= deadline || atomic_load(&send_error)) {
                printf("SCM_RECEIVE_RACE_FAIL round=%d bytes=%ld errno=%d send_errno=%d timeout=%d END\n", round, (long)n, error, atomic_load(&send_error), now_ms() >= deadline);
                failed = 1; break;
            }
        }
        if (failed) break;
    }
    atomic_store(&stop, 1);
    pthread_join(thread, NULL);
    close(attachment); close(sockets[0]); close(sockets[1]);
    printf("SCM_RECEIVE_RACE_%s rounds=%d expected=%d END\n", failed ? "FAIL" : "OK", completed, ROUNDS);
    return failed;
}
