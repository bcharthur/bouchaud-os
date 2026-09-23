/*
 * SORTIE-IMMEDIATE : la plus petite cible possible pour un `execve`.
 *
 * Elle ne sert qu'a une chose : etre la NOUVELLE IMAGE d'un processus qui
 * vient de faire `fork`. C'est le geste que `Core::Process::spawn` de Ladybird
 * repete pour chacun des six services du navigateur, et le chemin par lequel
 * `sys_execve` -- donc le chargeur ELF, la pile initiale, la quiescence des
 * autres coeurs et la LIBERATION de l'espace duplique -- est reellement pris.
 *
 * Elle ecrit une ligne puis sort. Tout ce qui est mesure autour d'elle est
 * donc du noyau : le programme lui-meme ne coute rien.
 */
static long appel(long n, long a, long b, long c) {
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return r;
}

void _start(void) {
    char message[] = "COUT_FORK image remplacee\n";
    appel(1, 1, (long)message, sizeof(message) - 1);
    appel(60, 0, 0, 0);
    for (;;) {}
}
