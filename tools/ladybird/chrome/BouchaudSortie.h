#pragma once

#include <cerrno>
#include <cstddef>
#include <cstring>

namespace BouchaudTransport {

// File bornee de messages complets. Le moteur ne dort jamais en attendant
// le compositeur. Une ecriture partielle reprend a son offset, dans l'ordre.
template<size_t Taille = 4112, size_t Capacite = 16>
class Sortie {
public:
    bool ajoute(void const* donnees, size_t taille)
    {
        if (m_erreur || taille == 0 || taille > Taille || m_nombre == Capacite)
            return false;
        auto& message = m_messages[(m_tete + m_nombre) % Capacite];
        std::memcpy(message.octets, donnees, taille);
        message.taille = taille;
        message.offset = 0;
        ++m_nombre;
        return true;
    }

    template<typename Ecrit>
    bool vide(Ecrit ecrit)
    {
        if (m_erreur)
            return false;
        // Meme un ecrivain qui n'accepte qu'un octet ne monopolise pas le tour.
        for (size_t budget = 0; budget < Capacite && m_nombre != 0; ++budget) {
            auto& message = m_messages[m_tete];
            auto restant = message.taille - message.offset;
            auto n = ecrit(message.octets + message.offset, restant);
            if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR))
                return true;
            if (n <= 0 || static_cast<size_t>(n) > restant) {
                m_erreur = true;
                return false;
            }
            message.offset += static_cast<size_t>(n);
            if (message.offset == message.taille) {
                m_tete = (m_tete + 1) % Capacite;
                --m_nombre;
            }
        }
        return true;
    }

    size_t nombre() const { return m_nombre; }

private:
    struct Message {
        unsigned char octets[Taille] {};
        size_t taille { 0 };
        size_t offset { 0 };
    };
    Message m_messages[Capacite] {};
    size_t m_tete { 0 };
    size_t m_nombre { 0 };
    bool m_erreur { false };
};

}
