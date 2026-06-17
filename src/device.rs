//! HID transport: discover, open, and exchange raw reports with the keyboard.

use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};

use crate::error::{Error, Result};
use crate::protocol::consts::{
    PRODUCT_IDS, REPORT_ID, REPORT_LEN, USAGE, USAGE_PAGE, VENDOR_ID, cmd,
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

    /// Perform the identity handshake and return the parsed reply.
    pub fn identity(&self) -> Result<Identity> {
        self.write(&crate::protocol::packet::identity())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(data) = self.read(Duration::from_millis(200))? {
                if data.first() == Some(&cmd::IDENTITY) {
                    return Identity::parse(&data).ok_or(Error::NoResponse);
                }
            }
        }
        Err(Error::NoResponse)
    }
}
