//! `fawnd` CLI entry point.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use fawnd::config::Profile;
use fawnd::protocol::byte_to_mm;
use fawnd::Controller;

#[derive(Parser)]
#[command(name = "fawnd", version, about = "DrunkDeer A75 driver / CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print keyboard model, firmware, and toggle state.
    Info,
    /// Set the actuation point (mm) for all keys, or specific named keys.
    Actuation {
        /// Distance in millimetres (0.2 – 3.8).
        mm: f32,
        /// Optional key names (e.g. W A S D). Empty = all keys.
        keys: Vec<String>,
    },
    /// Toggle rapid trigger (and optionally turbo / snap-tap).
    RapidTrigger {
        /// on | off
        #[arg(value_parser = ["on", "off"])]
        state: String,
        /// Also enable turbo (snap-tap).
        #[arg(long)]
        turbo: bool,
    },
    /// Apply a TOML profile.
    Apply {
        /// Path to a profile file.
        path: PathBuf,
    },
    /// Restore default configuration.
    Reset,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fawnd=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let mut ctl = Controller::open()?;

    match cli.command {
        Command::Info => {
            let id = ctl.identity()?;
            println!("model:        {:?}", id.model);
            println!("firmware:     {}", id.firmware_version);
            println!("rapid trigger: {}", id.rapid_trigger);
            println!("turbo:        {}", id.turbo);
        }
        Command::Actuation { mm, keys } => {
            if keys.is_empty() {
                ctl.set_actuation_all(mm);
            } else {
                let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
                ctl.set_actuation(&refs, mm)?;
            }
            ctl.flush_actuation()?;
            println!("actuation set to {} mm", byte_to_mm(fawnd::protocol::mm_to_byte(mm)));
        }
        Command::RapidTrigger { state, turbo } => {
            let on = state == "on";
            ctl.set_rapid_trigger(on, turbo)?;
            println!("rapid trigger: {on}, turbo: {turbo}");
        }
        Command::Apply { path } => {
            let profile = Profile::load(&path)?;
            profile.apply(&mut ctl)?;
            println!("applied profile {}", path.display());
        }
        Command::Reset => {
            ctl.write_defaults()?;
            println!("restored defaults");
        }
    }

    Ok(())
}
