/// CoreMIDI packet 境界と MIDI message 境界は一致しない。
#[derive(Default)]
pub struct MidiStream {
    pending: Vec<u8>,
    running: Option<u8>,
    length: usize,
}
impl MidiStream {
    pub fn push(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut messages = Vec::new();
        for &byte in bytes {
            if byte >= 0xf8 {
                messages.push(vec![byte]);
                continue;
            }
            if self.pending.first() == Some(&0xf0) {
                if byte == 0xf7 {
                    self.pending.push(byte);
                    messages.push(std::mem::take(&mut self.pending));
                    continue;
                }
                if byte < 0x80 {
                    self.pending.push(byte);
                    if self.pending.len() > 131_072 {
                        self.pending.clear();
                        anyhow::bail!("SysEx exceeds 128 KiB");
                    }
                    continue;
                }
                self.pending.clear(); // 中断された SysEx は送らない。
            }
            if byte >= 0x80 {
                self.pending.clear();
                self.running = (byte < 0xf0).then_some(byte);
                self.length = match byte {
                    0x80..=0xbf | 0xe0..=0xef | 0xf2 => 3,
                    0xc0..=0xdf | 0xf1 | 0xf3 => 2,
                    0xf6 => 1,
                    0xf0 => {
                        self.pending.push(byte);
                        continue;
                    }
                    _ => continue,
                };
                self.pending.push(byte);
            } else {
                if self.pending.is_empty() {
                    let Some(status) = self.running else {
                        continue;
                    };
                    self.pending.push(status);
                }
                self.pending.push(byte);
            }
            if self.pending.len() == self.length {
                messages.push(std::mem::take(&mut self.pending));
            }
        }
        Ok(messages)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_sysex_preserves_realtime_and_waits_for_terminator() {
        let mut s = MidiStream::default();
        assert_eq!(s.push(&[0xf0, 0x7d, 1, 0xf8, 2]).unwrap(), vec![vec![0xf8]]);
        assert_eq!(
            s.push(&[3, 0xf7, 0xb0, 16, 127]).unwrap(),
            vec![vec![0xf0, 0x7d, 1, 2, 3, 0xf7], vec![0xb0, 16, 127]]
        );
    }
    #[test]
    fn running_status_and_partial_messages_are_reconstructed() {
        let mut s = MidiStream::default();
        assert!(s.push(&[0x90, 60]).unwrap().is_empty());
        assert_eq!(
            s.push(&[100, 61, 0, 0xc0, 7]).unwrap(),
            vec![vec![0x90, 60, 100], vec![0x90, 61, 0], vec![0xc0, 7]]
        );
    }
    #[test]
    fn unbounded_sysex_fails_closed() {
        let mut s = MidiStream::default();
        let mut data = vec![0; 131_074];
        data[0] = 0xf0;
        assert!(s.push(&data).is_err());
    }
}
