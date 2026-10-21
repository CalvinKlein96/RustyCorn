//! Frame building/validation and OPC payload encoding.
//!
//! HDLC-like sliding-window link layer (window = 1, SN mod 4) over RS-232; every
//! frame is printable ASCII: `# <addr> <ctrl> <payload…> <checksum> CR`.
//! Indices are 0-based: the checksum of a frame covers the bytes from index 1
//! (after '#'), i.e. `compute_checksum(buf, 1, n)`.

use rc_core::opc::OpcMessage;

// ── Frame byte constants ──────────────────────────────────────────────────────
/// '#'
pub const FRAME_START: u8 = 0x23;
/// '0'
pub const PC_ADDR: u8 = 0x30;
/// '1'
pub const AKTA_ADDR: u8 = 0x31;
/// CR
pub const FRAME_END: u8 = 0x0d;

// ── Control byte constants ────────────────────────────────────────────────────
pub const CTRL_POLL: u8 = 0x53;
pub const CTRL_CONN: u8 = 0x75;
pub const CTRL_RR_BASE: u8 = 0x30;
pub const CTRL_IFRAME: u8 = 0x40;

/// Hex dump plus printable ASCII, for debug logs: `23 30 53 35 0d  |#0S5.|`.
pub fn fmt_frame(buf: &[u8]) -> String {
    let hex: Vec<String> = buf.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}  |{}|", hex.join(" "), printable(buf))
}

