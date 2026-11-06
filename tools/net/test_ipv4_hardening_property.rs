//! Property test on the *production* IPv4/ICMP/stack modules.
 //! 64 reproducible seeds in Reliability V3; no kernel mock decoder.
#![allow(dead_code)]
extern crate alloc;
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{ let _value = format!($($arg)*); }};
}
#[path = "../../src/net/internet/ipv4.rs"]
pub mod ipv4;
#[path = "../../src/net/internet/icmp.rs"]
pub mod icmp;
#[path = "../../src/net/stack.rs"]
pub mod stack;
pub mod net {
    pub use crate::{icmp, ipv4};
}

fn echo_packet() -> Vec<u8> {
    let mut icmp_buf = [0u8; 72];
    let len = icmp::build(&mut icmp_buf, icmp::ECHO_REQUEST, 8, 77, b"hello").unwrap();
    let mut out = vec![0; 256];
    let size = ipv4::build_packet(&mut out, [127, 0, 0, 1], [127, 0, 0, 1],
                                  ipv4::PROTO_ICMP, 12, &icmp_buf[..len]).unwrap();
    out.truncate(size);
    out
}

fn set_ipv4_checksum(buf: &mut [u8]) {
    let ihl = ((buf[0] & 15) as usize) * 4;
    buf[10] = 0;
    buf[11] = 0;
    let sum = ipv4::checksum(&buf[..ihl]);
    buf[10..12].copy_from_slice(&sum.to_be_bytes());
}

#[test]
fn valid_echo_still_works() {
    let packet = echo_packet();
    assert_eq!(ipv4::parse_header(&packet).unwrap().total_len, packet.len());
    let mut out = [0u8; 256];
    let size = stack::handle_ipv4(&packet, &mut out).expect("valid echo reply");
    let header = ipv4::parse_header(&out[..size]).unwrap();
    assert_eq!(header.proto, ipv4::PROTO_ICMP);
    let reply = icmp::parse(&out[header.header_len..header.total_len]).unwrap();
    assert_eq!(reply.msg_type, icmp::ECHO_REPLY);
    assert_eq!(reply.seq, 77);
}

#[test]
fn total_length_shorter_than_header_is_refused_without_panic() {
    for bad_len in [0u16, 1, 19] {
        let mut packet = echo_packet();
        packet[2..4].copy_from_slice(&bad_len.to_be_bytes());
        set_ipv4_checksum(&mut packet);
        assert!(ipv4::parse_header(&packet).is_none());
        assert!(stack::handle_ipv4(&packet, &mut [0; 256]).is_none());
    }
}

#[test]
fn total_length_beyond_available_bytes_is_refused() {
    let mut packet = echo_packet();
    packet[2..4].copy_from_slice(&600u16.to_be_bytes());
    set_ipv4_checksum(&mut packet);
    assert!(ipv4::parse_header(&packet).is_none());
    assert!(stack::handle_ipv4(&packet, &mut [0; 256]).is_none());
}

#[test]
fn invalid_header_or_icmp_checksum_fails_closed() {
    let mut packet = echo_packet();
    packet[8] ^= 1;
    assert!(ipv4::parse_header(&packet).is_none());
    let mut packet = echo_packet();
    packet[23] ^= 1; // ICMP header, IPv4 header remains valid.
    assert!(ipv4::parse_header(&packet).is_some());
    assert!(stack::handle_ipv4(&packet, &mut [0; 256]).is_none());
}

#[test]
fn fragmented_icmp_requires_reassembly_and_is_rejected() {
    let mut packet = echo_packet();
    packet[6] = 0x60; // DF + MF (fragment follows).
    set_ipv4_checksum(&mut packet);
    assert!(ipv4::parse_header(&packet).is_some());
    assert!(stack::handle_ipv4(&packet, &mut [0; 256]).is_none());
    packet[6] = 0x40;
    packet[7] = 0x01; // nonzero fragment offset.
    set_ipv4_checksum(&mut packet);
    assert!(stack::handle_ipv4(&packet, &mut [0; 256]).is_none());
}

#[test]
fn ipv4_builder_refuses_unrepresentable_packet_length() {
    let mut out = vec![0u8; 70_200];
    let data = vec![42u8; 70_000];
    assert!(ipv4::build_packet(&mut out, [1, 1, 1, 1], [2, 2, 2, 2],
                                ipv4::PROTO_TCP, 0, &data).is_none());
}

#[test]
fn randomized_untrusted_packets_never_panic_or_escape_bounds() {
    let seed = std::env::var("BOUCHAUD_PROP_SEED").ok()
        .and_then(|x| x.parse::<u64>().ok()).unwrap_or(1);
    let mut state = seed.wrapping_add(1) | 1;
    for _ in 0..1_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let len = (state as usize) % 180;
        let mut buf = vec![0u8; len];
        for b in &mut buf {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *b = state as u8;
        }
        if let Some(header) = ipv4::parse_header(&buf) {
            assert!(header.header_len >= ipv4::HEADER_LEN);
            assert!(header.total_len >= header.header_len);
            assert!(header.total_len <= buf.len());
            assert!(ipv4::somme_juste(&buf[..header.header_len]));
        }
        let _ = stack::handle_ipv4(&buf, &mut [0u8; 256]);
    }
}
