/*
 * healthd-defaillant -- a critical service that never becomes ready
 * (BOUCHAUD_SERVICED_SYSTEM_V1, negative bench only).
 *
 * Installed as /bin/healthd in a CANDIDATE slot by tools/ci/run_ab_rollback.sh
 * (scenario sante-ko): it exits 1 before answering "hello". serviced restarts
 * it within its crash budget, declares it Failed, enters RecoveryRequired and
 * never confirms the boot -- so the loader rolls the candidate back.
 * Never shipped in an image.
 */
#include "bouchaud/sdk.h"

int bo_main(const struct bo_start *start)
{
    (void)start;
    bo_say("[HEALTHD-DEFAILLANT] exit before ready\n");
    return 1;
}
