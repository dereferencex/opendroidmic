use anyhow::{Context, Result};
use opus::{Decoder, Channels};
use tracing::info;

pub struct OpusDecoder {
    decoder: Decoder,
    sample_rate: u32,
    channels: u32,
}

impl OpusDecoder {
    pub fn new(sample_rate: u32, channels: u32) -> Result<Self> {
        let opus_channels = match channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => anyhow::bail!("Unsupported channel count: {}", channels),
        };

        let decoder = Decoder::new(sample_rate, opus_channels)
            .context("Failed to create Opus decoder")?;

        info!(
            "Opus decoder created: {} Hz, {} ch",
            sample_rate, channels
        );

        Ok(Self {
            decoder,
            sample_rate,
            channels,
        })
    }

    pub fn decode(&mut self, opus_data: &[u8], output: &mut [i16]) -> Result<usize> {
        let max_frames = output.len() / self.channels as usize;
        let decoded = self
            .decoder
            .decode(opus_data, &mut output[..max_frames * self.channels as usize], false)
            .context("Opus decode failed")?;
        Ok(decoded * self.channels as usize)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }
}
