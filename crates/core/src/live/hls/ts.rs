//! MPEG-TS packets (ISO/IEC 13818-1) as the client reads and writes them:
//! their size, their sync byte, and telling a stream of them by its start.

pub(super) const PACKET: usize = 188;
pub(super) const SYNC: u8 = 0x47;
/// Sync bytes one packet apart that tell MPEG-TS, as FFmpeg's probe looks
/// for them: an answer may start in the middle of a packet.
const SYNCED_PACKETS: usize = 4;
/// What `is_transport_stream` needs of a stream's start.
pub(super) const HEAD: usize = PACKET * (SYNCED_PACKETS + 1);

/// Whether `head`, the start of a stream, is MPEG-TS. Text may have a `G`
/// (0x47) in the same places: a playlist is told apart first.
pub(super) fn is_transport_stream(head: &[u8]) -> bool {
    (0..PACKET).any(|start| (0..SYNCED_PACKETS).all(|i| head.get(start + i * PACKET) == Some(&SYNC)))
}

/// Where the first packet of a program association table (PID 0, start of
/// a section) begins in `data`: from there a demuxer learns the program's
/// streams before their data. Confirmed by the next packet's sync byte when
/// `data` holds it.
pub(super) fn tables_start(data: &[u8]) -> Option<usize> {
    (0..data.len().saturating_sub(3)).find(|&i| {
        data[i] == SYNC && data[i + 1] & 0x5F == 0x40 && data[i + 2] == 0 && data.get(i + PACKET).is_none_or(|&b| b == SYNC)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packets(offset: usize, count: usize) -> Vec<u8> {
        let mut data = vec![0xAB; offset];
        for _ in 0..count {
            data.push(SYNC);
            data.extend([0; PACKET - 1]);
        }
        data
    }

    #[test]
    fn transport_streams_are_told_apart_even_mid_packet() {
        assert!(is_transport_stream(&packets(0, SYNCED_PACKETS)));
        assert!(is_transport_stream(&packets(PACKET - 1, SYNCED_PACKETS)));
        assert!(!is_transport_stream(&packets(0, SYNCED_PACKETS - 1)));
        assert!(!is_transport_stream(b"<html>404</html>"));
    }

    #[test]
    fn the_tables_start_at_the_first_association_table() {
        let mut data = packets(5, 3);
        let pat = 5 + 2 * PACKET;
        data[pat + 1] = 0x40; // start of a section, PID 0
        data[5 + 1] = 0x01; // PID 0x100: not a table
        assert_eq!(tables_start(&data), Some(pat));
        assert_eq!(tables_start(&data[..pat]), None);
    }
}
