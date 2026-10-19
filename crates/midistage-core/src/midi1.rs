//! MIDI 1.0 のバイト列 ⇄ UMP（MT 1 = system、MT 2 = MIDI 1.0 channel voice、MT 3 = SysEx7）。
//! Windows MIDI Services は UMP で読み書きする（design 07）。純粋。group は 0 だけを扱う。

/// 完全な MIDI 1.0 メッセージ 1 つ（running status 無し）を UMP packet の列にする。
/// SysEx（F0 … F7）は 6 byte ずつの SysEx7 packet に分ける。表せないものは空
pub fn encode(message: &[u8]) -> Vec<Vec<u32>> {
    let Some(&status) = message.first() else {
        return vec![];
    };
    if status == 0xf0 {
        return encode_sysex(message);
    }
    let data_ok = message[1..].iter().all(|b| *b < 0x80);
    if status < 0x80 || message.len() != message_len(status) || !data_ok {
        return vec![];
    }
    let mt = if status >= 0xf0 { 0x1 } else { 0x2 };
    let d1 = message.get(1).copied().unwrap_or(0) as u32;
    let d2 = message.get(2).copied().unwrap_or(0) as u32;
    vec![vec![(mt << 28) | ((status as u32) << 16) | (d1 << 8) | d2]]
}

/// status byte から、メッセージ全体の長さ（F0 と未定義は 0）
fn message_len(status: u8) -> usize {
    match status {
        0x80..=0xbf | 0xe0..=0xef | 0xf2 => 3,
        0xc0..=0xdf | 0xf1 | 0xf3 => 2,
        0xf6 | 0xf8..=0xff => 1,
        _ => 0,
    }
}

fn encode_sysex(message: &[u8]) -> Vec<Vec<u32>> {
    let Some((&0xf7, body)) = message.split_last() else {
        return vec![];
    };
    let payload = &body[1..];
    if payload.iter().any(|b| *b >= 0x80) {
        return vec![];
    }
    let chunks: Vec<&[u8]> = if payload.is_empty() {
        vec![&[]]
    } else {
        payload.chunks(6).collect()
    };
    let last = chunks.len() - 1;
    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            // 0 = complete、1 = start、2 = continue、3 = end
            let state: u32 = match (last, i) {
                (0, _) => 0,
                (_, 0) => 1,
                (_, i) if i == last => 3,
                _ => 2,
            };
            let mut b = [0u32; 6];
            for (slot, byte) in b.iter_mut().zip(chunk.iter()) {
                *slot = *byte as u32;
            }
            vec![
                (0x3 << 28) | (state << 20) | ((chunk.len() as u32) << 16) | (b[0] << 8) | b[1],
                (b[2] << 24) | (b[3] << 16) | (b[4] << 8) | b[5],
            ]
        })
        .collect()
}

