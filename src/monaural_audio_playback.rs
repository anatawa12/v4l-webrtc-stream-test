use crate::audio::Error;

use alsa::pcm::{Access, Format, HwParams};
use alsa::{ValueOr, PCM};
use opus::{Channels, Decoder};

pub struct MonauralAudioPlayback {
    pcm: PCM,
    opus_decoder: Decoder,
    output_buffer: Vec<i16>,
}

impl MonauralAudioPlayback {
    pub fn new(name: &str, sample_rate: u32) -> Result<Self, Error> {
        let pcm = PCM::new(name, alsa::Direction::Playback, false)?;
        {
            let params = HwParams::any(&pcm).unwrap();
            params.set_channels(1)?;
            params.set_format(Format::s16())?;
            params.set_rate(sample_rate, ValueOr::Nearest)?;
            params.set_access(Access::RWInterleaved)?;
            pcm.hw_params(&params)?;
        }

        let opus_decoder = Decoder::new(sample_rate, Channels::Mono)?;
        let output_buffer = vec![0i16; sample_rate as usize * 40 / 1000];

        Ok(Self {
            pcm,
            opus_decoder,
            output_buffer,
        })
    }

    pub fn play_frame(&mut self, encoded: &[u8]) -> Result<(), Error> {
        let samples = self
            .opus_decoder
            .decode(&encoded, &mut self.output_buffer, false)?;
        let buffer = &self.output_buffer[..samples];
        self.pcm.io_i16()?.writei(buffer)?;
        Ok(())
    }
}