/// Bytes as ASCII with everything outside 0x20–0x7e shown as '.'.
pub(crate) fn printable(buf: &[u8]) -> String {
    buf.iter()
        .map(|&b| {
            if (0x20..=0x7e).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

/// Checksum over `len` bytes from `offset` (0-based): a wrapping UInt8 sum `s`,
/// then `((s + (s >> 6)) & 0x3f) + 0x30`.
pub fn compute_checksum(buf: &[u8], offset: usize, len: usize) -> u8 {
    let s = buf[offset..offset + len]
        .iter()
        .fold(0u8, |s, &b| s.wrapping_add(b));
    ((s as u32 + (s >> 6) as u32) & 0x3f) as u8 + 0x30
}

/// A complete frame from the Akta: start, Akta address, CR at the end and a
/// checksum over everything between '#' and the checksum byte.
pub fn validate_rx_frame(buf: &[u8]) -> bool {
    let len = buf.len();
    if len < 5 {
        return false;
    }
    if buf[0] != FRAME_START || buf[1] != AKTA_ADDR || buf[len - 1] != FRAME_END {
        return false;
    }
    buf[len - 2] == compute_checksum(buf, 1, len - 3)
}

/// SNRM / poll: `#0S5\r`.
pub fn build_poll_frame() -> Vec<u8> {
    let mut buf = vec![FRAME_START, PC_ADDR, CTRL_POLL, 0x00, FRAME_END];
    buf[3] = compute_checksum(&buf, 1, 2);
    buf
}

/// Receive-ready: acknowledges the Akta's I-frame number `rn` (the SN of the
/// last frame received, not the next one expected as in standard HDLC).
pub fn build_ack_frame(rn: u8) -> Vec<u8> {
    let mut buf = vec![
        FRAME_START,
        PC_ADDR,
        CTRL_RR_BASE | (rn & 0x03),
        0x00,
        FRAME_END,
    ];
    buf[3] = compute_checksum(&buf, 1, 2);
    buf
}

pub fn build_i_frame(payload: &[u8], sn: u8, rn: u8) -> Vec<u8> {
    let plen = payload.len();
    let mut buf = Vec::with_capacity(plen + 5);
    buf.push(FRAME_START);
    buf.push(PC_ADDR);
    buf.push(((0x10 | (sn & 0x03)) << 2) | (rn & 0x03));
    buf.extend_from_slice(payload);
    buf.push(compute_checksum(&buf, 1, plen + 2));
    buf.push(FRAME_END);
    buf
}

/// Encode one OPC message for an I-frame payload. Binary OPCs (bit 15 clear)
/// are packed 6 bits per character, 3 characters per 2 bytes; ASCII OPCs are
/// sent as they are, with bytes that would break a frame replaced by spaces.
pub fn encode_opc_payload(opc: u16, data: &[u8]) -> Vec<u8> {
    let nob = data.len();
    let mut out = Vec::with_capacity(4 + nob * 3 / 2 + 2);
    let binary = (opc & 0x8000) == 0;
    // The first character is the number of encoded data characters + '0'.
    let count = if binary {
        ((nob >> 1) + 0x10) * 3 + (nob & 1) * 2
    } else {
        nob + 0x30
    };
    if count > 0xff {
        // The count does not fit in its byte; it is written truncated (and logged).
        // No message the PC sends comes close.
        log::warn!(
            "encode_opc_payload: OPC 0x{opc:04x} with {nob} bytes is too long for one message"
        );
    }
    out.push(count as u8);
    out.push(((opc & 0x003f) + 0x30) as u8);
    out.push((((opc >> 6) & 0x3f) + 0x30) as u8);
    out.push(((opc >> 12) + 0x30) as u8);
    if binary {
        for pair in data.chunks(2) {
            match *pair {
                [b0, b1] => {
                    out.push((b0 & 0x3f) + 0x30);
                    out.push((((b1 & 0x0f) << 2) | (b0 >> 6)) + 0x30);
                    out.push((b1 >> 4) + 0x30);
                }
                [b0] => {
                    out.push((b0 & 0x3f) + 0x30);
                    out.push((b0 >> 6) + 0x30);
                }
                _ => unreachable!(),
            }
        }
    } else {
        for &b in data {
            if !(0x20..=0x7f).contains(&b) || b == FRAME_START {
                log::warn!("encode_opc_payload: byte 0x{b:x} out of ASCII range");
                out.push(0x20);
            } else {
                out.push(b);
            }
        }
    }
    out
}

/// Decode an I-frame payload into its OPC messages. Stops at the first header
/// that does not look like one; a truncated last message keeps what arrived.
pub fn decode_opc_payload(enc: &[u8]) -> Vec<OpcMessage> {
    let mut results = Vec::new();
    let mut pos = 0usize;
    let mut remaining = enc.len();
    while remaining > 0 {
        if remaining < 4 {
            break;
        }
        let nob_byte = enc[pos];
        if nob_byte < 0x30 {
            break;
        }
        let nob_chars = (nob_byte - 0x30) as usize;
        let (b1, b2, b3) = (enc[pos + 1], enc[pos + 2], enc[pos + 3]);
        if !(0x30..=0x6f).contains(&b1)
            || !(0x30..=0x6f).contains(&b2)
            || !(0x30..=0x3f).contains(&b3)
        {
            break;
        }
        let opc = (b1 - 0x30) as u16 | (((b2 - 0x30) as u16) << 6) | (((b3 - 0x30) as u16) << 12);
        pos += 4;
        remaining -= 4;
        let mut data = Vec::new();
        if (opc & 0x8000) == 0 {
            // Characters outside '0'…'o' are not valid in a binary payload; they
            // cannot pass the checksum in practice, so rather than dropping the
            // connection they are simply truncated to a byte.
            let c = |i: usize| enc[i] as i32 - 0x30;
            let mut chars_left = nob_chars;
            while chars_left > 0 && remaining > 0 {
                if chars_left >= 3 && remaining >= 3 {
                    let (c0, c1, c2) = (c(pos), c(pos + 1), c(pos + 2));
                    data.push((c0 | ((c1 & 0x03) << 6)) as u8);
                    data.push((((c1 >> 2) & 0x0f) | (c2 << 4)) as u8);
                    pos += 3;
                    remaining -= 3;
                    chars_left -= 3;
                } else if chars_left >= 2 && remaining >= 2 {
                    let (c0, c1) = (c(pos), c(pos + 1));
                    data.push((c0 | ((c1 & 0x03) << 6)) as u8);
                    pos += 2;
                    remaining -= 2;
                    chars_left -= 2;
                } else {
                    data.push((c(pos) & 0x3f) as u8);
                    pos += 1;
                    remaining -= 1;
                    chars_left -= 1;
                }
            }
        } else {
            let take = nob_chars.min(remaining);
            data.extend_from_slice(&enc[pos..pos + take]);
            pos += take;
            remaining -= take;
        }
        results.push((opc, data));
    }
    results
}

/// Little-endian u16 at `i` (0-based); 0 if too short.
pub(crate) fn u16_le(data: &[u8], i: usize) -> u16 {
    match data.get(i..i + 2) {
        Some(&[lo, hi]) => lo as u16 | ((hi as u16) << 8),
        _ => 0,
    }
}

/// Little-endian unsigned integer of `n` bytes from `i` (0-based).
pub(crate) fn le_uint(data: &[u8], i: usize, n: usize) -> u32 {
    data[i..i + n]
        .iter()
        .rev()
        .fold(0u32, |acc, &b| (acc << 8) | b as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_core::opc::*;

    // Known-good values for the frame and payload encoders and decoders,
    // byte for byte.

    #[test]
    fn fmt_frame_matches_reference() {
        assert_eq!(
            fmt_frame(&[0x23, 0x30, 0x53, 0x35, 0x0d]),
            "23 30 53 35 0d  |#0S5.|"
        );
        assert_eq!(
            fmt_frame(&[0x00, 0x7e, 0x7f, 0x20, 0x41, 0xff]),
            "00 7e 7f 20 41 ff  |.~. A.|"
        );
    }

    #[test]
    fn poll_and_ack_frames() {
        assert_eq!(build_poll_frame(), b"#0S5\r");
        let acks: [[u8; 5]; 6] = [
            [0x23, 0x30, 0x30, 0x51, 0x0d],
            [0x23, 0x30, 0x31, 0x52, 0x0d],
            [0x23, 0x30, 0x32, 0x53, 0x0d],
            [0x23, 0x30, 0x33, 0x54, 0x0d],
            [0x23, 0x30, 0x30, 0x51, 0x0d],
            [0x23, 0x30, 0x31, 0x52, 0x0d],
        ];
        for (rn, want) in acks.iter().enumerate() {
            assert_eq!(build_ack_frame(rn as u8), want, "rn={rn}");
        }
    }

    #[test]
    fn checksum_worked_examples() {
        // PROTOCOL.md: SNRM '5', UA 'X'
        assert_eq!(compute_checksum(&[0x23, 0x30, 0x53], 1, 2), b'5');
        assert_eq!(compute_checksum(&[0x23, 0x31, 0x75], 1, 2), b'X');
        // wrapping UInt8 sum: 3 × 0xff = 0xfd; (0xfd + 3) & 0x3f = 0
        assert_eq!(compute_checksum(&[0xff, 0xff, 0xff], 0, 3), 0x30);
        assert_eq!(compute_checksum(&[], 0, 0), 0x30);
    }

    fn encode_cases() -> Vec<(u16, Vec<u8>, Vec<u8>)> {
        vec![
            (0x1001, vec![0x01], vec![0x32, 0x31, 0x30, 0x31, 0x31, 0x30]),
            (
                (0x2001),
                (0..14).collect(),
                vec![
                    0x45, 0x31, 0x30, 0x32, 0x30, 0x34, 0x30, 0x32, 0x3c, 0x30, 0x34, 0x44, 0x30,
                    0x36, 0x4c, 0x30, 0x38, 0x54, 0x30, 0x3a, 0x5c, 0x30, 0x3c, 0x64, 0x30,
                ],
            ),
            (
                0x8004,
                b"Flow 0.5 ml/min".to_vec(),
                [b"?408".as_slice(), b"Flow 0.5 ml/min"].concat(),
            ),
            (
                0x2016,
                [vec![3, 0, 5, 0, 12, 0], vec![0xff; 14]].concat(),
                vec![
                    0x4e, 0x46, 0x30, 0x32, 0x33, 0x30, 0x30, 0x35, 0x30, 0x30, 0x3c, 0x30, 0x30,
                    0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f,
                    0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f,
                ],
            ),
            (0x2013, vec![], vec![0x30, 0x43, 0x30, 0x32]),
            (
                0x8008,
                b"Max|2,zy3 zz".to_vec(),
                [b"<808".as_slice(), b"Max|2,zy3 zz"].concat(),
            ),
            // '#', LF, 0x80, 0xff and 0x1f would break a frame: sent as spaces; 0x7f passes
            (
                0x8003,
                vec![0x41, 0x23, 0x0a, 0x7f, 0x80, 0xff, 0x20, 0x1f],
                vec![
                    0x38, 0x33, 0x30, 0x38, 0x41, 0x20, 0x20, 0x7f, 0x20, 0x20, 0x20, 0x20,
                ],
            ),
            (
                0x2002,
                vec![0xff, 0xfe, 0x80, 0x7f, 0x00, 0x01, 0x40, 0xc0],
                vec![
                    0x3c, 0x32, 0x30, 0x32, 0x6f, 0x6b, 0x3f, 0x30, 0x6e, 0x37, 0x30, 0x34, 0x30,
                    0x30, 0x31, 0x3c,
                ],
            ),
            (
                0x2003,
                vec![0x9a, 0x02, 0x31, 0x00, 0xc4, 0x09, 0x00, 0x00, 0x01, 0x01],
                vec![
                    0x3f, 0x33, 0x30, 0x32, 0x4a, 0x3a, 0x30, 0x61, 0x30, 0x30, 0x34, 0x57, 0x30,
                    0x30, 0x30, 0x30, 0x31, 0x34, 0x30,
                ],
            ),
            (
                0xfffd,
                b"dbg~}".to_vec(),
                vec![0x35, 0x6d, 0x6f, 0x3f, 0x64, 0x62, 0x67, 0x7e, 0x7d],
            ),
            (
                0x2001,
                vec![0xff; 13],
                vec![
                    0x44, 0x31, 0x30, 0x32, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f,
                    0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f, 0x33,
                ],
            ),
            (0x8002, vec![], vec![0x30, 0x32, 0x30, 0x38]),
        ]
    }

    #[test]
    fn encode_matches_reference() {
        for (opc, data, want) in encode_cases() {
            assert_eq!(encode_opc_payload(opc, &data), want, "opc 0x{opc:04x}");
        }
    }

    #[test]
    fn i_frames_match_reference() {
        let cases = encode_cases();
        let reference: Vec<(usize, u8, u8, Vec<u8>)> = vec![
            (
                0,
                0,
                0,
                vec![
                    0x23, 0x30, 0x40, 0x32, 0x31, 0x30, 0x31, 0x31, 0x30, 0x47, 0x0d,
                ],
            ),
            (
                0,
                1,
                0,
                vec![
                    0x23, 0x30, 0x44, 0x32, 0x31, 0x30, 0x31, 0x31, 0x30, 0x4b, 0x0d,
                ],
            ),
            (
                0,
                3,
                2,
                vec![
                    0x23, 0x30, 0x4e, 0x32, 0x31, 0x30, 0x31, 0x31, 0x30, 0x55, 0x0d,
                ],
            ),
            (
                0,
                2,
                3,
                vec![
                    0x23, 0x30, 0x4b, 0x32, 0x31, 0x30, 0x31, 0x31, 0x30, 0x52, 0x0d,
                ],
            ),
            (
                2,
                1,
                1,
                [b"#0E?408Flow 0.5 ml/min".as_slice(), &[0x37, 0x0d]].concat(),
            ),
            (
                4,
                0,
                0,
                vec![0x23, 0x30, 0x40, 0x30, 0x43, 0x30, 0x32, 0x36, 0x0d],
            ),
            (
                3,
                7,
                5,
                vec![
                    0x23, 0x30, 0x4d, 0x4e, 0x46, 0x30, 0x32, 0x33, 0x30, 0x30, 0x35, 0x30, 0x30,
                    0x3c, 0x30, 0x30, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f,
                    0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x6f, 0x6f, 0x3f, 0x32, 0x0d,
                ],
            ),
        ];
        for (ci, sn, rn, want) in reference {
            let (opc, data, _) = &cases[ci];
            let f = build_i_frame(&encode_opc_payload(*opc, data), sn, rn);
            assert_eq!(f, want, "case {ci} sn={sn} rn={rn}");
        }
        // PROTOCOL.md: connect I-frame with SN=0, RN=0 is "#0@210110G\r"
        assert_eq!(
            build_i_frame(&encode_opc_payload(OPC_IDENTITY_REQUEST, &[1]), 0, 0),
            b"#0@210110G\r"
        );
    }

    #[test]
    fn decode_matches_reference() {
        let enc = |i: usize| encode_cases()[i].2.clone();
        let reference: Vec<(Vec<u8>, Vec<OpcMessage>)> = vec![
            (
                [enc(0), enc(2), enc(1)].concat(),
                vec![
                    (0x1001, vec![1]),
                    (0x8004, b"Flow 0.5 ml/min".to_vec()),
                    (0x2001, (0..14).collect()),
                ],
            ),
            // truncated binary keeps what arrived
            (
                enc(1)[..enc(1).len() - 2].to_vec(),
                vec![(0x2001, (0..13).collect())],
            ),
            (
                enc(2)[..enc(2).len() - 3].to_vec(),
                vec![(0x8004, b"Flow 0.5 ml/".to_vec())],
            ),
            (vec![0x2f, 0x31, 0x32, 0x33], vec![]),
            (vec![0x30], vec![]),
            (vec![0x34, 0x70, 0x30, 0x30, 0x31], vec![]),
            (vec![0x34, 0x31, 0x30, 0x40, 0x31], vec![]),
            (
                vec![0x31, 0x31, 0x30, 0x32, 0x3f],
                vec![(0x2001, vec![0x0f])],
            ),
            (
                vec![0x34, 0x31, 0x30, 0x32, 0x3f, 0x3f, 0x3f, 0x3f],
                vec![(0x2001, vec![0xcf, 0xf3, 0x0f])],
            ),
            (
                vec![0x35, 0x31, 0x30, 0x32, 0x3f, 0x3f, 0x3f, 0x3f, 0x3f],
                vec![(0x2001, vec![0xcf, 0xf3, 0xcf])],
            ),
            (
                vec![0x37, 0x31, 0x30, 0x32, 0x3f, 0x3f, 0x3f, 0x3f, 0x3f],
                vec![(0x2001, vec![0xcf, 0xf3, 0xcf])],
            ),
            (
                [enc(4), enc(11), enc(3)].concat(),
                vec![
                    (0x2013, vec![]),
                    (0x8002, vec![]),
                    (0x2016, [vec![3, 0, 5, 0, 12, 0], vec![0xff; 14]].concat()),
                ],
            ),
            (
                vec![0x40, 0x34, 0x30, 0x38, 0x41, 0x42],
                vec![(0x8004, vec![0x41, 0x42])],
            ),
        ];
        for (input, want) in reference {
            assert_eq!(decode_opc_payload(&input), want, "input {input:02x?}");
        }
    }

    /// Link layer frames: round trips and frame validation.
    #[test]
    fn link_layer_frames() {
        for (opc, data) in [
            (OPC_IDENTITY_REQUEST, vec![0x01]), // binary, odd length
            (OPC_TREND_DATA, (0..14).collect::<Vec<u8>>()), // binary, even length
            (OPC_LOG_SINGLE, b"Flow 0.5 ml/min".to_vec()), // ASCII
        ] {
            assert_eq!(
                decode_opc_payload(&encode_opc_payload(opc, &data)),
                vec![(opc, data)]
            );
        }
        let mut frame = vec![FRAME_START, AKTA_ADDR, CTRL_CONN, 0x00, FRAME_END];
        frame[3] = compute_checksum(&frame, 1, 2);
        assert!(validate_rx_frame(&frame));
        frame[2] ^= 0x01;
        assert!(!validate_rx_frame(&frame));
    }

    #[test]
    fn binary_round_trip_all_lengths() {
        for n in 0..=40usize {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(
                decode_opc_payload(&encode_opc_payload(0x2016, &data)),
                vec![(0x2016, data)]
            );
        }
    }

    #[test]
    fn validate_matches_reference() {
        assert!(validate_rx_frame(&[0x23, 0x31, 0x75, 0x58, 0x0d]));
        assert!(!validate_rx_frame(&[0x23, 0x31, 0x75, 0x59, 0x0d])); // checksum
        assert!(!validate_rx_frame(&[0x23, 0x30, 0x75, 0x58, 0x0d])); // PC address
        assert!(!validate_rx_frame(&[0x23, 0x31, 0x75, 0x58])); // too short
        assert!(!validate_rx_frame(&[0x23, 0x31, 0x30, 0x61, 0x0d]));
        // an I-frame from the Akta: a method list request
        let mut f = build_i_frame(
            &encode_opc_payload(OPC_METHOD_LIST, &[0x01, 0x00, 0x01]),
            1,
            0,
        );
        f[1] = AKTA_ADDR;
        let n = f.len();
        f[n - 2] = compute_checksum(&f, 1, n - 3);
        assert_eq!(
            f,
            [
                0x23, 0x31, 0x44, 0x35, 0x45, 0x30, 0x32, 0x31, 0x30, 0x30, 0x31, 0x30, 0x34, 0x0d
            ]
        );
        assert!(validate_rx_frame(&f));
    }

    #[test]
    fn little_endian_helpers() {
        assert_eq!(u16_le(&[0x34, 0x12], 0), 0x1234);
        assert_eq!(u16_le(&[0x34], 0), 0);
        assert_eq!(le_uint(&[0x9a, 0x02, 0x31, 0x00], 0, 3), 0x31029a);
        assert_eq!(le_uint(&[0x15, 0xcd, 0x5b, 0x07], 0, 4), 123_456_789);
    }
}