/// UMP packet を MIDI 1.0 メッセージに戻す。SysEx7 の断片は F7 まで溜める
#[derive(Default)]
pub struct Decoder {
    sysex: Option<Vec<u8>>,
}
impl Decoder {
    /// packet 1 つ（word 1〜4 個）。メッセージが完成したら返す。
    /// MT 1 / 2 / 3 以外（utility、stream、MIDI 2.0 など）と group 0 以外は捨てる
    pub fn push(&mut self, words: &[u32]) -> Option<Vec<u8>> {
        let first = *words.first()?;
        let mt = first >> 28;
        let group = (first >> 24) & 0xf;
        if group != 0 {
            return None;
        }
        let status = ((first >> 16) & 0xff) as u8;
        let [d1, d2] = [((first >> 8) & 0x7f) as u8, (first & 0x7f) as u8];
        match mt {
            0x1 | 0x2 => {
                let valid = if mt == 0x1 {
                    status >= 0xf1 && status != 0xf7
                } else {
                    (0x80..0xf0).contains(&status)
                };
                let len = message_len(status);
                (valid && len > 0).then(|| [status, d1, d2][..len].to_vec())
            }
            0x3 => {
                let second = words.get(1).copied().unwrap_or(0);
                let count = (((first >> 16) & 0xf) as usize).min(6);
                let bytes = [
                    (first >> 8) as u8,
                    first as u8,
                    (second >> 24) as u8,
                    (second >> 16) as u8,
                    (second >> 8) as u8,
                    second as u8,
                ];
                let chunk = bytes[..count].iter().map(|b| b & 0x7f);
                match (first >> 20) & 0xf {
                    0 => {
                        self.sysex = None;
                        Some(std::iter::once(0xf0).chain(chunk).chain([0xf7]).collect())
                    }
                    1 => {
                        // 途中の SysEx があれば捨てて始め直す
                        self.sysex = Some(std::iter::once(0xf0).chain(chunk).collect());
                        None
                    }
                    2 => {
                        self.sysex.as_mut()?.extend(chunk);
                        None
                    }
                    3 => {
                        let mut message = self.sysex.take()?;
                        message.extend(chunk);
                        message.push(0xf7);
                        Some(message)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(packets: &[Vec<u32>]) -> Vec<Vec<u8>> {
        let mut decoder = Decoder::default();
        packets.iter().filter_map(|p| decoder.push(p)).collect()
    }

    #[test]
    fn channel_voice_is_mt2_and_round_trips() {
        assert_eq!(encode(&[0xb0, 0x10, 0x7f]), vec![vec![0x20b0_107f]]);
        assert_eq!(encode(&[0xc5, 0x03]), vec![vec![0x20c5_0300]]);
        for message in [
            vec![0x90, 60, 100],
            vec![0x80, 60, 0],
            vec![0xb3, 7, 64],
            vec![0xc0, 5],
            vec![0xd1, 9],
            vec![0xe2, 0x00, 0x40],
        ] {
            assert_eq!(decode_all(&encode(&message)), vec![message]);
        }
    }

    #[test]
    fn system_messages_are_mt1_and_round_trip() {
        assert_eq!(encode(&[0xf8]), vec![vec![0x10f8_0000]]);
        assert_eq!(encode(&[0xf2, 0x01, 0x02]), vec![vec![0x10f2_0102]]);
        for message in [
            vec![0xf8],
            vec![0xfa],
            vec![0xf6],
            vec![0xf1, 0x11],
            vec![0xf3, 2],
        ] {
            assert_eq!(decode_all(&encode(&message)), vec![message]);
        }
    }

    #[test]
    fn short_sysex_is_one_complete_packet() {
        // Device Inquiry。spike で nanoKONTROL2 に送った形（2026-10-19）
        assert_eq!(
            encode(&[0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7]),
            vec![vec![0x3004_7e7f, 0x0601_0000]]
        );
        assert_eq!(encode(&[0xf0, 0xf7]), vec![vec![0x3000_0000, 0]]);
    }

    #[test]
    fn long_sysex_splits_into_start_continue_end_and_round_trips() {
        let mut message = vec![0xf0];
        message.extend(0..13u8);
        message.push(0xf7);
        let packets = encode(&message);
        assert_eq!(packets.len(), 3);
        assert_eq!(packets[0][0] >> 16, 0x3016); // start, 6 byte
        assert_eq!(packets[1][0] >> 16, 0x3026); // continue, 6 byte
        assert_eq!(packets[2][0] >> 16, 0x3031); // end, 1 byte
        assert_eq!(decode_all(&packets), vec![message]);
    }

    #[test]
    fn fragments_from_the_driver_are_reassembled() {
        // nanoKONTROL2 の Device Inquiry 応答は 2〜3 byte の断片で届いた（spike 2026-10-19）
        let packets = [
            vec![0x3012_7e00, 0],
            vec![0x3023_0602, 0x4200_0000],
            vec![0x3023_1301, 0],
            vec![0x3023_0003, 0],
            vec![0x3032_0100, 0],
        ];
        assert_eq!(
            decode_all(&packets),
            vec![vec![
                0xf0, 0x7e, 0x00, 0x06, 0x02, 0x42, 0x13, 0x01, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00,
                0xf7
            ]]
        );
    }

    #[test]
    fn stray_continuations_other_groups_and_other_types_are_dropped() {
        // start の無い continue / end
        assert_eq!(
            decode_all(&[vec![0x3023_0102, 0], vec![0x3031_0300, 0]]),
            Vec::<Vec<u8>>::new()
        );
        // group 1
        assert_eq!(decode_all(&[vec![0x21b0_107f]]), Vec::<Vec<u8>>::new());
        // stream（MT F）と utility（MT 0）と MIDI 2.0（MT 4）
        assert_eq!(
            decode_all(&[
                vec![0xf006_0100, 0, 0, 0],
                vec![0x0020_1234],
                vec![0x4090_3c00, 0xffff_0000]
            ]),
            Vec::<Vec<u8>>::new()
        );
        // 新しい start は途中の SysEx を捨てて始め直す
        assert_eq!(
            decode_all(&[vec![0x3012_0102, 0], vec![0x3002_0304, 0]]),
            vec![vec![0xf0, 0x03, 0x04, 0xf7]]
        );
    }

    #[test]
    fn running_status_fragments_and_bad_input_encode_to_nothing() {
        assert!(encode(&[]).is_empty());
        assert!(encode(&[0x10, 0x20]).is_empty());
        assert!(encode(&[0xb0, 0x10]).is_empty()); // 足りない
        assert!(encode(&[0xf0, 0x01]).is_empty()); // F7 が無い
        assert!(encode(&[0xf0, 0x81, 0xf7]).is_empty()); // data byte でない
    }
}
