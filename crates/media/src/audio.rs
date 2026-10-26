// TODO: this should be handled depending on audio format chosen by config
pub const FRAME_SAMPLES: usize = 320; // 20 ms @ 16 kHz
pub const FRAME_BYTES: usize = FRAME_SAMPLES * 2; // 64
pub const SAMPLE_RATE: u32 = 16_000;

pub fn samples_to_le_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}
