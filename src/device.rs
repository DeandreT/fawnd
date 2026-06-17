//! HID transport: discover, open, and exchange raw reports with the keyboard.

use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};

use crate::error::{Error, Result};
use crate::protocol::consts::{
    PRODUCT_IDS, REPORT_ID, REPORT_LEN, USAGE, USAGE_PAGE, VENDOR_ID,
};
use crate::protocol::packet::Payload;
use crate::protocol::Identity;

/// An opened DrunkDeer device on the vendor config interface.
pub struct Device {
    handle: HidDevice,
}

impl Device {
    /// Open the first connected DrunkDeer keyboard's config interface.
    pub fn open() -> Result<Device> {
        let api = HidApi::new()?;
        let info = api
            .device_list()
            .find(|d| {
                d.vendor_id() == VENDOR_ID
                    && PRODUCT_IDS.contains(&d.product_id())
                    && d.usage_page() == USAGE_PAGE
                    && d.usage() == USAGE
            })
            .ok_or(Error::DeviceNotFound)?;

        let handle = info.open_device(&api)?;
        Ok(Device { handle })
    }

    /// Write a single config payload to the device. The report ID is prepended
    /// and the buffer padded to the full report length automatically.
    pub fn write(&self, payload: &Payload) -> Result<()> {
        let mut report = [0u8; REPORT_LEN];
        report[0] = REPORT_ID;
        report[1..].copy_from_slice(payload);
        self.handle.write(&report)?;
        Ok(())
    }

    /// Read one inbound report, blocking up to `timeout`. Returns the payload
    /// bytes after the report ID, or `None` on timeout.
    pub fn read(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut buf = [0u8; REPORT_LEN];
        let n = self
            .handle
            .read_timeout(&mut buf, timeout.as_millis() as i32)?;
        if n == 0 {
            return Ok(None);
        }
        // hidapi strips the report ID on platforms with numbered reports, but
        // not all; normalize by matching on our known report ID.
        let payload = if buf[0] == REPORT_ID {
            buf[1..n].to_vec()
        } else {
            buf[..n].to_vec()
        };
        Ok(Some(payload))
    }

    /// Discard reports queued from earlier writes (e.g. command echoes) until the
    /// device goes quiet, so a following read sees only fresh replies.
    ///
    /// A short per-read timeout (rather than zero) is important: command echoes
    /// trickle in with a few ms of delay, so a purely non-blocking drain would
    /// miss the later ones and leave them to corrupt the next handshake.
    pub fn drain(&self) -> Result<()> {
        while self.read(Duration::from_millis(15))?.is_some() {}
        Ok(())
    }

    /// Perform the identity handshake and return the parsed reply.
    pub fn identity(&self) -> Result<Identity> {
        // Flush stale echoes first; otherwise we may parse a leftover packet.
        self.drain()?;
        self.write(&crate::protocol::packet::identity())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(data) = self.read(Duration::from_millis(200))? {
                // Skip non-identity packets and stub/echo replies that fail to
                // parse; keep reading until a valid reply or the deadline.
                if let Some(id) = Identity::parse(&data) {
                    return Ok(id);
                }
            }
        }
        Err(Error::NoResponse)
    }
}
