/// Audio playback stub for pronunciation (future feature).
/// Will support TTS for Greek pronunciation using reconstructed
/// classical or Koine pronunciation systems.

pub struct AudioPlayer {
    _private: (),
}

impl AudioPlayer {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}
