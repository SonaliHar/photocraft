//! Only local Unix X11 sockets. A single absolute deadline bounds the entire request.
use async_io::{Async, Timer};
use futures_lite::future::{block_on, race};
use rustix::{
    event::{PollFd, PollFlags, poll},
    fd::AsFd,
    io::Errno,
    net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, recvmsg, sendmsg},
};
use std::{
    io::{self, IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use x11rb::{
    rust_connection::{DefaultStream, PollMode, Stream},
    utils::RawFdContainer,
};

#[derive(Clone, Debug)]
pub(crate) struct Deadline {
    inner: Arc<Mutex<(Instant, Arc<AtomicBool>)>>,
}
impl Deadline {
    pub(crate) fn new(at: Instant) -> Self {
        Self { inner: Arc::new(Mutex::new((at, Arc::new(AtomicBool::new(false))))) }
    }
    pub(crate) fn set(&self, at: Instant, cancel: Arc<AtomicBool>) -> io::Result<()> {
        *self.inner.lock().map_err(|_| io::Error::other("deadline lock poisoned"))? = (at, cancel);
        Ok(())
    }
    fn remaining(&self) -> io::Result<std::time::Duration> {
        let lock = self.inner.lock().map_err(|_| io::Error::other("deadline lock poisoned"))?;
        if lock.1.load(Ordering::Acquire) {
            return Err(timeout());
        }
        lock.0.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or_else(timeout)
    }
}
fn timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "X11 owner request deadline")
}

#[derive(Debug)]
pub(crate) struct DeadlineStream {
    inner: DefaultStream,
    deadline: Deadline,
}
impl DeadlineStream {
    pub(crate) fn from_unix(stream: UnixStream, deadline: Deadline) -> io::Result<(Self, (x11rb::reexports::x11rb_protocol::xauth::Family, Vec<u8>))> {
        let (inner, peer) = DefaultStream::from_unix_stream(stream)?;
        Ok((Self { inner, deadline }, peer))
    }
    pub(crate) fn connect(path: &str, deadline: Deadline) -> io::Result<(Self, (x11rb::reexports::x11rb_protocol::xauth::Family, Vec<u8>))> {
        let until = Instant::now() + deadline.remaining()?;
        // async-io explicitly supports Linux abstract Unix names beginning with NUL.
        let attempt = |name: String| {
            block_on(race(async { Async::<UnixStream>::connect(name).await?.into_inner() }, async {
                Timer::at(until).await;
                Err(timeout())
            }))
        };
        let stream = attempt(format!("\0{path}")).or_else(|_| {
            deadline.remaining()?;
            attempt(path.to_owned())
        })?;
        Self::from_unix(stream, deadline)
    }
}
impl Stream for DeadlineStream {
    fn poll(&self, mode: PollMode) -> io::Result<()> {
        let mut flags = PollFlags::empty();
        if mode.readable() {
            flags |= PollFlags::IN;
        }
        if mode.writable() {
            flags |= PollFlags::OUT;
        }
        loop {
            let duration = self.deadline.remaining()?;
            let ts = rustix::event::Timespec {
                tv_sec: duration.as_secs().try_into().map_err(|_| io::Error::other("deadline too large"))?,
                tv_nsec: duration.subsec_nanos().into(),
            };
            let mut fds = [PollFd::from_borrowed_fd(self.inner.as_fd(), flags)];
            match poll(&mut fds, Some(&ts)) {
                Ok(0) => return Err(timeout()),
                Ok(_) => {
                    self.deadline.remaining()?;
                    return Ok(());
                }
                Err(Errno::INTR) => (),
                Err(e) => return Err(e.into()),
            }
        }
    }
    fn read(&self, buf: &mut [u8], fd_storage: &mut Vec<RawFdContainer>) -> io::Result<usize> {
        loop {
            self.deadline.remaining()?;
            let mut bytes = [MaybeUninit::uninit(); 1024];
            let mut ancillary = RecvAncillaryBuffer::new(&mut bytes);
            match recvmsg(self.inner.as_fd(), &mut [IoSliceMut::new(buf)], &mut ancillary, RecvFlags::CMSG_CLOEXEC) {
                Ok(msg) => {
                    for item in ancillary.drain() {
                        if let RecvAncillaryMessage::ScmRights(fds) = item {
                            fd_storage.extend(fds);
                        }
                    }
                    return Ok(msg.bytes);
                }
                Err(Errno::INTR) => (),
                Err(e) => return Err(e.into()),
            }
        }
    }
    fn write(&self, buf: &[u8], fds: &mut Vec<RawFdContainer>) -> io::Result<usize> {
        loop {
            self.deadline.remaining()?;
            let borrowed = fds.iter().map(AsFd::as_fd).collect::<Vec<_>>();
            let rights = SendAncillaryMessage::ScmRights(&borrowed);
            let mut bytes = vec![MaybeUninit::uninit(); rights.size()];
            let mut ancillary = SendAncillaryBuffer::new(&mut bytes);
            if !fds.is_empty() && !ancillary.push(rights) {
                return Err(io::Error::other("FD ancillary buffer"));
            }
            match sendmsg(self.inner.as_fd(), &[IoSlice::new(buf)], &mut ancillary, SendFlags::NOSIGNAL) {
                Ok(n) => {
                    fds.clear();
                    return Ok(n);
                }
                Err(Errno::INTR) => (),
                Err(e) => return Err(e.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use x11rb::rust_connection::RustConnection;
    #[test]
    fn setup_on_silent_local_socket_has_a_fixed_deadline() {
        let (client, _silent_server) = UnixStream::pair().unwrap();
        let start = Instant::now();
        let (stream, _) = DeadlineStream::from_unix(client, Deadline::new(start + Duration::from_millis(35))).unwrap();
        let err = RustConnection::connect_to_stream(stream, 0).unwrap_err();
        assert!(err.to_string().contains("deadline"), "{err}");
        assert!(start.elapsed() >= Duration::from_millis(30));
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
    }
    #[test]
    fn read_is_nonblocking_and_cancellation_prevents_writes() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let deadline = Deadline::new(Instant::now() + Duration::from_secs(1));
        let cancel = Arc::new(AtomicBool::new(false));
        deadline.set(Instant::now() + Duration::from_secs(1), cancel.clone()).unwrap();
        let (stream, _) = DeadlineStream::from_unix(client, deadline).unwrap();
        assert_eq!(stream.read(&mut [0; 1], &mut vec![]).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        cancel.store(true, Ordering::Release);
        assert_eq!(stream.write(b"x", &mut vec![]).unwrap_err().kind(), io::ErrorKind::TimedOut);
        use std::io::Read;
        server.set_nonblocking(true).unwrap();
        assert_eq!(server.read(&mut [0; 1]).unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }
    #[test]
    fn poll_and_intervening_writes_do_not_extend_deadline() {
        let (client, _server) = UnixStream::pair().unwrap();
        let start = Instant::now();
        let (stream, _) = DeadlineStream::from_unix(client, Deadline::new(start + Duration::from_millis(35))).unwrap();
        stream.poll(PollMode::Writable).unwrap();
        stream.write(b"x", &mut vec![]).unwrap();
        assert_eq!(stream.poll(PollMode::Readable).unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
