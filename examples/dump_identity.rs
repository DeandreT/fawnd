use std::time::Duration;

use fawnd::device::Device;

fn main() -> anyhow::Result<()> {
    let d = Device::open()?;
    d.write(&fawnd::protocol::packet::identity())?;
    for _ in 0..10 {
        if let Some(data) = d.read(Duration::from_millis(300))? {
            if data.first() == Some(&0xA0) {
                println!("len={} bytes={:02x?}", data.len(), data);
                return Ok(());
            }
        }
    }
    println!("no identity reply");
    Ok(())
}
