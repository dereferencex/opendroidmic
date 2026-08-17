use std::collections::VecDeque;

pub const SAMPLE_RATE: u32 = 48000;
pub const FRAME_SIZE: usize = 960;

pub struct SharedAudioBuffer {
    buffer: VecDeque<i16>,
    total_read: u64,
    total_written: u64,
}

impl SharedAudioBuffer {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::with_capacity(SAMPLE_RATE as usize * 2),
            total_read: 0,
            total_written: 0,
        }
    }

    pub fn push_pcm(&mut self, data: &[i16]) {
        self.buffer.extend(data);
        self.total_written += data.len() as u64;

        let max_samples = SAMPLE_RATE as usize * 2;
        if self.buffer.len() > max_samples {
            let excess = self.buffer.len() - max_samples;
            self.buffer.drain(..excess);
        }
    }

    pub fn read_frame(&mut self, output: &mut [i16]) {
        let available = self.buffer.len();
        let needed = output.len();
        let to_read = available.min(needed);

        for sample in output.iter_mut().take(to_read) {
            *sample = self.buffer.pop_front().unwrap_or(0);
        }
        for sample in output.iter_mut().skip(to_read) {
            *sample = 0;
        }
        self.total_read += needed as u64;
    }

    pub fn buffered_samples(&self) -> usize {
        self.buffer.len()
    }

    pub fn stats(&self) -> (u64, u64, usize) {
        (self.total_read, self.total_written, self.buffer.len())
    }
}
