use std::path::PathBuf;

use clap::Parser;

/// Program to automatically start MIDI recordings of songs played on an attached MIDI device.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Name of the MIDI client to attach to
    #[clap(short('c'), long)]
    pub midi_client: String,
    /// Path where recorded songs are stored.
    #[clap(short('d'), long)]
    pub song_directory: PathBuf,
}
