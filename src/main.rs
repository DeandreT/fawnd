//! `fawnd` CLI entry point.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use fawnd::config::Profile;
use fawnd::ipc::{self, Request, Response};
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
    /// Talk to the running fawnd daemon.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Show keyboard status and active profile.
    Status,
    /// List profiles available in the store.
    Profiles,
    /// Apply a named profile from the store.
    Apply {
        /// Profile name (without the .toml extension).
        name: String,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fawnd=info".into()),
        )
        .init();

    let cli = Cli::parse();

    // Daemon client commands talk over the socket and never open the device.
    if let Command::Daemon { command } = cli.command {
        return run_daemon_client(command);
    }

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
        Command::Daemon { .. } => unreachable!("handled before opening the device"),
    }

    Ok(())
}

fn run_daemon_client(command: DaemonCommand) -> anyhow::Result<()> {
    let mut client = ipc::Client::connect()?;
    let response = match command {
        DaemonCommand::Status => client.request(&Request::Status)?,
        DaemonCommand::Profiles => client.request(&Request::ListProfiles)?,
        DaemonCommand::Apply { name } => client.request(&Request::ApplyProfile(name))?,
    };

    match response {
        Response::Ok => println!("ok"),
        Response::Status(s) => {
            println!("model:         {}", s.model);
            println!("firmware:      {}", s.firmware);
            println!("rapid trigger: {}", s.rapid_trigger);
            println!("turbo:         {}", s.turbo);
            println!(
                "active profile: {}",
                s.active_profile.as_deref().unwrap_or("(none)")
            );
        }
        Response::Profiles(names) => {
            if names.is_empty() {
                println!("(no profiles in {})", fawnd::daemon::profiles_dir().display());
            } else {
                for name in names {
                    println!("{name}");
                }
            }
        }
        Response::Depths(_) => println!("(depth frame received)"),
        Response::Error(e) => anyhow::bail!("daemon error: {e}"),
    }
    Ok(())
}
