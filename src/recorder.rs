
use std::time::{Duration};

use tracing::{info, trace};

use crate::midi::{self, RecordEvent, RecordPayload};

pub async fn run_recorder(mut recorder: midi::Recorder) -> color_eyre::Result<()> {
    loop {
        info!("Waiting for song to start");
        let event = recorder.next().await?;

        if let RecordPayload::RecordEnd = event.payload {
            break;
        }

        record_song(event, &mut recorder).await?;
    }
    Ok(())
}

pub async fn record_song(first_event: RecordEvent, recorder: &mut midi::Recorder) -> color_eyre::Result<()> {
    info!("Song started");

    trace!("recorded event {:?}", first_event);
    let start_tick = first_event.timestamp;
    let mut events = vec![first_event];

    loop {
        match tokio::time::timeout(Duration::from_secs(5), recorder.next()).await {
            Ok(event) => {
                let event = event?;
                let reltime = recorder.tick_to_duration(event.timestamp - start_tick);
                trace!("recorded event {:?} at {:.3}s", event, reltime.as_secs_f64());
                events.push(event);
            }
            Err(_elapsed) => {
                // Nothing played for 5 seconds => end this song
                break;
            },
        }
    }
    let last_tick = events.last().expect("we have at least `first_event`").timestamp;
    let duration = recorder.tick_to_duration(last_tick - start_tick);
    info!("Song ended, duration {:.3}s, {} events", duration.as_secs_f64(), events.len());
    Ok(())
}
