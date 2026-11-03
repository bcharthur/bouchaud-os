/*
 * bouchaud-sdk runtime: process entry, bootstrap handles, console helpers.
 *
 * The kernel enters an ELF image at e_entry with the System V initial stack:
 *
 *     rsp -> argc, argv[0..argc), NULL, envp[...], NULL, auxv...
 *
 * There is no return address, so a naked _start aligns the stack and hands
 * the original rsp to bo__start_c.
 *
 * Bootstrap handles: bo_process_spawn installs the transferred handles in
 * the child's table before its first instruction, and names them in the
 * environment as BO_BOOTSTRAP=<id>,<id>,... (decimal). The kernel, not the
 * program, chooses the ids; the ORDER is the order the parent passed.
 */
#include "bouchaud/sdk.h"

static int starts_with(const char *s, const char *prefix)
{
    while (*prefix) {
        if (*s++ != *prefix++) return 0;
    }
    return 1;
}

static void parse_bootstrap(struct bo_start *start, const char *value)
{
    bo_u64 current = 0;
    int digits = 0;
    for (;; value++) {
        char c = *value;
        if (c >= '0' && c <= '9') {
            current = current * 10 + (bo_u64)(c - '0');
            digits++;
            continue;
        }
        if (digits && start->bootstrap_count < BO_START_MAX_BOOTSTRAP) {
            start->bootstrap[start->bootstrap_count++] = current;
        }
        current = 0;
        digits = 0;
        if (c == 0) break;
    }
}

__attribute__((used, noreturn))
void bo__start_c(bo_u64 *sp)
{
    struct bo_start start;
    int i;

    start.argc = (int)sp[0];
    start.argv = (char **)(sp + 1);
    start.envp = start.argv + start.argc + 1;
    start.bootstrap_count = 0;
    for (i = 0; i < BO_START_MAX_BOOTSTRAP; i++) start.bootstrap[i] = 0;

    for (i = 0; start.envp[i]; i++) {
        if (starts_with(start.envp[i], "BO_BOOTSTRAP=")) {
            parse_bootstrap(&start, start.envp[i] + 13);
        }
    }

    bo_exit(bo_main(&start));
}

__attribute__((naked, used, noreturn))
void _start(void)
{
    __asm__ volatile(
        "mov %rsp, %rdi\n\t"
        "andq $-16, %rsp\n\t"
        "call bo__start_c\n\t"
        "ud2\n\t");
}

__attribute__((noreturn))
void bo_exit(int code)
{
    (void)bo__sys6(231 /* exit_group */, (bo_u64)(bo_i64)code, 0, 0, 0, 0, 0);
    for (;;) __asm__ volatile("pause");
}

void bo_put_u64(bo_u64 v)
{
    char b[24];
    int i = 23;
    b[i] = 0;
    if (v == 0) b[--i] = '0';
    while (v) {
        b[--i] = (char)('0' + (v % 10));
        v /= 10;
    }
    bo_puts(&b[i]);
}

void bo_put_i64(bo_i64 v)
{
    if (v < 0) {
        bo_puts("-");
        bo_put_u64((bo_u64)(-(v + 1)) + 1);
    } else {
        bo_put_u64((bo_u64)v);
    }
}

bo_i64 bo_sleep_ns(bo_u64 ns)
{
    static bo_i64 idle = 0;
    struct bo_wait_event unused;
    bo_i64 r;

    if (idle <= 0) {
        idle = bo_waitset_create();
        if (idle < 0) return idle;
    }
    r = bo_waitset_wait((bo_handle_t)idle, &unused, 1, ns);
    return r == -BO_E_TIMED_OUT ? 0 : r;
}

/* The compiler may lower struct/array initialisation to these even with
 * -fno-builtin; a freestanding image must provide them. */
void *memset(void *dst, int c, bo_usize n)
{
    unsigned char *d = (unsigned char *)dst;
    while (n--) *d++ = (unsigned char)c;
    return dst;
}

void *memcpy(void *dst, const void *src, bo_usize n)
{
    unsigned char *d = (unsigned char *)dst;
    const unsigned char *s = (const unsigned char *)src;
    while (n--) *d++ = *s++;
    return dst;
}
