//! Audio: waveform preview (symphonia) and playback (cpal, feature `audio-play`).
use crate::{AudioData, Preview};
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const BINS: usize = 480;
/// Samples kept in memory for playback (the waveform always covers the whole file).
pub const KEEP_SECS: f64 = 300.0;

pub struct Decoded {
    /// Interleaved f32, first `KEEP_SECS` only.
    pub samples: Vec<f32>,
    pub rate: u32,
    pub channels: u16,
    pub peaks: Vec<f32>,
    pub duration_s: f64,
    pub codec: String,
}

pub fn decode(path: &Path, keep_samples: bool) -> Result<Decoded, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) { hint.with_extension(e); }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).map_err(|e| format!("unsupported audio: {e}"))?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or("no audio track")?.clone();
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| e.to_string())?;
    let codec = format!("{:?}", track.codec_params.codec).trim_start_matches("CodecType(").trim_end_matches(')').to_string();
    let mut rate = track.codec_params.sample_rate.unwrap_or(44100);
    let mut channels = track.codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);
    let keep_frames = (KEEP_SECS * rate as f64) as usize;
    let mut samples: Vec<f32> = vec![];
    let mut fine: Vec<f32> = vec![];
    const BLOCK: usize = 512;
    let (mut cur_peak, mut in_block, mut total_frames) = (0f32, 0usize, 0usize);
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    loop {
        let pkt = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(_)) | Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if pkt.track_id() != track.id { continue; }
        let dec = match decoder.decode(&pkt) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *dec.spec();
        rate = spec.rate;
        channels = spec.channels.count() as u16;
        let sb = sbuf.get_or_insert_with(|| SampleBuffer::<f32>::new(dec.capacity() as u64, spec));
        if sb.capacity() < dec.frames() * channels as usize { *sb = SampleBuffer::<f32>::new(dec.capacity() as u64, spec); }
        sb.copy_interleaved_ref(dec);
        let data = sb.samples();
        for fr in data.chunks_exact(channels as usize) {
            let m = fr.iter().fold(0f32, |a, &s| a.max(s.abs()));
            cur_peak = cur_peak.max(m);
            in_block += 1;
            if in_block == BLOCK { fine.push(cur_peak); cur_peak = 0.0; in_block = 0; }
        }
        total_frames += data.len() / channels as usize;
        if keep_samples && samples.len() / (channels as usize) < keep_frames {
            samples.extend_from_slice(data);
        }
    }
    if in_block > 0 { fine.push(cur_peak); }
    if total_frames == 0 { return Err("no audio decoded".into()); }
    let mut peaks = vec![0f32; BINS];
    for (i, p) in fine.iter().enumerate() {
        let b = (i * BINS / fine.len()).min(BINS - 1);
        peaks[b] = peaks[b].max(*p);
    }
    Ok(Decoded { samples, rate, channels, peaks, duration_s: total_frames as f64 / rate as f64, codec })
}

pub fn load(path: &Path) -> Preview {
    match decode(path, false) {
        Ok(d) => Preview::Audio(AudioData { peaks: d.peaks, duration_s: d.duration_s, sample_rate: d.rate, channels: d.channels, codec: d.codec }),
        Err(e) => Preview::Error(e),
    }
}

#[cfg(feature = "audio-play")]
pub use play::Player;

#[cfg(feature = "audio-play")]
mod play {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;

    struct Shared {
        samples: Vec<f32>,
        rate: u32,
        channels: usize,
        pos: AtomicU64, // f64 bits, in source frames
        playing: AtomicBool,
        #[allow(dead_code)]
        duration_s: f64,
    }

    pub struct Player {
        _stream: cpal::Stream,
        sh: Arc<Shared>,
        pub duration_s: f64,
    }

    impl Player {
        /// Decode (first 5 minutes) and open the default output device.
        pub fn open(path: &Path) -> Result<Player, String> {
            let d = decode(path, true)?;
            Player::from_decoded(d)
        }

        pub fn from_decoded(d: Decoded) -> Result<Player, String> {
            let host = cpal::default_host();
            let dev = host.default_output_device().ok_or("no audio output device")?;
            let cfgs = dev.supported_output_configs().map_err(|e| e.to_string())?;
            let cfg = cfgs
                .filter(|c| c.sample_format() == cpal::SampleFormat::F32 && c.channels() >= 2)
                .min_by_key(|c| c.channels())
                .ok_or("no f32 stereo output config")?;
            let cfg = cfg.with_sample_rate(dev.default_output_config().map(|c| c.sample_rate()).unwrap_or(cpal::SampleRate(44100)).clamp(cfg.min_sample_rate(), cfg.max_sample_rate()));
            let out_rate = cfg.sample_rate().0 as f64;
            let out_ch = cfg.channels() as usize;
            let sh = Arc::new(Shared::new(d.samples, d.rate, d.channels as usize));
            let sh2 = sh.clone();
            let step = sh.rate as f64 / out_rate;
            let stream = dev
                .build_output_stream(
                    &cfg.config(),
                    move |out: &mut [f32], _| {
                        let frames = sh2.samples.len() / sh2.channels;
                        let mut pos = f64::from_bits(sh2.pos.load(Ordering::Relaxed));
                        let playing = sh2.playing.load(Ordering::Relaxed);
                        for fr in out.chunks_mut(out_ch) {
                            let i = pos as usize;
                            if !playing || i + 1 >= frames {
                                fr.iter_mut().for_each(|s| *s = 0.0);
                                if playing && i + 1 >= frames { sh2.playing.store(false, Ordering::Relaxed); }
                                continue;
                            }
                            let t = (pos - i as f64) as f32;
                            let s = |f: usize, c: usize| sh2.samples[f * sh2.channels + c.min(sh2.channels - 1)];
                            let (l, r) = (s(i, 0) * (1.0 - t) + s(i + 1, 0) * t, s(i, 1) * (1.0 - t) + s(i + 1, 1) * t);
                            fr[0] = l;
                            if out_ch > 1 { fr[1] = r; }
                            for x in fr.iter_mut().skip(2) { *x = 0.0; }
                            pos += step;
                        }
                        sh2.pos.store(pos.to_bits(), Ordering::Relaxed);
                    },
                    |e| eprintln!("audio stream error: {e}"),
                    None,
                )
                .map_err(|e| e.to_string())?;
            stream.play().map_err(|e| e.to_string())?;
            Ok(Player { _stream: stream, duration_s: d.duration_s, sh })
        }

        pub fn play(&self) { self.sh.playing.store(true, Ordering::Relaxed) }
        pub fn pause(&self) { self.sh.playing.store(false, Ordering::Relaxed) }
        pub fn is_playing(&self) -> bool { self.sh.playing.load(Ordering::Relaxed) }
        pub fn position_s(&self) -> f64 { f64::from_bits(self.sh.pos.load(Ordering::Relaxed)) / self.sh.rate as f64 }
        pub fn seek_frac(&self, f: f64) {
            let frames = (self.sh.samples.len() / self.sh.channels) as f64;
            self.sh.pos.store((frames * f.clamp(0.0, 0.999)).to_bits(), Ordering::Relaxed);
        }
    }

    impl Shared {
        fn new(samples: Vec<f32>, rate: u32, channels: usize) -> Shared {
            Shared { samples, rate, channels: channels.max(1), pos: AtomicU64::new(0f64.to_bits()), playing: AtomicBool::new(false), duration_s: 0.0 }
        }
    }
}
