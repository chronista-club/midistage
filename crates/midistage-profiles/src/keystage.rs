//! Keystage connection finalization; recognizes only the documented connect command.
fn connection_value(bytes: &[u8]) -> Option<u8> {
    if bytes.len() == 13
        && bytes[0] == 0xf0
        && bytes[1] == 0x42
        && (0x40..=0x4f).contains(&bytes[2])
        && bytes[3..6] == [0, 1, 0x69]
        && [1, 9].contains(&bytes[6])
        && bytes[7..11] == [2, 0, 0, 0x6f]
        && bytes[11] <= 1
        && bytes[12] == 0xf7
    {
        Some(bytes[11])
    } else {
        None
    }
}
/// Remember this inverse only after the driver successfully sent the connection.
pub fn disconnect_after(bytes: &[u8]) -> Option<Vec<u8>> {
    if connection_value(bytes) != Some(1) {
        return None;
    }
    let mut inverse = bytes.to_vec();
    inverse[11] = 0;
    Some(inverse)
}
pub fn is_disconnect(bytes: &[u8]) -> bool {
    connection_value(bytes) == Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnect_preserves_model_and_channel_and_rejects_other_frames() {
        for model in [1, 9] {
            for channel in 0..16 {
                let mut frame = vec![
                    0xf0,
                    0x42,
                    0x40 | channel,
                    0,
                    1,
                    0x69,
                    model,
                    2,
                    0,
                    0,
                    0x6f,
                    1,
                    0xf7,
                ];
                let disconnect = disconnect_after(&frame).unwrap();
                frame[11] = 0;
                assert_eq!(disconnect, frame);
                assert!(is_disconnect(&frame));
                assert_eq!(disconnect_after(&frame), None);
                frame[10] = 0x28;
                assert!(!is_disconnect(&frame));
                assert_eq!(disconnect_after(&frame), None);
            }
        }
        assert_eq!(disconnect_after(&[0xf0, 0xf7]), None);
        assert!(!is_disconnect(&[]));
    }
}
