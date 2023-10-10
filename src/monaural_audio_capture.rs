use alsa::pcm::{Access, Format, HwParams};
use alsa::{Direction, ValueOr, PCM};
use opus::{Application, Bitrate, Channels, Encoder};
use std::time::Duration;

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Alsa(alsa::Error),
    Opus(opus::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Alsa(e) => std::fmt::Display::fmt(e, f),
            Error::Opus(e) => std::fmt::Display::fmt(e, f),
        }
    }
}

impl From<alsa::Error> for Error {
    fn from(value: alsa::Error) -> Self {
        Self::Alsa(value)
    }
}

impl From<opus::Error> for Error {
    fn from(value: opus::Error) -> Self {
        Self::Opus(value)
    }
}

impl std::error::Error for Error {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            Error::Alsa(x) => Some(x),
            Error::Opus(x) => Some(x),
        }
    }
}

pub struct MonauralAudioCapture {
    pcm: PCM,
    opus_encoder: Encoder,
    capture_buffer: Vec<i16>,
    encoded_buffer: Vec<u8>,
    sample_rate: u32,
}

impl MonauralAudioCapture {
    pub fn new(name: &str, sample_rate: u32, bit_rate: i32, frame_ms: u32) -> Result<Self, Error> {
        let pcm = PCM::new(name, Direction::Capture, false)?;
        {
            let params = HwParams::any(&pcm).unwrap();
            params.set_rate_resample(true)?;
            params.set_access(Access::RWInterleaved)?;
            params.set_channels(1)?;
            params.set_rate(sample_rate, ValueOr::Nearest)?;
            params.set_format(Format::s16())?;
            pcm.hw_params(&params)?;
        }

        pcm.prepare()?;

        let mut opus_encoder = Encoder::new(sample_rate, Channels::Mono, Application::Voip)?;
        opus_encoder.set_bitrate(Bitrate::Bits(bit_rate))?;

        let mut capture_buffer = vec![0i16; (sample_rate * frame_ms / 1000) as usize];
        let mut encoded_buffer = vec![0u8; bit_rate as usize / 8 / (1000 / frame_ms as usize)];
        Ok(Self {
            pcm,
            opus_encoder,
            capture_buffer,
            encoded_buffer,
            sample_rate,
        })
    }

    pub fn capture_frame(&mut self) -> Result<(Duration, &[u8]), Error> {
        // https://github.com/diwic/alsa-rs/issues/111
        let read = self.pcm.io_i16()?.readi(&mut self.capture_buffer)?;
        let buffer = &self.capture_buffer[..read];
        let duration = Duration::from_millis(buffer.len() as u64 * 1000 / self.sample_rate as u64);

        let encoded = self.opus_encoder.encode(buffer, &mut self.encoded_buffer)?;
        Ok((duration, &self.encoded_buffer[..encoded]))
    }
}
