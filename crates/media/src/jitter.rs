use std::collections::VecDeque;

use crate::SAMPLE_RATE;

pub struct JitterBuffer {
    buf: VecDeque<i16>,
    prime_samples: usize,
    max_samples: usize,
    chunk_samples: usize,
    primed: bool,
}

impl JitterBuffer {
    /// `chunk_ms` sets the size of every chunk this buffer emits.
    /// Telegram -> Asterisk uses 20ms, Asterisk -> Telegram uses 10ms
    pub fn new(prime_ms: usize, max_ms: usize, chunk_ms: usize) -> Self {
        let to_samples = |ms: usize| ms * (SAMPLE_RATE as usize) / 1000;
        Self {
            buf: VecDeque::with_capacity(to_samples(max_ms)),
            prime_samples: to_samples(prime_ms),
            max_samples: to_samples(max_ms),
            chunk_samples: to_samples(chunk_ms),
            primed: false,
        }
    }

    /// Append inbound samples of any length (e.g. 160 from Telegram).
    pub fn push_samples(&mut self, samples: &[i16]) {
        self.buf.extend(samples.iter().copied());

        // Cap latency: drop the oldest audio if we fall too far behind.
        if self.buf.len() > self.max_samples {
            let excess = self.buf.len() - self.max_samples;
            self.buf.drain(..excess);
        }
    }

    /// Append little-endian slin16 bytes of any length
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        let samples: Vec<i16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();

        self.push_samples(&samples);
    }

    /// Returns one chunk if buffer is primed and holds enough audio
    pub fn pop_chunk(&mut self) -> Option<Vec<i16>> {
        if self.ready() {
            Some(self.take_chunk())
        } else {
            None
        }
    }

    /// For timer-driven output: always retuns one chunk,
    /// provides silence if no audio is available (priming or dry)
    pub fn pop_chunk_or_silence(&mut self) -> Vec<i16> {
        if self.ready() {
            self.take_chunk()
        } else {
            vec![0i16; self.chunk_samples]
        }
    }

    fn ready(&mut self) -> bool {
        if !self.primed {
            if self.buf.len() < self.prime_samples {
                return false;
            }
            self.primed = true;
        }

        if self.buf.len() < self.chunk_samples {
            // ran dry, we need to re-prime
            self.primed = false;
            return false;
        }

        true
    }

    fn take_chunk(&mut self) -> Vec<i16> {
        // let mut out: Vec<i16> = Vec::with_capacity(self.chunk_samples);
        // for s in self.buf.drain(..self.chunk_samples) {
        //     out.extend_from_slice(&s.to_le_bytes());
        // }
        // out
        self.buf.drain(..self.chunk_samples).collect()
    }

    /// Push inbound samples and emit every complete frame to `sink`.
    pub fn on_samples<F>(&mut self, samples: &[i16], mut sink: F)
    where
        F: FnMut(Vec<i16>),
    {
        self.push_samples(samples);
        while let Some(frame) = self.pop_chunk() {
            sink(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: a 20 ms inbound-style buffer and a 10 ms outbound-style buffer.
    fn inbound() -> JitterBuffer {
        JitterBuffer::new(20, 200, 20)
    }
    fn outbound() -> JitterBuffer {
        JitterBuffer::new(40, 200, 10)
    }

    #[test]
    fn two_10ms_pushes_make_one_frame() {
        // let mut jb = JitterBuffer::new(20, 200, 20);
        let mut jb = inbound();
        jb.push_samples(&[1i16; 160]);
        assert!(jb.pop_chunk().is_none()); // not primed yet (needs 320)
        jb.push_samples(&[1i16; 160]);
        let frame = jb.pop_chunk().expect("chunk");
        assert_eq!(frame.len(), 320);
    }

    #[test]
    fn no_partial_chunks() {
        let mut jb = JitterBuffer::new(0, 200, 20);
        jb.push_samples(&[1i16; 200]);
        assert!(jb.pop_chunk().is_none());
    }

    #[test]
    fn silence_while_priming() {
        let mut jb = inbound();
        let chunk = jb.pop_chunk_or_silence();
        assert_eq!(chunk.len(), 320);
        assert!(chunk.iter().all(|&b| b == 0));
    }

    #[test]
    fn outbound_releases_10ms_chunks_then_resilences() {
        let mut jb = outbound();
        jb.push_samples(&[7i16; 640]); // 40 ms = 640 samples, exactly the prime

        for _ in 0..4 {
            let chunk = jb.pop_chunk_or_silence();
            assert_eq!(chunk.len(), 160); // 10 ms
            assert_eq!(chunk[0], 7);
        }

        // Buffer is now empty: next tick is silence and re-primes.
        let chunk = jb.pop_chunk_or_silence();
        assert_eq!(chunk.len(), 160);
        assert!(chunk.iter().all(|&s| s == 0));
        assert!(!jb.primed);
    }

    #[test]
    fn max_ms_drops_oldest_samples() {
        let mut jb = JitterBuffer::new(0, 20, 20); // cap = 320 samples
        let samples: Vec<i16> = (0..480).collect();
        jb.push_samples(&samples);

        let chunk = jb.pop_chunk().expect("chunk");
        // Oldest 160 samples dropped, so the first sample out is 160.
        assert_eq!(chunk[0], 160);
    }

    #[test]
    fn push_bytes_matches_push_samples() {
        let mut a = inbound();
        let mut b = inbound();
        let samples: Vec<i16> = (0..320).collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();

        a.push_samples(&samples);
        b.push_bytes(&bytes);
        assert_eq!(a.pop_chunk(), b.pop_chunk());
    }
}
