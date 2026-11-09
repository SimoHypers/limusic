//! A WAV file that is still being written, so playback can switch to an instrumental long before
//! it is finished. The header carries the song's real length from the start, so the player knows
//! the duration and can seek; samples land as the separation finishes them, in any order, and
//! [`GrowingWav::ready_from`] says how much can be read from a given byte without waiting.
//!
//! 16-bit stereo at 44.1 kHz: 10 MB a minute, on disk only while the song plays (the cache keeps
//! the FLAC). The file goes when the last holder drops it.

use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

pub const SAMPLE_RATE: u32 = 44_100;
pub const HEADER_LEN: u64 = 44;
const BYTES_PER_FRAME: u64 = 4;

pub struct GrowingWav {
    path: PathBuf,
    file: Mutex<Option<File>>,
    frames: usize,
    /// Byte ranges written so far, sorted and merged. The header is ready from the start.
    ready: Mutex<Vec<(u64, u64)>>,
    /// No more samples will come (the job failed or was cancelled): a reader waiting on a range
    /// that isn't written should give up.
    abandoned: AtomicBool,
    /// Where the stream starts (a byte): see [`Self::servable_from`].
    hold_from: AtomicU64,
}

impl GrowingWav {
    pub fn create(path: &Path, frames: usize) -> io::Result<Self> {
        let data = frames as u64 * BYTES_PER_FRAME;
        if data > u32::MAX as u64 - HEADER_LEN {
            return Err(io::Error::other("too long for a WAV"));
        }
        let mut file = File::create(path)?;
        file.set_len(HEADER_LEN + data)?;
        file.write_all(&header(data as u32))?;
        Ok(Self {
            path: path.to_path_buf(),
            file: Mutex::new(Some(file)),
            frames,
            ready: Mutex::new(vec![(0, HEADER_LEN)]),
            abandoned: AtomicBool::new(false),
            hold_from: AtomicU64::new(HEADER_LEN),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The whole file's length, header included, written or not.
    pub fn len(&self) -> u64 {
        HEADER_LEN + self.frames as u64 * BYTES_PER_FRAME
    }

    pub fn is_empty(&self) -> bool {
        self.frames == 0
    }

    /// Samples from frame `at` on, clamped to 16 bits.
    pub fn write(&self, at: usize, left: &[f32], right: &[f32]) -> io::Result<()> {
        let n = left.len().min(right.len()).min(self.frames.saturating_sub(at));
        if n == 0 {
            return Ok(());
        }
        let mut buf = Vec::with_capacity(n * BYTES_PER_FRAME as usize);
        for i in 0..n {
            for s in [left[i], right[i]] {
                buf.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
            }
        }
        let off = HEADER_LEN + at as u64 * BYTES_PER_FRAME;
        {
            let mut guard = self.file.lock().unwrap_or_else(PoisonError::into_inner);
            let file = guard.as_mut().ok_or_else(|| io::Error::other("closed"))?;
            file.seek(SeekFrom::Start(off))?;
            file.write_all(&buf)?;
        }
        self.mark(off, off + buf.len() as u64);
        Ok(())
    }

    fn mark(&self, a: u64, b: u64) {
        let mut r = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        r.push((a, b));
        r.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(r.len());
        for &(s, e) in r.iter() {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        *r = merged;
    }

    /// The stream starts at `frame` (where the song was playing): everything before it is
    /// written last.
    pub fn set_start(&self, frame: usize) {
        let byte = (HEADER_LEN + frame as u64 * BYTES_PER_FRAME).min(self.len());
        self.hold_from.store(byte, Ordering::SeqCst);
    }

    /// Bytes a reader can be given from `byte` on without waiting: what [`Self::ready_from`]
    /// says, plus, before the stream's start, silence where nothing is written yet.
    ///
    /// mpv reads forward from the top of the file after opening it, before it seeks to where the
    /// song is playing, and its reader can't take the seek while it waits on a read. Holding those
    /// bytes, written last when karaoke went on mid-song, froze playback until the whole job was
    /// done (measured: over a minute). Only what lies ahead of the stream's start is worth waiting
    /// for. The file is created zero-filled, so the silence costs nothing, and it never counts as
    /// ready: a seek into it still goes back to the original.
    pub fn servable_from(&self, byte: u64) -> u64 {
        let ready = self.ready_from(byte);
        let hold = self.hold_from.load(Ordering::SeqCst);
        if ready > 0 || byte >= hold {
            return ready;
        }
        let r = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        let next = r.iter().map(|&(s, _)| s).find(|&s| s > byte).unwrap_or(hold);
        next.min(hold) - byte
    }

    /// Bytes readable from `byte` on without waiting: 0 while that byte isn't written.
    pub fn ready_from(&self, byte: u64) -> u64 {
        let r = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
        r.iter().find(|&&(s, e)| s <= byte && byte < e).map_or(0, |&(_, e)| e - byte)
    }

    /// Seconds of audio readable from `secs` into the song without waiting.
    pub fn ready_secs_from(&self, secs: f64) -> f64 {
        let frame = (secs.max(0.0) * SAMPLE_RATE as f64) as u64;
        let bytes = self.ready_from(HEADER_LEN + frame * BYTES_PER_FRAME);
        bytes as f64 / BYTES_PER_FRAME as f64 / SAMPLE_RATE as f64
    }

    /// Every sample is written.
    pub fn is_complete(&self) -> bool {
        self.ready_from(0) == self.len()
    }

    pub fn abandon(&self) {
        self.abandoned.store(true, Ordering::SeqCst);
    }

    pub fn is_abandoned(&self) -> bool {
        self.abandoned.load(Ordering::SeqCst)
    }
}

impl Drop for GrowingWav {
    fn drop(&mut self) {
        // Closed first: Windows won't delete a file that is still open.
        drop(self.file.lock().unwrap_or_else(PoisonError::into_inner).take());
        let _ = std::fs::remove_file(&self.path);
    }
}

fn header(data: u32) -> [u8; HEADER_LEN as usize] {
    let mut h = [0u8; HEADER_LEN as usize];
    let mut put = |at: usize, b: &[u8]| h[at..at + b.len()].copy_from_slice(b);
    put(0, b"RIFF");
    put(4, &(36 + data).to_le_bytes());
    put(8, b"WAVEfmt ");
    put(16, &16u32.to_le_bytes());
    put(20, &1u16.to_le_bytes()); // PCM
    put(22, &2u16.to_le_bytes());
    put(24, &SAMPLE_RATE.to_le_bytes());
    put(28, &(SAMPLE_RATE * BYTES_PER_FRAME as u32).to_le_bytes());
    put(32, &(BYTES_PER_FRAME as u16).to_le_bytes());
    put(34, &16u16.to_le_bytes());
    put(36, b"data");
    put(40, &data.to_le_bytes());
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_out_of_order_and_read_back_whole() {
        let dir = std::env::temp_dir().join(format!("limusic-growing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("g.wav");
        let frames = 1000;
        let wav = GrowingWav::create(&path, frames).unwrap();
        assert_eq!(wav.len(), HEADER_LEN + 4000);
        assert_eq!(wav.ready_from(0), HEADER_LEN, "only the header to start with");
        assert_eq!(wav.ready_from(HEADER_LEN), 0);
        assert_eq!(wav.servable_from(HEADER_LEN), 0, "nothing to hand out before a start is set");
        // Started at frame 600: before it, unwritten bytes go out as silence, never as ready.
        wav.set_start(600);
        assert_eq!(wav.servable_from(HEADER_LEN), 2400);
        assert_eq!(wav.servable_from(HEADER_LEN + 600 * 4), 0, "the stream itself is waited for");
        assert_eq!(wav.ready_secs_from(0.0), 0.0);

        let ramp =
            |n: usize, from: usize| (0..n).map(|i| (from + i) as f32 / 2000.0).collect::<Vec<_>>();
        // The tail first, as when karaoke starts mid-song.
        wav.write(600, &ramp(400, 600), &ramp(400, 600)).unwrap();
        assert_eq!(wav.ready_from(HEADER_LEN + 600 * 4), 1600);
        assert_eq!(wav.ready_from(HEADER_LEN + 599 * 4), 0);
        assert!((wav.ready_secs_from(600.0 / 44100.0) - 400.0 / 44100.0).abs() < 1e-9);
        assert!(!wav.is_complete());

        wav.write(0, &ramp(600, 0), &ramp(600, 0)).unwrap();
        assert!(wav.is_complete());
        assert_eq!(wav.ready_from(0), wav.len());

        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 44100);
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.duration() as usize, frames);
        let samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(samples[2 * 700], (0.35f32 * 32767.0) as i16);

        drop(wav);
        assert!(!path.exists(), "dropped with its last holder");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
