#define _GNU_SOURCE
#include <sched.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

struct partage {
    _Atomic int tour;
    uint64_t temps[2];
    int cpu[2];
    unsigned distincts, reculs;
};
static uint64_t ns(void)
{
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) _exit(2);
    return (uint64_t)t.tv_sec * 1000000000 + t.tv_nsec;
}
static int cpu_noyau(void)
{
    unsigned cpu;
    if (syscall(SYS_getcpu, &cpu, NULL, NULL) != 0)
        _exit(4);
    return (int)cpu;
}

static void echange(struct partage *p, int moi)
{
    for (int i = 0; i < 20000; ++i) {
        while (atomic_load_explicit(&p->tour, memory_order_acquire) != moi)
            sched_yield();
        int avant, apres;
        uint64_t t;
        do {
            avant = cpu_noyau();
            t = ns();
            apres = cpu_noyau();
        } while (avant < 0 || avant != apres);
        int autre = 1 - moi;
        if ((i != 0 || moi != 0) && p->cpu[autre] != avant) {
            ++p->distincts;
            if (t < p->temps[autre])
                ++p->reculs;
        }
        p->cpu[moi] = avant;
        p->temps[moi] = t;
        atomic_store_explicit(&p->tour, autre, memory_order_release);
    }
}
int main(void)
{
    struct partage *p = mmap(NULL, sizeof *p, PROT_READ | PROT_WRITE,
                             MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) return 1;
    pid_t fils = fork();
    if (fils == 0) {
        echange(p, 1);
        _exit(0);
    }
    if (fils < 0) return 1;
    echange(p, 0);
    int statut;
    if (waitpid(fils, &statut, 0) != fils || !WIFEXITED(statut) || WEXITSTATUS(statut)) return 2;
    printf("HORLOGE_COEURS distincts=%u reculs=%u dernier=%d:%d\n", p->distincts, p->reculs, p->cpu[0], p->cpu[1]);
    if (p->distincts == 0 || p->reculs != 0)
        return 3;
    puts("HORLOGE_COEURS_OK");
    return 0;
}
