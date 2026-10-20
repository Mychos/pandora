//! Audio playback for a drive document: Symphonia decodes the bytes, CPAL plays
//! them. The player belongs to the screen, not to the vault, so it is dropped
//! the moment the entry it was opened for is no longer selected.

use std::io::Cursor;
use std::time::Duration;

pub struct Audio {
    pub entry: String,
    device: Option<Session>,
    pub total: Option<Duration>,
    pub failed: Option<String>,
}

struct Session {
    player: rodio::Player,
    /// Dropping the sink stops the stream, so it travels with the player.
    _sink: rodio::MixerDeviceSink,
}

impl Audio {
    /// Decode `bytes` and queue it, held on the first frame: a preview never
    /// starts playing on its own. `level` is 0..=1, `muted` silences without
    /// losing it.
    pub fn open(entry: String, bytes: &[u8], level: f32, muted: bool) -> Self {
        let mut audio = Self {
            entry,
            device: None,
            total: None,
            failed: None,
        };
        let sink = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(sink) => sink,
            Err(e) => {
                audio.failed = Some(e.to_string());
                return audio;
            }
        };
        let source = match decode(bytes) {
            Ok(source) => source,
            Err(e) => {
                audio.failed = Some(e);
                return audio;
            }
        };
        audio.total = rodio::Source::total_duration(&source);
        let player = rodio::Player::connect_new(sink.mixer());
        player.pause();
        player.set_volume(if muted { 0.0 } else { level.clamp(0.0, 1.0) });
        player.append(source);
        audio.device = Some(Session {
            player,
            _sink: sink,
        });
        audio
    }

    /// A player with no device, for layout tests to draw against.
    #[cfg(test)]
    pub fn detached(entry: String, total: Option<Duration>) -> Self {
        Self {
            entry,
            device: None,
            total,
            failed: None,
        }
    }

    pub fn playing(&self) -> bool {
        self.device
            .as_ref()
            .is_some_and(|s| !s.player.is_paused() && !s.player.empty())
    }

    /// The queue is drained: the next play starts the document over.
    pub fn ended(&self) -> bool {
        self.device.as_ref().is_some_and(|s| s.player.empty())
    }

    pub fn toggle(&self) {
        let Some(session) = self.device.as_ref() else {
            return;
        };
        if session.player.is_paused() {
            session.player.play();
        } else {
            session.player.pause();
        }
    }

    pub fn position(&self) -> Duration {
        self.device
            .as_ref()
            .map_or(Duration::ZERO, |s| s.player.get_pos())
    }

    pub fn set_volume(&self, level: f32, muted: bool) {
        if let Some(session) = self.device.as_ref() {
            session
                .player
                .set_volume(if muted { 0.0 } else { level.clamp(0.0, 1.0) });
        }
    }

    /// Move the playhead. A drained queue has nothing to seek, so the document is
    /// queued again first: the bar stays usable after the last second.
    pub fn seek(&mut self, bytes: &[u8], to: Duration) {
        let to = self.total.map_or(to, |total| to.min(total));
        let Some(session) = self.device.as_mut() else {
            return;
        };
        if session.player.empty() {
            match decode(bytes) {
                Ok(source) => session.player.append(source),
                Err(_) => return,
            }
        }
        let _ = session.player.try_seek(to);
    }

    /// Nudge the playhead, never past either end.
    pub fn skip(&mut self, bytes: &[u8], seconds: f32) {
        let to = target(self.position(), seconds, self.total);
        self.seek(bytes, to);
    }
}

/// Decode the bytes with the settings the bar needs: `byte_len` and `seekable` are
/// what give the duration and let the playhead move in both directions (without
/// them Symphonia reports a non-seekable source and only forward seeks may work).
/// Coarse seeking keeps mp3 usable — it lacks a time base, which the accurate mode
/// refuses. `ponytail: coarse seek rounds to the keyframe; switch to accurate if a
/// sample-exact position ever matters`.
fn decode(bytes: &[u8]) -> Result<rodio::Decoder<Cursor<Vec<u8>>>, String> {
    rodio::Decoder::builder()
        .with_data(Cursor::new(bytes.to_vec()))
        .with_byte_len(bytes.len() as u64)
        .with_seekable(true)
        .with_coarse_seek(true)
        .build()
        .map_err(|e| e.to_string())
}

fn target(played: Duration, seconds: f32, total: Option<Duration>) -> Duration {
    let seconds = (played.as_secs_f32() + seconds).max(0.0);
    let seconds = total.map_or(seconds, |total| seconds.min(total.as_secs_f32()));
    Duration::from_secs_f32(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;

    /// The decoder the bar rides on must know its length (a real duration, not an
    /// animated guess) and must seek backwards, which the default cursor settings
    /// refuse.
    #[test]
    fn an_mp3_reports_its_duration_and_seeks_backwards() {
        let mut source =
            decode(include_bytes!("../../../tests/audio.mp3")).expect("the fixture decodes");
        assert!(
            Source::total_duration(&source).is_some(),
            "byte_len gives the decoder a duration"
        );
        assert!(source.try_seek(Duration::from_secs(1)).is_ok());
        assert!(
            source.try_seek(Duration::from_millis(200)).is_ok(),
            "a seek back is not refused"
        );
    }

    #[test]
    fn a_skip_stops_at_both_ends() {
        let total = Some(Duration::from_secs(60));
        assert_eq!(target(Duration::ZERO, -15.0, total), Duration::ZERO);
        assert_eq!(
            target(Duration::from_secs(10), -15.0, total),
            Duration::ZERO
        );
        assert_eq!(
            target(Duration::from_secs(50), 15.0, total),
            Duration::from_secs(60)
        );
        assert_eq!(
            target(Duration::from_secs(20), 15.0, total),
            Duration::from_secs(35)
        );
        assert_eq!(
            target(Duration::from_secs(20), 15.0, None),
            Duration::from_secs(35)
        );
    }
}
