# Ladybird platform completion + SMP4 bring-up

This stage moves more responsibilities back to upstream Ladybird and adds the
first real hardware SMP bring-up proof.

## Browser platform

- persistent profile under `/persist/ladybird`;
- SQL databases and HTTP disk cache enabled by default;
- top-level site isolation enabled;
- async scrolling enabled;
- downloads under `/persist/Downloads`;
- upstream text clipboard from `WebView::Application`;
- upstream popup/fullscreen plumbing from `HeadlessWebView`;
- default timezone `Europe/Paris`;
- `/dev/dsp` advertised as audio output;
- WebDriver built and packaged;
- Bouchaud persistence enlarged to 128 MiB / 2048 entries.

## SMP4

The BSP wakes APs through Local APIC INIT/SIPI. APs execute a real-mode
trampoline, increment a shared counter and halt. Four exposed vCPUs should yield:

    SMP4_DISCOVERED count=4
    SMP4_AP_STARTED count=3
    SMP4_SCHEDULER online=1 mode=UP-pending-refactor

The last line is intentional. `kernel::task` is still a UP scheduler and cannot
be run concurrently until global task/process state, TSS/GS, interrupt routing,
run queues and TLB shootdowns are made SMP-safe.

## Remaining hard platform boundaries

- WebGL/GPU: guest GPU/OpenGL driver required;
- native sandbox: Bouchaud process sandbox required;
- geolocation/notifications: OS services required;
- rich multi-MIME clipboard: OS-global clipboard service required;
- audio: `/dev/dsp` wiring still needs runtime proof;
- user-process SMP: dedicated scheduler refactor required.
