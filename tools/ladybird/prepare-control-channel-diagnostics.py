#!/usr/bin/env python3
"""Attribute control-channel teardown without changing IPC behavior or budgets.

The first transport cause and errno are captured together atomically. Later
close/destruction and the loop-exit drain cannot overwrite it. Only the UI /
Compositor control transport emits records; other IPC sockets remain quiet.
Each record repeats that cause, PID, original fd and transport identity, so a
lost log line is not silently interpreted as EOF. No verdict is relaxed.
"""
import sys
from pathlib import Path


def replace(root, file, old, new):
    path = root / file
    text = path.read_text()
    if new in text:
        return
    if text.count(old) != 1:
        raise SystemExit(f"control diagnostics: ambiguous/missing anchor in {file}: {old!r}")
    path.write_text(text.replace(old, new, 1))


def main(root):
    h = "Libraries/LibIPC/TransportSocket.h"
    cpp = "Libraries/LibIPC/TransportSocket.cpp"
    replace(root, h, "    void close();\n", '''    // BOUCHAUD_CONTROL_DIAGNOSTICS_V1
    void set_control_diagnostic_side(char const* side) { m_control_side.store(side); }
    void trace_control_close(char const* event, char const* connection_reason = "transport") const;

    void close();
''')
    replace(root, h, "    NonnullOwnPtr<Core::LocalSocket> m_socket;\n", '''    enum class CloseCause : u32 {
        None, ReadEOF, ReadError, WriteError, PollHangup, PollError,
        PollInvalid, PollSyscallError, LocalClose, LocalFlushClose,
        Destructor, Transfer, ProtocolError, ResourceError,
    };
    void remember_close_cause(CloseCause, int error = 0);
    Atomic<u64> m_first_close_cause { 0 };
    Atomic<char const*> m_control_side { nullptr };
    int m_diagnostic_fd { -1 };
    NonnullOwnPtr<Core::LocalSocket> m_socket;
''')
    replace(root, cpp, "    m_socket->set_notifications_enabled(false);\n", "    m_diagnostic_fd = m_socket->fd().value();\n    m_socket->set_notifications_enabled(false);\n")
    replace(root, cpp, "void TransportSocket::wake_io_thread()\n", '''// BOUCHAUD_CONTROL_DIAGNOSTICS_V1: one atomic publication for cause + errno.
void TransportSocket::remember_close_cause(CloseCause cause, int error)
{
    u64 expected = 0;
    u64 value = (static_cast<u64>(cause) << 32) | static_cast<u32>(error);
    (void)m_first_close_cause.compare_exchange_strong(expected, value);
}

void TransportSocket::trace_control_close(char const* event, char const* connection_reason) const
{
    auto* side = m_control_side.load();
    if (!side)
        return;
    auto value = m_first_close_cause.load();
    char const* reason = "NONE";
    switch (static_cast<CloseCause>(value >> 32)) {
    case CloseCause::None: break;
    case CloseCause::ReadEOF: reason = "CONTROL_CHANNEL_EOF"; break;
    case CloseCause::ReadError: reason = "CONTROL_CHANNEL_READ_ERROR"; break;
    case CloseCause::WriteError: reason = "CONTROL_CHANNEL_WRITE_ERROR"; break;
    case CloseCause::PollHangup: reason = "CONTROL_CHANNEL_PEER_CLOSE"; break;
    case CloseCause::PollError: reason = "POLLERR"; break;
    case CloseCause::PollInvalid: reason = "POLLNVAL"; break;
    case CloseCause::PollSyscallError: reason = "POLL_SYSCALL_ERROR"; break;
    case CloseCause::LocalClose: reason = "CONTROL_CHANNEL_LOCAL_CLOSE"; break;
    case CloseCause::LocalFlushClose: reason = "LOCAL_FLUSH_CLOSE"; break;
    case CloseCause::Destructor: reason = "DESTRUCTOR"; break;
    case CloseCause::Transfer: reason = "TRANSFER"; break;
    case CloseCause::ProtocolError: reason = "PROTOCOL_ERROR"; break;
    case CloseCause::ResourceError: reason = "RESOURCE_ERROR"; break;
    }
    dbgln("[LB:CONTROL] pid={} side={} fd={} transport={:p} event={} cause={} errno={} connection={} END",
        Core::System::getpid(), side, m_diagnostic_fd, this, event, reason,
        static_cast<i32>(value & 0xffffffffu), connection_reason);
}

void TransportSocket::wake_io_thread()
''')
    replace(root, cpp, '            dbgln("TransportSocket poll error: {}", result.error());', '            remember_close_cause(CloseCause::PollSyscallError, result.error().code());\n            dbgln("TransportSocket poll error: {}", result.error());')
    replace(root, cpp, "        if (pollfds[0].revents & POLLHUP) {\n", "        if (pollfds[0].revents & POLLHUP) {\n            remember_close_cause(CloseCause::PollHangup);\n")
    replace(root, cpp, "        if (pollfds[0].revents & (POLLERR | POLLNVAL)) {\n", "        if (pollfds[0].revents & (POLLERR | POLLNVAL)) {\n            remember_close_cause((pollfds[0].revents & POLLNVAL) ? CloseCause::PollInvalid : CloseCause::PollError);\n")
    replace(root, cpp, "    VERIFY(m_io_thread_state == IOThreadState::Stopped);\n", '    VERIFY(m_io_thread_state == IOThreadState::Stopped);\n    trace_control_close("IO_STOP");\n')
    for signature, cause in [
        ("TransportSocket::~TransportSocket()", "Destructor"),
        ("void TransportSocket::close()", "LocalClose"),
        ("void TransportSocket::close_after_sending_all_pending_messages()", "LocalFlushClose"),
        ("ErrorOr<TransportHandle> TransportSocket::release_for_transfer()", "Transfer"),
    ]:
        replace(root, cpp, signature + "\n{\n", signature + f'\n{{\n    remember_close_cause(CloseCause::{cause});\n    trace_control_close("{cause}");\n')
    replace(root, cpp, "    if (auto result = send_message(*m_socket, bytes, fds); result.is_error()) {\n", "    if (auto result = send_message(*m_socket, bytes, fds); result.is_error()) {\n        remember_close_cause(CloseCause::WriteError, result.error().is_errno() ? result.error().code() : -1);\n")
    replace(root, cpp, "            if (error.is_errno() && error.code() == ECONNRESET) {\n", "            remember_close_cause(CloseCause::ReadError, error.is_errno() ? error.code() : -1);\n            if (error.is_errno() && error.code() == ECONNRESET) {\n")
    replace(root, cpp, "        if (bytes_read.is_empty() && received_fds.is_empty()) {\n", "        if (bytes_read.is_empty() && received_fds.is_empty()) {\n            remember_close_cause(CloseCause::ReadEOF);\n")
    # Every parser/resource rejection is distinct from a real recvmsg EOF.
    for anchor in [
        '            dbgln("TransportSocket: Unprocessed buffer would exceed {} bytes, disconnecting peer", MAX_UNPROCESSED_BUFFER_SIZE);',
        '            dbgln("TransportSocket: Failed to append to unprocessed_bytes buffer");',
        '            dbgln("TransportSocket: Unprocessed FDs would exceed {}, disconnecting peer", MAX_UNPROCESSED_FDS);',
        '                dbgln("TransportSocket: Rejecting message with payload_size {} exceeding limit {}", header.payload_size, MAX_MESSAGE_PAYLOAD_SIZE);',
        '                dbgln("TransportSocket: Rejecting message with fd_count {} exceeding limit {}", header.fd_count, MAX_MESSAGE_FD_COUNT);',
        '                dbgln("TransportSocket: received_fd_count would overflow");',
        '                dbgln("TransportSocket: Failed to allocate message buffer for payload_size {}", header.payload_size);',
        '                dbgln("TransportSocket: FileDescriptorAcknowledgement with non-zero payload_size {}", header.payload_size);',
        '                dbgln("TransportSocket: acknowledged_fd_count would overflow");',
        '            dbgln("TransportSocket: Unknown message header type {}", static_cast<u8>(header.type));',
        '            dbgln("TransportSocket: index would overflow");',
        '                dbgln("TransportSocket: Peer acknowledged more FDs than we sent");',
    ]:
        cause = "ResourceError" if "Failed to" in anchor else "ProtocolError"
        indent = anchor[:len(anchor) - len(anchor.lstrip())]
        replace(root, cpp, anchor, indent + f"remember_close_cause(CloseCause::{cause});\n" + anchor)
    replace(root, cpp, "    bool const peer_eof = m_peer_eof;\n", '    bool const peer_eof = m_peer_eof;\n    if (peer_eof)\n        trace_control_close("READ_EOF_PUBLISHED");\n')

    c = "Libraries/LibIPC/Connection.cpp"
    guard = "#if defined(AK_OS_LINUX)\n"
    end = "#endif\n"
    replace(root, "Libraries/LibIPC/Connection.h", "    u32 m_local_endpoint_magic { 0 };\n", '    char const* m_close_diagnostic_reason { "local_shutdown" };\n    u32 m_local_endpoint_magic { 0 };\n')
    replace(root, c, "ConnectionBase::~ConnectionBase() = default;", "ConnectionBase::~ConnectionBase()\n{\n" + guard + '    m_transport->trace_control_close("CONNECTION_DESTRUCTOR", m_close_diagnostic_reason);\n' + end + "}")
    replace(root, c, "void ConnectionBase::shutdown()\n{\n", "void ConnectionBase::shutdown()\n{\n" + guard + '    m_transport->trace_control_close("CONNECTION_SHUTDOWN", m_close_diagnostic_reason);\n' + end)
    replace(root, c, "void ConnectionBase::shutdown_with_error(Error const& error)\n{\n", 'void ConnectionBase::shutdown_with_error(Error const& error)\n{\n    m_close_diagnostic_reason = "shutdown_with_error";\n')
    replace(root, c, "    if (parse_error) {\n", '    if (parse_error) {\n        m_close_diagnostic_reason = "malformed_message";\n')
    replace(root, c, "    if (schedule_shutdown == Transport::ShouldShutdown::Yes) {\n", '    if (schedule_shutdown == Transport::ShouldShutdown::Yes) {\n        if (!parse_error)\n            m_close_diagnostic_reason = "transport_shutdown";\n' + guard + '        m_transport->trace_control_close("SHUTDOWN_SCHEDULED", m_close_diagnostic_reason);\n' + end)
    cf = "Libraries/LibIPC/ConnectionFromClient.h"
    for signature, reason in [("void did_misbehave()", "misbehaved"), ("void did_misbehave(char const* message)", "misbehaved_message"), ("virtual void shutdown_with_error(Error const& error) override", "shutdown_with_error")]:
        replace(root, cf, "    " + signature + "\n    {\n", "    " + signature + f'\n    {{\n        this->m_close_diagnostic_reason = "{reason}";\n')

    server = "Services/Compositor/ConnectionFromClient.cpp"
    replace(root, server, "    m_compositor_state->set_client(*this);\n", guard + '    m_transport->set_control_diagnostic_side("Compositor");\n    m_transport->trace_control_close("CONTROL_CHANNEL_OPEN");\n' + end + "    m_compositor_state->set_client(*this);\n")
    replace(root, server, "void ConnectionFromClient::die()\n{\n", "void ConnectionFromClient::die()\n{\n" + guard + '    m_transport->trace_control_close("COMPOSITOR_DIE_REASON", m_close_diagnostic_reason);\n' + end)
    replace(root, server, "    Core::Process::terminate_immediately(0);\n", guard + '    m_transport->trace_control_close("COMPOSITOR_EXIT", m_close_diagnostic_reason);\n' + end + "    Core::Process::terminate_immediately(0);\n")
    client = "Libraries/LibWebView/CompositorClient.cpp"
    replace(root, client, "CompositorControlServerEndpoint>(*this, move(transport))\n{\n", "CompositorControlServerEndpoint>(*this, move(transport))\n{\n" + guard + '    m_transport->set_control_diagnostic_side("UI");\n    m_transport->trace_control_close("CONTROL_CHANNEL_OPEN");\n' + end)
    replace(root, client, "void CompositorClient::die()\n{\n", "void CompositorClient::die()\n{\n" + guard + '    m_transport->trace_control_close("UI_CONTROL_DIE", m_close_diagnostic_reason);\n' + end)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: prepare-control-channel-diagnostics.py <ladybird-tree>")
    main(Path(sys.argv[1]))
