//! WAV inspection + streaming chunk planning/reading.
//!
//! Long meetings (1-2h) are ~230MB at 16kHz mono 16-bit. We never load the
//! whole file into RAM: `plan_chunks` is a pure function that computes frame
//! windows, and `read_chunk_wav` seeks into the WAV and materializes only one
//! chunk at a time.
//!
//!   total frames
//!   |------------------------------------------------------|
//!   [   chunk 0   ]
//!             [   chunk 1   ]      <- overlap avoids word cuts
//!                       [   chunk 2   ]
//!
//! Each emitted chunk is a self-contained 16kHz mono WAV (bytes) under the Groq
//! size threshold.

use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result};

/// A half-open frame window `[start, end)` into the source WAV (frames are
/// per-channel sample positions, i.e. what `hound`'s `seek` understands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSpec {
    pub start_frame: u32,
    pub end_frame: u32,
}

impl ChunkSpec {
    pub fn frames(&self) -> u32 {
        self.end_frame - self.start_frame
    }
}

/// Basic facts about a WAV file, read from its header only.
#[derive(Debug, Clone, Copy)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    /// Frames per channel (total samples / channels).
    pub total_frames: u32,
}

impl WavInfo {
    pub fn bytes_per_frame(&self) -> u32 {
        self.channels as u32 * (self.bits_per_sample as u32 / 8)
    }

    pub fn read(path: &Path) -> Result<WavInfo> {
        let reader = hound::WavReader::open(path)
            .with_context(|| format!("WAV açılamadı: {}", path.display()))?;
        let spec = reader.spec();
        Ok(WavInfo {
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            bits_per_sample: spec.bits_per_sample,
            total_frames: reader.duration(),
        })
    }
}

/// Largest number of frames whose encoded WAV stays under `threshold_bytes`.
/// Reserves a fixed margin for the 44-byte WAV header plus slack.
pub fn max_frames_for_threshold(info: &WavInfo, threshold_bytes: u64) -> u32 {
    const HEADER_AND_SLACK: u64 = 1024;
    let bpf = info.bytes_per_frame().max(1) as u64;
    let usable = threshold_bytes.saturating_sub(HEADER_AND_SLACK);
    ((usable / bpf).max(1)) as u32
}

/// PURE: split `total_frames` into overlapping windows no larger than
/// `max_frames`, each overlapping the previous by `overlap_frames`.
///
/// Invariants this guarantees (covered by unit tests):
/// - every chunk has `frames() <= max_frames`
/// - chunks cover `[0, total_frames)` with no gaps
/// - consecutive chunks share exactly `overlap_frames` (except when the tail is
///   shorter than the overlap)
pub fn plan_chunks(total_frames: u32, max_frames: u32, overlap_frames: u32) -> Vec<ChunkSpec> {
    if total_frames == 0 {
        return Vec::new();
    }
    let max_frames = max_frames.max(1);
    // Overlap is capped at half the chunk so stride stays >= max/2: this both
    // guarantees forward progress and bounds the chunk count for degenerate input.
    let overlap = overlap_frames.min(max_frames / 2);

    if total_frames <= max_frames {
        return vec![ChunkSpec { start_frame: 0, end_frame: total_frames }];
    }

    let stride = max_frames - overlap;
    let mut chunks = Vec::new();
    let mut start = 0u32;
    loop {
        let end = (start + max_frames).min(total_frames);
        chunks.push(ChunkSpec { start_frame: start, end_frame: end });
        if end >= total_frames {
            break;
        }
        start += stride;
    }
    chunks
}

/// Read one chunk from `path` and encode it as a standalone WAV in memory.
/// Seeks directly to the chunk's start frame — does not read preceding audio.
pub fn read_chunk_wav(path: &Path, info: &WavInfo, spec: ChunkSpec) -> Result<Vec<u8>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("WAV açılamadı: {}", path.display()))?;
    reader
        .seek(spec.start_frame)
        .context("WAV chunk başlangıcına seek edilemedi")?;

    let out_spec = hound::WavSpec {
        channels: info.channels,
        sample_rate: info.sample_rate,
        bits_per_sample: info.bits_per_sample,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buf = Cursor::new(Vec::<u8>::new());
    {
        let mut writer =
            hound::WavWriter::new(&mut buf, out_spec).context("chunk WAV writer kurulamadı")?;
        let samples_to_read = spec.frames() as usize * info.channels as usize;
        for (i, sample) in reader.samples::<i16>().enumerate() {
            if i >= samples_to_read {
                break;
            }
            let sample = sample.context("WAV sample okunamadı")?;
            writer.write_sample(sample).context("chunk sample yazılamadı")?;
        }
        writer.finalize().context("chunk WAV finalize edilemedi")?;
    }
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(sr: u32, ch: u16) -> WavInfo {
        WavInfo { sample_rate: sr, channels: ch, bits_per_sample: 16, total_frames: 0 }
    }

    #[test]
    fn empty_audio_yields_no_chunks() {
        assert!(plan_chunks(0, 100, 10).is_empty());
    }

    #[test]
    fn single_chunk_when_under_max() {
        let c = plan_chunks(50, 100, 10);
        assert_eq!(c, vec![ChunkSpec { start_frame: 0, end_frame: 50 }]);
    }

    #[test]
    fn exactly_at_threshold_is_one_chunk() {
        let c = plan_chunks(100, 100, 10);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0], ChunkSpec { start_frame: 0, end_frame: 100 });
    }

    #[test]
    fn long_audio_splits_with_overlap_and_covers_everything() {
        let total = 1000;
        let max = 300;
        let overlap = 50;
        let chunks = plan_chunks(total, max, overlap);

        // No chunk exceeds the size cap.
        for c in &chunks {
            assert!(c.frames() <= max, "chunk too big: {:?}", c);
        }
        // Coverage: starts at 0, ends at total, no gaps.
        assert_eq!(chunks.first().unwrap().start_frame, 0);
        assert_eq!(chunks.last().unwrap().end_frame, total);
        for pair in chunks.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            // Next chunk starts inside the previous one → no gap.
            assert!(b.start_frame < a.end_frame, "gap between {:?} and {:?}", a, b);
            // Overlap is exactly `overlap` while there is room for it.
            assert_eq!(a.end_frame - b.start_frame, overlap);
        }
    }

    #[test]
    fn overlap_larger_than_max_still_makes_progress() {
        // Degenerate config must not infinite-loop.
        let chunks = plan_chunks(500, 100, 999);
        assert!(chunks.last().unwrap().end_frame == 500);
        assert!(chunks.len() < 100);
    }

    #[test]
    fn max_frames_for_threshold_respects_size() {
        let i = info(16_000, 1); // 2 bytes/frame
        // 25MB free-tier-ish: ~ (25_000_000 - 1024) / 2 frames
        let f = max_frames_for_threshold(&i, 25_000_000);
        assert!(f > 12_000_000 && f < 12_500_000);
    }
}
