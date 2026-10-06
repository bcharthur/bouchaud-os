/*
 * Bouchaud OS -- sortie audio de LibMedia sur /dev/dsp (OSS, AC'97).
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * BOUCHAUD_AUDIO_DSP_V1 (P8)
 *
 * Sans libpulse, `Meta/CMake/audio.cmake` ne choisit aucun backend et
 * `PlaybackStream::create_platform_or_null` rend toujours la sortie NULLE :
 * les medias avancent au rythme de l'horloge murale et le son est jete. Ce
 * fichier est le backend de la plateforme Bouchaud (installe dans
 * `Libraries/LibMedia/Audio/` par `tools/ladybird/prepare-audio-bouchaud.py`).
 *
 * Il reprend la machine d'etats de `NullPlaybackStream` (Playing, Draining,
 * Suspended, Underrun, Stopped ; promesses resolues sur le fil de sortie) et
 * remplace deux choses :
 *
 *   - la SORTIE : les echantillons flottants sont convertis en S16 LE
 *     entrelace, volume applique, et ECRITS sur /dev/dsp ;
 *   - l'HORLOGE : `total_time_played` vient de ce que le peripherique a
 *     reellement joue -- trames ecrites moins `SNDCTL_DSP_GETODELAY` --, pas
 *     du temps qui passe. C'est elle qui synchronise l'image sur le son.
 *
 * Le peripherique est ouvert, regle et VERIFIE avant que la promesse soit
 * resolue : une machine sans AC'97 (ENODEV au premier ioctl) rejette, et
 * LibMedia retombe d'elle-meme sur la sortie nulle -- le comportement
 * d'upstream quand PulseAudio est absent. `BOUCHAUD_DISABLE_AUDIO` force ce
 * repli pour un diagnostic.
 *
 * Le processus qui l'ouvre est WebContent (role de rendu confine) : le noyau
 * lui accorde l'ecriture de /dev/dsp et rien d'autre
 * (`src/kernel/security/chemins.rs`).
 */

#include <AK/Atomic.h>
#include <AK/Math.h>
#include <AK/ScopeGuard.h>
#include <AK/StdLibExtras.h>
#include <AK/Vector.h>
#include <LibMedia/Audio/PlaybackStream.h>
#include <LibSync/ConditionVariable.h>
#include <LibSync/Mutex.h>
#include <LibThreading/Thread.h>

#include <cerrno>
#include <cstdlib>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <unistd.h>

namespace Audio {

namespace {

// Les ioctls OSS que le noyau sert (`src/compat/linux/file.rs`).
constexpr unsigned long SNDCTL_DSP_RESET = 0x00005000;
constexpr unsigned long SNDCTL_DSP_SPEED = 0xC0045002;
constexpr unsigned long SNDCTL_DSP_SETFMT = 0xC0045005;
constexpr unsigned long SNDCTL_DSP_CHANNELS = 0xC0045006;
constexpr unsigned long SNDCTL_DSP_GETOSPACE = 0x800C500C;
constexpr unsigned long SNDCTL_DSP_GETODELAY = 0x80045017;
constexpr int AFMT_S16_LE = 0x00000010;

struct InfoTampon {
    int fragments;
    int fragstotal;
    int fragsize;
    int bytes;
};

constexpr u32 TAUX_DEMANDE = 48000;
constexpr u32 VOIES = 2;
constexpr u32 OCTETS_PAR_TRAME = VOIES * sizeof(i16);
constexpr u32 INTERVALLE_MS = 5;

}

class PlaybackStreamBouchaud final : public PlaybackStream {
public:
    class State;

    static ErrorOr<NonnullRefPtr<PlaybackStream>> create(OutputState, u32 target_latency_ms, AudioDataRequestCallback&&);

