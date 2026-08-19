use std::{io, sync::Arc};

use cch_auth::ManagerPin;
use thiserror::Error;

use crate::DaemonCore;

pub struct DaemonServers {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    manager: std::thread::JoinHandle<()>,
    #[cfg(any(target_os = "linux", target_os = "android"))]
    bridge: std::thread::JoinHandle<()>,
}

impl DaemonServers {
    pub fn start(core: Arc<DaemonCore>, pin: ManagerPin) -> Result<Self, ServerError> {
        platform::start(core, pin)
    }

    pub fn wait(self) -> Result<(), ServerError> {
        platform::wait(self)
    }
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("socket io failed: {0}")]
    Io(#[from] io::Error),
    #[error("server thread stopped unexpectedly")]
    ThreadStopped,
    #[error("abstract Unix sockets are unsupported on this host")]
    UnsupportedPlatform,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod platform {
    use std::{
        io::{self, Write},
        mem::{self, offset_of},
        os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        os::unix::net::UnixStream,
        ptr,
        sync::Arc,
        thread,
    };

    use cch_auth::{ManagerPin, peer_credentials};
    use cch_wire::{
        BRIDGE_SOCKET_NAME, BridgeEvent, ChannelKind, MANAGER_SOCKET_NAME, Request,
        RequestEnvelope, Response, ResponseEnvelope,
    };
    use tracing::{debug, warn};

    use super::{DaemonServers, ServerError};
    use crate::{DaemonCore, read_json_frame, write_json_frame};

    pub(super) fn start(
        core: Arc<DaemonCore>,
        pin: ManagerPin,
    ) -> Result<DaemonServers, ServerError> {
        let manager_listener = AbstractListener::bind(MANAGER_SOCKET_NAME)?;
        let bridge_listener = AbstractListener::bind(BRIDGE_SOCKET_NAME)?;

        let manager_core = Arc::clone(&core);
        let manager = thread::Builder::new()
            .name("ct-manager-listener".to_owned())
            .spawn(move || manager_accept_loop(manager_listener, manager_core, pin))?;
        let bridge = thread::Builder::new()
            .name("ct-bridge-listener".to_owned())
            .spawn(move || bridge_accept_loop(bridge_listener, core))?;
        Ok(DaemonServers { manager, bridge })
    }

    pub(super) fn wait(servers: DaemonServers) -> Result<(), ServerError> {
        servers
            .manager
            .join()
            .map_err(|_| ServerError::ThreadStopped)?;
        servers
            .bridge
            .join()
            .map_err(|_| ServerError::ThreadStopped)?;
        Err(ServerError::ThreadStopped)
    }

    fn manager_accept_loop(listener: AbstractListener, core: Arc<DaemonCore>, pin: ManagerPin) {
        loop {
            match listener.accept() {
                Ok(stream) => {
                    let core = Arc::clone(&core);
                    let pin = pin.clone();
                    if let Err(error) = thread::Builder::new()
                        .name("ct-manager-client".to_owned())
                        .spawn(move || {
                            if let Err(error) = handle_manager(stream, &core, &pin) {
                                debug!(%error, "manager client disconnected");
                            }
                        })
                    {
                        warn!(%error, "failed to start manager client thread");
                    }
                }
                Err(error) => warn!(%error, "manager socket accept failed"),
            }
        }
    }

    fn handle_manager(
        mut stream: UnixStream,
        core: &DaemonCore,
        pin: &ManagerPin,
    ) -> io::Result<()> {
        let credentials = peer_credentials(stream.as_raw_fd()).map_err(permission_denied)?;
        core.authenticate_uid(credentials.uid, pin)
            .map_err(permission_denied)?;

        match read_json_frame::<_, ChannelKind>(&mut stream)? {
            ChannelKind::Control => manager_control(stream, core),
            ChannelKind::Subscribe => manager_subscribe(stream, core),
        }
    }

    fn manager_control(mut stream: UnixStream, core: &DaemonCore) -> io::Result<()> {
        loop {
            let envelope = match read_json_frame::<_, RequestEnvelope>(&mut stream) {
                Ok(envelope) => envelope,
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(error) => return Err(error),
            };
            if let Request::OpenPayload { id } = &envelope.request {
                match core.open_payload_fd(id) {
                    Ok((fd, opened)) => {
                        let response = ResponseEnvelope::ok(
                            envelope.seq,
                            Response::PayloadOpened { payload: opened },
                        );
                        send_json_frame_with_fd(&mut stream, &response, fd.as_raw_fd())?;
                    }
                    Err(error) => {
                        write_json_frame(&mut stream, &ResponseEnvelope::err(envelope.seq, error))?;
                    }
                }
            } else {
                write_json_frame(&mut stream, &core.dispatch(envelope))?;
            }
        }
    }

    fn manager_subscribe(mut stream: UnixStream, core: &DaemonCore) -> io::Result<()> {
        let events = core.subscribe();
        while let Ok(event) = events.recv() {
            write_json_frame(&mut stream, &event)?;
        }
        Ok(())
    }

    fn bridge_accept_loop(listener: AbstractListener, core: Arc<DaemonCore>) {
        loop {
            match listener.accept() {
                Ok(stream) => {
                    if let Err(error) = handle_bridge(stream, &core) {
                        debug!(%error, "system bridge disconnected");
                    }
                    core.bridge().detach();
                }
                Err(error) => warn!(%error, "bridge socket accept failed"),
            }
        }
    }

    fn handle_bridge(mut stream: UnixStream, core: &DaemonCore) -> io::Result<()> {
        let credentials = peer_credentials(stream.as_raw_fd()).map_err(permission_denied)?;
        if !matches!(credentials.uid, 0 | 1_000) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("bridge peer uid {} is not root/system", credentials.uid),
            ));
        }

        let first = read_json_frame::<_, BridgeEvent>(&mut stream)?;
        if !matches!(first, BridgeEvent::Hello { .. }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bridge first frame is not hello",
            ));
        }
        let commands = core.bridge().attach().map_err(invalid_data)?;
        core.bridge().handle_event(first);

        let mut writer = stream.try_clone()?;
        let writer_thread = thread::Builder::new()
            .name("ct-bridge-writer".to_owned())
            .spawn(move || {
                while let Ok(command) = commands.recv() {
                    if write_json_frame(&mut writer, &command).is_err() {
                        break;
                    }
                }
            })?;

        loop {
            match read_json_frame::<_, BridgeEvent>(&mut stream) {
                Ok(event) => core.bridge().handle_event(event),
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(error) => {
                    core.bridge().detach();
                    let _ = writer_thread.join();
                    return Err(error);
                }
            }
        }
        core.bridge().detach();
        let _ = writer_thread.join();
        Ok(())
    }

    fn send_json_frame_with_fd<T: serde::Serialize>(
        stream: &mut UnixStream,
        value: &T,
        passed_fd: RawFd,
    ) -> io::Result<()> {
        let body = serde_json::to_vec(value).map_err(invalid_data)?;
        let frame = cch_wire::encode_frame(&body).map_err(invalid_data)?;
        let fd_bytes = u32::try_from(mem::size_of::<RawFd>())
            .map_err(|_| invalid_data("file descriptor size overflow"))?;
        // SAFETY: CMSG_SPACE is a pure size calculation for one descriptor.
        let control_len = unsafe { libc::CMSG_SPACE(fd_bytes) } as usize;
        let mut control = vec![0_u8; control_len];
        let mut io_vector = libc::iovec {
            iov_base: frame.as_ptr().cast_mut().cast(),
            iov_len: frame.len(),
        };
        // SAFETY: zero is a valid initial state for msghdr before fields are assigned.
        let mut message: libc::msghdr = unsafe { mem::zeroed() };
        message.msg_iov = ptr::addr_of_mut!(io_vector);
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = control.len();

        // SAFETY: message owns writable ancillary storage of CMSG_SPACE bytes.
        let header = unsafe { libc::CMSG_FIRSTHDR(&message) };
        if header.is_null() {
            return Err(invalid_data("failed to allocate SCM_RIGHTS header"));
        }
        // SAFETY: header points inside control and has space for one RawFd.
        unsafe {
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(fd_bytes) as usize;
            ptr::write(libc::CMSG_DATA(header).cast::<RawFd>(), passed_fd);
            message.msg_controllen = (*header).cmsg_len;
        }

        // SAFETY: all message buffers are live for the call, and the kernel copies
        // both payload and descriptor reference before returning.
        let sent = unsafe { libc::sendmsg(stream.as_raw_fd(), &message, libc::MSG_NOSIGNAL) };
        if sent < 0 {
            return Err(io::Error::last_os_error());
        }
        let sent = usize::try_from(sent).map_err(|_| invalid_data("negative send length"))?;
        if sent < frame.len() {
            stream.write_all(&frame[sent..])?;
        }
        Ok(())
    }

    struct AbstractListener {
        fd: OwnedFd,
    }

    impl AbstractListener {
        fn bind(name: &str) -> io::Result<Self> {
            if name.is_empty() || name.len() + 1 > 108 {
                return Err(invalid_data("invalid abstract socket name"));
            }
            // SAFETY: socket has no pointer arguments and returns a fresh descriptor.
            let raw_fd =
                unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
            if raw_fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: raw_fd is fresh and is transferred exactly once.
            let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
            // SAFETY: zero is the correct initial representation for sockaddr_un.
            let mut address: libc::sockaddr_un = unsafe { mem::zeroed() };
            address.sun_family = libc::AF_UNIX as libc::sa_family_t;
            for (slot, byte) in address.sun_path[1..].iter_mut().zip(name.bytes()) {
                *slot = byte as libc::c_char;
            }
            let address_length = offset_of!(libc::sockaddr_un, sun_path) + 1 + name.len();
            let address_length = libc::socklen_t::try_from(address_length)
                .map_err(|_| invalid_data("socket address length overflow"))?;
            // SAFETY: address points to initialized storage and length includes the
            // leading NUL plus the exact abstract name bytes.
            if unsafe {
                libc::bind(
                    fd.as_raw_fd(),
                    ptr::addr_of!(address).cast::<libc::sockaddr>(),
                    address_length,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: fd is a bound stream socket.
            if unsafe { libc::listen(fd.as_raw_fd(), 32) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { fd })
        }

        fn accept(&self) -> io::Result<UnixStream> {
            // SAFETY: listener is valid; null address pointers explicitly discard
            // the peer pathname; the returned descriptor is fresh.
            let accepted = unsafe {
                libc::accept4(
                    self.fd.as_raw_fd(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    libc::SOCK_CLOEXEC,
                )
            };
            if accepted < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: accepted is a fresh connected Unix stream descriptor.
            Ok(unsafe { UnixStream::from_raw_fd(accepted) })
        }
    }

    fn permission_denied(error: impl std::fmt::Display) -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, error.to_string())
    }

    fn invalid_data(error: impl std::fmt::Display) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, error.to_string())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
mod platform {
    use std::sync::Arc;

    use cch_auth::ManagerPin;

    use super::{DaemonServers, ServerError};
    use crate::DaemonCore;

    pub(super) fn start(
        _core: Arc<DaemonCore>,
        _pin: ManagerPin,
    ) -> Result<DaemonServers, ServerError> {
        Err(ServerError::UnsupportedPlatform)
    }

    pub(super) fn wait(_servers: DaemonServers) -> Result<(), ServerError> {
        Err(ServerError::UnsupportedPlatform)
    }
}
