// Probe the 0xB7 key-depth stream. Enables tracking, re-requests each cycle,
// and prints the row byte + first values, plus any keys reading above baseline.
use std::time::{Duration, Instant};

use fawnd::device::Device;
use fawnd::protocol::layout::name_of;
use fawnd::protocol::packet;

fn main() -> anyhow::Result<()> {
    let d = Device::open()?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut printed_layout = [false; 3];

    while Instant::now() < deadline {
        d.write(&packet::key_tracking(true))?; // request one round (3 row packets)
        let read_until = Instant::now() + Duration::from_millis(60);
        while Instant::now() < read_until {
            let Some(data) = d.read(Duration::from_millis(30))? else {
                continue;
            };
            if data.first() != Some(&0xB7) || data.len() < 6 {
                continue;
            }
            let row = data[3] as usize;
            let values = &data[4..];

            // Show the raw framing once per row.
            if row < 3 && !printed_layout[row] {
                printed_layout[row] = true;
                println!(
                    "row {row}: data[0..6]={:02x?}  first 12 values={:02x?}",
                    &data[..6.min(data.len())],
                    &values[..12.min(values.len())]
                );
            }

            // Report pressed keys (above rest baseline).
            let base = row * 59;
            let pressed: Vec<String> = values
                .iter()
                .enumerate()
                .filter(|&(_, &v)| v > 6)
                .map(|(i, &v)| {
                    let name = name_of(base + i).unwrap_or("?");
                    format!("{name}={v}")
                })
                .collect();
            if !pressed.is_empty() {
                println!("  pressed: {}", pressed.join(" "));
            }
        }
    }
    Ok(())
}