    virtual SampleSpecification sample_specification() const override;
    virtual NonnullRefPtr<Core::ThreadedPromise<AK::Duration>> resume() override;
    virtual NonnullRefPtr<Core::ThreadedPromise<void>> drain_buffer_and_suspend() override;
    virtual NonnullRefPtr<Core::ThreadedPromise<void>> discard_buffer_and_suspend() override;
    virtual void notify_data_available() override;
    virtual AK::Duration total_time_played() const override;
    virtual NonnullRefPtr<Core::ThreadedPromise<void>> set_volume(double) override;

private:
    explicit PlaybackStreamBouchaud(NonnullRefPtr<State>);
    virtual ~PlaybackStreamBouchaud() override;

    NonnullRefPtr<State> m_state;
};

class PlaybackStreamBouchaud::State final : public AtomicRefCounted<State> {
public:
    State(int fd, u32 taux, OutputState initial_output_state, u32 target_latency_ms, AudioDataRequestCallback data_request_callback)
        : m_fd(fd)
        , m_taux(taux)
        , m_data_request_callback(move(data_request_callback))
        , m_trames_cibles(max(taux / 100, taux * target_latency_ms / 1000))
        , m_state(initial_output_state == OutputState::Playing ? StreamState::Playing : StreamState::Suspended)
    {
    }

    ~State()
    {
        stop();
        if (m_fd >= 0)
            ::close(m_fd);
    }

    void start()
    {
        auto thread = MUST(Threading::Thread::try_create("Bouchaud Audio Output"sv, [self = NonnullRefPtr(*this)] {
            return self->thread_main();
        }));
        thread->start();
        thread->detach();
    }

    SampleSpecification sample_specification() const
    {
        return { m_taux, ChannelMap::stereo() };
    }

    NonnullRefPtr<Core::ThreadedPromise<AK::Duration>> resume()
    {
        auto promise = Core::ThreadedPromise<AK::Duration>::create();
        Sync::MutexLocker locker(m_mutex);
        if (m_state == StreamState::Stopped) {
            promise->reject(Error::from_string_literal("Bouchaud playback stream has stopped"));
            return promise;
        }
        m_ready_void_promises.extend(move(m_drain_promises));
        m_state = StreamState::Playing;
        m_resume_promises.append(promise);
        m_wake_condition.signal();
        return promise;
    }

    NonnullRefPtr<Core::ThreadedPromise<void>> drain_buffer_and_suspend()
    {
        auto promise = Core::ThreadedPromise<void>::create();
        Sync::MutexLocker locker(m_mutex);
        if (m_state == StreamState::Stopped) {
            promise->reject(Error::from_string_literal("Bouchaud playback stream has stopped"));
            return promise;
        }
        m_state = StreamState::Draining;
        m_drain_promises.append(promise);
        m_wake_condition.signal();
        return promise;
    }

    NonnullRefPtr<Core::ThreadedPromise<void>> discard_buffer_and_suspend()
    {
        auto promise = Core::ThreadedPromise<void>::create();
        Sync::MutexLocker locker(m_mutex);
        if (m_state == StreamState::Stopped) {
            promise->reject(Error::from_string_literal("Bouchaud playback stream has stopped"));
            return promise;
        }
        m_discard_demande = true;
        m_state = StreamState::Suspended;
        m_ready_void_promises.extend(move(m_drain_promises));
        m_ready_void_promises.append(promise);
        m_wake_condition.signal();
        return promise;
    }

    void notify_data_available()
    {
        Sync::MutexLocker locker(m_mutex);
        if (m_state != StreamState::Underrun)
            return;
        m_state = StreamState::Playing;
        m_wake_condition.signal();
    }

    AK::Duration total_time_played() const
    {
        return AK::Duration::from_time_units(m_trames_jouees.load(), 1, m_taux);
    }

    NonnullRefPtr<Core::ThreadedPromise<void>> set_volume(double volume)
    {
        auto promise = Core::ThreadedPromise<void>::create();
        Sync::MutexLocker locker(m_mutex);
        if (m_state == StreamState::Stopped) {
            promise->reject(Error::from_string_literal("Bouchaud playback stream has stopped"));
            return promise;
        }
        m_volume_millieme.store(static_cast<u32>(clamp(volume, 0.0, 1.0) * 1000.0 + 0.5));
        m_ready_void_promises.append(promise);
        m_wake_condition.signal();
        return promise;
    }

