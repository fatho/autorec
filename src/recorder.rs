use std::{time::Duration, sync::Arc};

use tracing::{info, trace};

use crate::{midi::{self, RecordEvent}, app::App};

pub async fn run_recorder(
    app: Arc<App>,
    mut recorder: midi::Recorder,
) -> color_eyre::Result<()> {
    loop {
        info!("Waiting for song to start");
        let event = recorder.next().await?;

        if let Some(event) = event {
            app.start_recording().await;

            let song = record_song(event, &mut recorder).await?;

            app.finish_recording(song).await;
        } else {
            break;
        }
    }
    Ok(())
}

pub async fn record_song(
    mut first_event: RecordEvent,
    recorder: &mut midi::Recorder,
) -> color_eyre::Result<Vec<RecordEvent>> {
    info!("Song started");

    trace!("recorded event {:?}", first_event);
    let start_tick = first_event.timestamp;
    first_event.timestamp = 0;
    let mut events = vec![first_event];

    loop {
        match tokio::time::timeout(Duration::from_secs(5), recorder.next()).await {
            Ok(event) => {
                if let Some(mut event) = event? {
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
                    break;
                }
            }
            Err(_elapsed) => {
                // TODO: rather check if there are still keys down or pedals pressed
                // Nothing played for 5 seconds => end this song
                break;
            }
        }
    }
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
    Ok(events)
}
