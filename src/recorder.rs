use std::{collections::HashSet, sync::Arc, time::Duration};

use tracing::{info, trace};

use crate::{
    app::Shared,
    midi::{self, RecordEvent},
};

pub async fn run_recorder(app: Arc<Shared>, mut recorder: midi::Recorder) -> color_eyre::Result<()> {
    loop {
        info!("Waiting for song to start");
        let event = recorder.next().await?;

        if let Some(event) = event {
            app.start_recording().await;

            let (song, stop_reason) = record_song(event, &mut recorder).await?;

            app.finish_recording(song).await;

            if let StopReason::Disconnect = stop_reason {
                info!("Recording device has been disconnected");
                break;
            }
        } else {
            break;
        }
    }
    Ok(())
}

/// Describes what caused the end of the recording.
pub enum StopReason {
    /// Pianist was idle for too long
    Idle,
    /// Device got disconnected/turned off
    Disconnect,
}

pub async fn record_song(
    mut first_event: RecordEvent,
    recorder: &mut midi::Recorder,
) -> color_eyre::Result<(Vec<RecordEvent>, StopReason)> {
    info!("Song started");

    // Keeping track of keyboard state for idle-detection
    let mut keyboard_state = KeyboardState::new();
    keyboard_state.update(&first_event);

    const IDLE_TIMEOUT: Duration = Duration::from_secs(5);
    const MAX_IDLE_PERIODS: usize = 6;
    let mut idle_periods = 0;

    // Initialize recording
    trace!("recorded event {:?}", first_event);
    let start_tick = first_event.timestamp;
    first_event.timestamp = 0;
    let mut events = vec![first_event];

    // Keep recording until idle
    let stop_reason = loop {
        match tokio::time::timeout(IDLE_TIMEOUT, recorder.next()).await {
            Ok(event) => {
                if let Some(mut event) = event? {
                    // Update idle detection
                    keyboard_state.update(&event);
                    idle_periods = 0;

                    // Normalize timestamps relative to first event of this song
                    event.timestamp -= start_tick;
                    let reltime = recorder.tick_to_duration(event.timestamp);
                    trace!(
                        "recorded event {:?} at {:.3}s",
                        event,
                        reltime.as_secs_f64()
                    );
                    events.push(event);
                } else {
                    break StopReason::Disconnect;
                }
            }
            Err(_elapsed) => {
                if keyboard_state.is_idle() {
                    break StopReason::Idle;
                } else {
                    idle_periods += 1;

                    // Emergency shutoff (in case state got corrupted)
                    if idle_periods >= MAX_IDLE_PERIODS {
                        break StopReason::Idle;
                    }
                }
            }
        }
    };
    // Ticks are already normalized here
    let last_tick = events
        .last()
        .expect("we have at least `first_event`")
        .timestamp;
    let duration = recorder.tick_to_duration(last_tick);
    info!(
        "Song ended, duration {:.3}s, {} events",
        duration.as_secs_f64(),
        events.len()
    );

    // TODO: stream events to disk - do not keep them in memory
    Ok((events, stop_reason))
}

struct KeyboardState {
    sustain_channels: HashSet<u8>,
    pressed_keys: HashSet<(u8, u8)>,
}

impl KeyboardState {
    fn update(&mut self, event: &RecordEvent) {
        match event.payload {
            midi::MidiEvent::NoteOn { channel, note, .. } => {
                self.pressed_keys.insert((channel, note));
            }
            midi::MidiEvent::NoteOff { channel, note } => {
                self.pressed_keys.remove(&(channel, note));
            }
            midi::MidiEvent::ControlChange {
                channel,
                controller,
                value,
            } => {
                if controller == 64 {
                    // Sustain
                    if value >= 64 {
                        self.sustain_channels.insert(channel);
                    } else {
                        self.sustain_channels.remove(&channel);
                    }
                }
            }
        }
    }

    fn is_idle(&self) -> bool {
        self.sustain_channels.is_empty() && self.pressed_keys.is_empty()
    }

    fn new() -> Self {
        Self {
            sustain_channels: HashSet::new(),
            pressed_keys: HashSet::new(),
        }
    }
}