    void stop()
    {
        Sync::MutexLocker locker(m_mutex);
        if (m_state == StreamState::Stopped)
            return;
        m_state = StreamState::Stopped;
        m_ready_void_promises.extend(move(m_drain_promises));
        m_wake_condition.broadcast();
    }

private:
    enum class StreamState {
        Playing,
        Draining,
        Suspended,
        Underrun,
        Stopped,
    };

    // Trames ecrites mais pas encore jouees par le peripherique.
    u64 trames_en_vol() const
    {
        int octets = 0;
        if (::ioctl(m_fd, SNDCTL_DSP_GETODELAY, &octets) != 0 || octets < 0)
            return 0;
        return static_cast<u64>(octets) / OCTETS_PAR_TRAME;
    }

    u64 trames_libres() const
    {
        InfoTampon info {};
        if (::ioctl(m_fd, SNDCTL_DSP_GETOSPACE, &info) != 0 || info.bytes < 0)
            return 0;
        return static_cast<u64>(info.bytes) / OCTETS_PAR_TRAME;
    }

    // L'horloge : ce que le peripherique a joue. Monotone par construction --
    // un ODELAY qui remonterait (reinitialisation) ne fait pas reculer le temps.
    void releve_horloge(u64 en_vol)
    {
        auto const ecrites = m_trames_ecrites;
        auto const jouees = ecrites > en_vol ? ecrites - en_vol : 0;
        if (jouees > m_trames_jouees.load())
            m_trames_jouees.store(jouees);
    }

    // Ecrit TOUTES les trames converties : /dev/dsp peut accepter moins que
    // demande, le reste est repris.
    bool ecrit_tout(ReadonlyBytes octets)
    {
        while (!octets.is_empty()) {
            auto n = ::write(m_fd, octets.data(), octets.size());
            if (n < 0) {
                if (errno == EINTR || errno == EAGAIN) {
                    ::usleep(1000);
                    continue;
                }
                if (m_erreurs_ecriture++ == 0)
                    warnln("[LB:AUDIO] ecriture /dev/dsp echoue errno={}", errno);
                return false;
            }
            octets = octets.slice(static_cast<size_t>(n));
        }
        return true;
    }

