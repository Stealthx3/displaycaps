// SPDX-License-Identifier: LGPL-2.1-or-later
//! DRM hotplug events from the kernel's uevent netlink socket (multicast group 1). Listening needs
//! no privileges and changes nothing; the socket only receives. Inside a sandbox without the host
//! network namespace (Flatpak without `--share=network`, Snap without `hardware-observe`) the
//! kernel delivers nothing, and watches fall back to their other triggers.

use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::time::Duration;

use nix::sys::socket::{
    AddressFamily, NetlinkAddr, SockFlag, SockProtocol, SockType, bind, recv, setsockopt, socket,
    sockopt,
};
use nix::sys::time::TimeVal;

/// What a DRM uevent says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrmEvent {
    /// `change`, `add` or `remove`.
    pub action: String,
    /// The device path, e.g. `/devices/pci0000:00/0000:00:01.0/drm/card0`.
    pub devpath: String,
    /// `HOTPLUG=1`: a connector's state changed.
    pub hotplug: bool,
    /// `CONNECTOR=<id>`, when the driver names the connector.
    pub connector: Option<u32>,
}

/// A bound uevent socket with a receive timeout.
pub(crate) struct Listener(OwnedFd);

impl Listener {
    /// Opens and binds the socket; `poll` is the longest a [`Listener::next`] call blocks.
    pub fn open(poll: Duration) -> io::Result<Self> {
        // A zero receive timeout means "block forever" to the kernel.
        let poll = poll.max(Duration::from_millis(1));
        let fd = socket(
            AddressFamily::Netlink,
            SockType::Datagram,
            SockFlag::SOCK_CLOEXEC,
            SockProtocol::NetlinkKObjectUEvent,
        )?;
        bind(fd.as_fd().as_raw_fd_compat(), &NetlinkAddr::new(0, 1))?;
        let tv = TimeVal::new(
            i64::try_from(poll.as_secs()).unwrap_or(1),
            i64::from(poll.subsec_micros()),
        );
        setsockopt(&fd, sockopt::ReceiveTimeout, &tv)?;
        Ok(Self(fd))
    }

    /// The next DRM event, or `None` when the timeout passed (or a non-DRM event arrived).
    pub fn next(&self) -> io::Result<Option<DrmEvent>> {
        let mut buf = [0u8; 8192];
        match recv(
            self.0.as_fd().as_raw_fd_compat(),
            &mut buf,
            nix::sys::socket::MsgFlags::empty(),
        ) {
            Ok(n) => Ok(parse(&buf[..n])),
            Err(nix::errno::Errno::EAGAIN | nix::errno::Errno::EINTR) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// Raw fd for the nix calls that still take one.
trait RawFdCompat {
    fn as_raw_fd_compat(&self) -> std::os::fd::RawFd;
}

impl RawFdCompat for std::os::fd::BorrowedFd<'_> {
    fn as_raw_fd_compat(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;
        self.as_raw_fd()
    }
}

/// Parses a kernel uevent (`action@devpath\0KEY=value\0...`). Only `SUBSYSTEM=drm` events count.
pub(crate) fn parse(msg: &[u8]) -> Option<DrmEvent> {
    let mut parts = msg.split(|&b| b == 0).filter(|p| !p.is_empty());
    let head = std::str::from_utf8(parts.next()?).ok()?;
    // udev's own re-broadcasts (group 2) start with "libudev"; group 1 is the kernel's.
    let (action, devpath) = head.split_once('@')?;
    let mut ev = DrmEvent {
        action: action.to_string(),
        devpath: devpath.to_string(),
        hotplug: false,
        connector: None,
    };
    let mut drm = false;
    for p in parts {
        let Ok(kv) = std::str::from_utf8(p) else {
            continue;
        };
        match kv.split_once('=') {
            Some(("SUBSYSTEM", "drm")) => drm = true,
            Some(("HOTPLUG", "1")) => ev.hotplug = true,
            Some(("CONNECTOR", v)) => ev.connector = v.parse().ok(),
            _ => {}
        }
    }
    drm.then_some(ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_drm_hotplug() {
        let m = b"change@/devices/pci0000:00/0000:00:01.0/drm/card0\0ACTION=change\0DEVPATH=/devices/pci0000:00/0000:00:01.0/drm/card0\0SUBSYSTEM=drm\0HOTPLUG=1\0CONNECTOR=37\0DEVNAME=/dev/dri/card0\0SEQNUM=2345\0";
        let e = parse(m).unwrap();
        assert_eq!(e.action, "change");
        assert!(e.hotplug);
        assert_eq!(e.connector, Some(37));
        assert!(parse(b"add@/devices/virtual/net/x\0SUBSYSTEM=net\0").is_none());
        assert!(parse(b"garbage").is_none());
    }
}
