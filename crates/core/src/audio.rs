//! Decodifica dei file audio e conversione nel formato richiesto da Whisper
//! (PCM a virgola mobile, mono, 16 kHz).

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::Cancelled;

/// Frequenza di campionamento richiesta da Whisper.
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Estensioni proposte nella finestra di apertura dei file.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "flac", "ogg", "oga", "m4a", "mp4", "aac", "alac", "caf", "aif", "aiff", "mka",
    "mkv", "webm",
];

/// Audio pronto per la trascrizione.
#[derive(Debug, Clone)]
pub struct PreparedAudio {
    /// Campioni mono a 16 kHz nell'intervallo [-1, 1].
    pub samples: Vec<f32>,
    /// Frequenza di campionamento originale.
    pub source_sample_rate: u32,
    /// Numero di canali originali.
    pub source_channels: usize,
}

impl PreparedAudio {
    /// Durata in millisecondi.
    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 * 1000 / u64::from(WHISPER_SAMPLE_RATE)
    }
}

/// Decodifica un file audio e lo converte in mono 16 kHz.
///
/// `progress` riceve valori tra 0 e 1 quando la durata del file è nota.
/// Se `cancel` diventa `true` l'operazione si interrompe con [`Cancelled`].
pub fn load_for_whisper(
    path: &Path,
    mut progress: impl FnMut(f32),
    cancel: &AtomicBool,
) -> Result<PreparedAudio> {
    let file = File::open(path)
        .with_context(|| format!("Impossibile aprire il file audio «{}».", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| anyhow!("Formato audio non riconosciuto o non supportato ({e})."))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| anyhow!("Il file non contiene tracce audio."))?;
    let track_id = track.id;
    let total_frames = track.num_frames;
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("Parametri del codec audio mancanti."))?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
        .map_err(|e| anyhow!("Codec audio non supportato ({e})."))?;

    let mut mono: Vec<f32> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();
    let mut sample_rate: Option<u32> = None;
    let mut channels = 0usize;
    let mut last_reported = -1.0f32;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            // Fine dei dati o catena di flussi cambiata: consideriamo concluso il file.
            Err(SymphoniaError::ResetRequired) => break,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(e) => bail!("Errore durante la lettura del file audio: {e}."),
        };
        if packet.track_id != track_id {
            continue;
        }

        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            // Pacchetti danneggiati: si saltano, come fanno i lettori multimediali.
            Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
            Err(e) => bail!("Errore di decodifica audio: {e}."),
        };

        let spec = buffer.spec();
        let rate = spec.rate();
        let n_channels = spec.channels().count().max(1);
        match sample_rate {
            None => {
                sample_rate = Some(rate);
                channels = n_channels;
            }
            Some(r) if r != rate => {
                bail!("Il file cambia frequenza di campionamento durante la riproduzione: non supportato.")
            }
            _ => {}
        }

        buffer.copy_to_vec_interleaved(&mut interleaved);
        downmix_into(&interleaved, n_channels, &mut mono);

        if let Some(total) = total_frames.filter(|t| *t > 0) {
            let fraction = (mono.len() as f64 / total as f64).min(1.0) as f32;
            if fraction - last_reported >= 0.01 {
                last_reported = fraction;
                progress(fraction);
            }
        }
    }

    let source_sample_rate =
        sample_rate.ok_or_else(|| anyhow!("Il file non contiene audio decodificabile."))?;
    if mono.is_empty() {
        bail!("Il file non contiene audio decodificabile.");
    }

    let samples = resample_to_whisper_rate(&mono, source_sample_rate)?;
    progress(1.0);

    Ok(PreparedAudio {
        samples,
        source_sample_rate,
        source_channels: channels,
    })
}

/// Media i canali di un buffer interleaved e accoda il risultato mono a `out`.
fn downmix_into(interleaved: &[f32], channels: usize, out: &mut Vec<f32>) {
    if channels == 1 {
        out.extend_from_slice(interleaved);
        return;
    }
    let scale = 1.0 / channels as f32;
    out.extend(
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() * scale),
    );
}

/// Ricampiona audio mono a 16 kHz con un ricampionatore FFT di qualità.
pub fn resample_to_whisper_rate(mono: &[f32], source_rate: u32) -> Result<Vec<f32>> {
    if source_rate == WHISPER_SAMPLE_RATE {
        return Ok(mono.to_vec());
    }
    let mut resampler = Fft::<f32>::new(
        source_rate as usize,
        WHISPER_SAMPLE_RATE as usize,
        1024,
        1,
        FixedSync::Input,
    )
    .map_err(|e| anyhow!("Impossibile creare il ricampionatore: {e}"))?;
    let input = InterleavedSlice::new(mono, 1, mono.len())
        .map_err(|e| anyhow!("Buffer audio non valido: {e}"))?;
    let output = resampler
        .process_all(&input, mono.len(), None)
        .map_err(|e| anyhow!("Errore durante il ricampionamento: {e}"))?;
    Ok(output.take_data())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_stereo() {
        let mut out = Vec::new();
        downmix_into(&[1.0, 0.0, 0.5, 0.5], 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    #[test]
    fn resample_keeps_duration() {
        let rate = 44_100u32;
        let input: Vec<f32> = (0..rate)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5)
            .collect();
        let out = resample_to_whisper_rate(&input, rate).unwrap();
        let expected = WHISPER_SAMPLE_RATE as usize;
        assert!(
            out.len().abs_diff(expected) <= 2,
            "lunghezza {} invece di {expected}",
            out.len()
        );
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((0.4..0.6).contains(&peak), "picco inatteso: {peak}");
    }

    #[test]
    fn decode_wav_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prova.wav");
        write_test_wav(&path, 8_000, 2, 8_000);
        let cancel = AtomicBool::new(false);
        let audio = load_for_whisper(&path, |_| {}, &cancel).unwrap();
        assert_eq!(audio.source_sample_rate, 8_000);
        assert_eq!(audio.source_channels, 2);
        assert!(audio.samples.len().abs_diff(16_000) <= 2);
        assert!(audio.duration_ms().abs_diff(1000) <= 1);
    }

    #[test]
    fn decode_can_be_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prova.wav");
        write_test_wav(&path, 16_000, 1, 16_000);
        let cancel = AtomicBool::new(true);
        let err = load_for_whisper(&path, |_| {}, &cancel).unwrap_err();
        assert!(crate::is_cancelled(&err));
    }

    /// Scrive un WAV PCM 16 bit con un'onda sinusoidale.
    fn write_test_wav(path: &Path, rate: u32, channels: u16, frames: u32) {
        let bits = 16u16;
        let block_align = channels * bits / 8;
        let data_len = frames * u32::from(block_align);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let v = ((i as f32 * 220.0 * std::f32::consts::TAU / rate as f32).sin() * 12_000.0)
                as i16;
            for _ in 0..channels {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        std::fs::write(path, bytes).unwrap();
    }
}