    intptr_t thread_main()
    {
        while (true) {
            Optional<u64> trames_a_demander;
            Vector<NonnullRefPtr<Core::ThreadedPromise<AK::Duration>>> resume_promises;
            Vector<NonnullRefPtr<Core::ThreadedPromise<void>>> ready_void_promises;
            Vector<NonnullRefPtr<Core::ThreadedPromise<void>>> drain_promises;
            bool stopped = false;
            bool inactive = false;
            bool discard = false;

            auto const en_vol = trames_en_vol();
            releve_horloge(en_vol);

            {
                Sync::MutexLocker locker(m_mutex);
                resume_promises = move(m_resume_promises);
                ready_void_promises = move(m_ready_void_promises);
                discard = exchange(m_discard_demande, false);
                if (m_state == StreamState::Stopped) {
                    ready_void_promises.extend(move(m_drain_promises));
                    stopped = true;
                }
                if (!stopped && (m_state == StreamState::Suspended || m_state == StreamState::Underrun))
                    inactive = true;
                if (!stopped && !inactive) {
                    if (m_state == StreamState::Draining && en_vol == 0) {
                        m_state = StreamState::Suspended;
                        drain_promises = move(m_drain_promises);
                    } else if (m_state == StreamState::Playing && en_vol < m_trames_cibles) {
                        auto const voulu = m_trames_cibles - en_vol;
                        auto const possible = min(voulu, trames_libres());
                        if (possible > 0)
                            trames_a_demander = possible;
                    }
                }
            }

            if (discard) {
                // Ce qui est en vol ne sera pas joue : l'horloge s'arrete ou
                // elle en est, et le compte des ecrites la rejoint.
                ::ioctl(m_fd, SNDCTL_DSP_RESET, 0);
                m_trames_ecrites = m_trames_jouees.load();
            }

            for (auto& promise : resume_promises)
                promise->resolve(total_time_played());
            for (auto& promise : ready_void_promises)
                promise->resolve();
            for (auto& promise : drain_promises)
                promise->resolve();

            if (stopped) {
                warnln("[LB:AUDIO] ferme trames_ecrites={} trames_jouees={} sous_alimentations={} erreurs={}",
                    m_trames_ecrites, m_trames_jouees.load(), m_sous_alimentations, m_erreurs_ecriture);
                return 0;
            }

            if (inactive) {
                Sync::MutexLocker locker(m_mutex);
                if (m_state == StreamState::Suspended || m_state == StreamState::Underrun)
                    m_wake_condition.wait();
                continue;
            }

            if (trames_a_demander.has_value()) {
                auto const trames = trames_a_demander.value();
                m_flottants.resize(trames * VOIES);
                auto const rendus = m_data_request_callback(m_flottants.span());
                auto const trames_rendues = min(trames, static_cast<u64>(rendus.size() / VOIES));

                if (trames_rendues > 0) {
                    auto const volume = static_cast<float>(m_volume_millieme.load()) / 1000.0f;
                    m_pcm.resize(trames_rendues * VOIES);
                    for (size_t i = 0; i < m_pcm.size(); ++i) {
                        auto const v = clamp(rendus[i] * volume, -1.0f, 1.0f);
                        m_pcm[i] = static_cast<i16>(v * 32767.0f);
                    }
                    if (ecrit_tout(ReadonlyBytes { m_pcm.data(), m_pcm.size() * sizeof(i16) })) {
                        m_trames_ecrites += trames_rendues;
                        if (!m_premiere_ecriture_dite) {
                            m_premiere_ecriture_dite = true;
                            warnln("[LB:AUDIO] premiere_ecriture trames={} taux={} voies={} cible_trames={}",
                                trames_rendues, m_taux, VOIES, m_trames_cibles);
                        }
                    }
                }

                Sync::MutexLocker locker(m_mutex);
                if (m_state == StreamState::Playing && trames_rendues == 0 && en_vol == 0) {
                    m_state = StreamState::Underrun;
                    ++m_sous_alimentations;
                }
                continue;
            }

            Sync::MutexLocker locker(m_mutex);
            if (m_state == StreamState::Playing || m_state == StreamState::Draining)
                m_wake_condition.wait_for(AK::Duration::from_milliseconds(INTERVALLE_MS));
        }
    }

    int m_fd { -1 };
    u32 m_taux { TAUX_DEMANDE };
    AudioDataRequestCallback m_data_request_callback;
    u64 m_trames_cibles { 0 };

    mutable Sync::Mutex m_mutex;
    Sync::ConditionVariable m_wake_condition { m_mutex };
    StreamState m_state { StreamState::Suspended };
    bool m_discard_demande { false };
    Vector<NonnullRefPtr<Core::ThreadedPromise<AK::Duration>>> m_resume_promises;
    Vector<NonnullRefPtr<Core::ThreadedPromise<void>>> m_drain_promises;
    Vector<NonnullRefPtr<Core::ThreadedPromise<void>>> m_ready_void_promises;

    // Propres au fil de sortie.
    Vector<float> m_flottants;
    Vector<i16> m_pcm;
    u64 m_trames_ecrites { 0 };
    u64 m_sous_alimentations { 0 };
    u64 m_erreurs_ecriture { 0 };
    bool m_premiere_ecriture_dite { false };

    Atomic<u64> m_trames_jouees { 0 };
    Atomic<u32> m_volume_millieme { 1000 };
};

ErrorOr<NonnullRefPtr<PlaybackStream>> PlaybackStreamBouchaud::create(OutputState initial_output_state, u32 target_latency_ms, AudioDataRequestCallback&& data_request_callback)
{
    if (getenv("BOUCHAUD_DISABLE_AUDIO"))
        return Error::from_string_literal("audio desactive (BOUCHAUD_DISABLE_AUDIO)");

    int fd = ::open("/dev/dsp", O_WRONLY | O_CLOEXEC);
    if (fd < 0)
        return Error::from_syscall("open /dev/dsp"sv, errno);

    ArmedScopeGuard ferme_fd = [fd] { ::close(fd); };

    // Chaque reglage est relu : le peripherique repond ce qu'il fera
    // reellement, et un AC'97 absent repond ENODEV des le premier.
    auto regle = [fd](unsigned long requete, int voulu, StringView nom) -> ErrorOr<int> {
        int valeur = voulu;
        if (::ioctl(fd, requete, &valeur) != 0)
            return Error::from_syscall(nom, errno);
        return valeur;
    };
    auto const format = TRY(regle(SNDCTL_DSP_SETFMT, AFMT_S16_LE, "SNDCTL_DSP_SETFMT"sv));
    auto const voies = TRY(regle(SNDCTL_DSP_CHANNELS, VOIES, "SNDCTL_DSP_CHANNELS"sv));
    auto const taux = TRY(regle(SNDCTL_DSP_SPEED, TAUX_DEMANDE, "SNDCTL_DSP_SPEED"sv));
    if (format != AFMT_S16_LE || voies != static_cast<int>(VOIES) || taux <= 0) {
        warnln("[LB:AUDIO] refuse format={} voies={} taux={}", format, voies, taux);
        return Error::from_string_literal("/dev/dsp ne sert pas S16 LE stereo");
    }

    ferme_fd.disarm();
    auto state = make_ref_counted<State>(fd, static_cast<u32>(taux), initial_output_state, target_latency_ms, move(data_request_callback));
    warnln("[LB:AUDIO] /dev/dsp ouvert taux={} voies={} latence_ms={} pid={}", taux, voies, target_latency_ms, getpid());
    state->start();
    return adopt_ref<PlaybackStream>(*new PlaybackStreamBouchaud(move(state)));
}

PlaybackStreamBouchaud::PlaybackStreamBouchaud(NonnullRefPtr<State> state)
    : m_state(move(state))
{
}

PlaybackStreamBouchaud::~PlaybackStreamBouchaud()
{
    m_state->stop();
}

SampleSpecification PlaybackStreamBouchaud::sample_specification() const { return m_state->sample_specification(); }
NonnullRefPtr<Core::ThreadedPromise<AK::Duration>> PlaybackStreamBouchaud::resume() { return m_state->resume(); }
NonnullRefPtr<Core::ThreadedPromise<void>> PlaybackStreamBouchaud::drain_buffer_and_suspend() { return m_state->drain_buffer_and_suspend(); }
NonnullRefPtr<Core::ThreadedPromise<void>> PlaybackStreamBouchaud::discard_buffer_and_suspend() { return m_state->discard_buffer_and_suspend(); }
void PlaybackStreamBouchaud::notify_data_available() { m_state->notify_data_available(); }
AK::Duration PlaybackStreamBouchaud::total_time_played() const { return m_state->total_time_played(); }
NonnullRefPtr<Core::ThreadedPromise<void>> PlaybackStreamBouchaud::set_volume(double volume) { return m_state->set_volume(volume); }

NonnullRefPtr<PlaybackStream::CreatePromise> PlaybackStream::create_platform_playback_stream(OutputState initial_output_state, u32 target_latency_ms, AudioDataRequestCallback&& data_request_callback)
{
    auto promise = CreatePromise::construct();
    auto stream = PlaybackStreamBouchaud::create(initial_output_state, target_latency_ms, move(data_request_callback));
    if (stream.is_error()) {
        warnln("[LB:AUDIO] sortie /dev/dsp indisponible : {} -- repli sur la sortie nulle", stream.error());
        promise->reject(stream.release_error());
    } else {
        promise->resolve(stream.release_value());
    }
    return promise;
}

}
