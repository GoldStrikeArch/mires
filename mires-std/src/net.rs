//! Networking types (`IpAddr`, `SocketAddr`, ...) from `core::net` plus honest
//! [`io::ErrorKind::Unsupported`] socket stubs matching `wasm32-unknown-unknown`.

use crate::io::{self, Read, Write};
use crate::option;
use crate::time::Duration;
use crate::vec;

pub use core_crate::net::{
    AddrParseError, IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6,
};

fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::unsupported(
        "networking sockets are not supported on JavaScript targets",
    ))
}

/// Possible values which can be passed to [`TcpStream::shutdown`].
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Shutdown {
    /// The reading portion of the stream should be shut down.
    Read,
    /// The writing portion of the stream should be shut down.
    Write,
    /// Both the reading and the writing portions should be shut down.
    Both,
}

/// A trait for objects which can be converted or resolved to one or more [`SocketAddr`] values.
pub trait ToSocketAddrs {
    /// Returned iterator over socket addresses.
    type Iter: Iterator<Item = SocketAddr>;

    /// Converts this object to an iterator of resolved [`SocketAddr`]s.
    fn to_socket_addrs(&self) -> io::Result<Self::Iter>;
}

impl ToSocketAddrs for SocketAddr {
    type Iter = option::IntoIter<SocketAddr>;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        Ok(Some(*self).into_iter())
    }
}

impl ToSocketAddrs for SocketAddrV4 {
    type Iter = option::IntoIter<SocketAddr>;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddr::V4(*self).to_socket_addrs()
    }
}

impl ToSocketAddrs for SocketAddrV6 {
    type Iter = option::IntoIter<SocketAddr>;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddr::V6(*self).to_socket_addrs()
    }
}

impl ToSocketAddrs for (IpAddr, u16) {
    type Iter = option::IntoIter<SocketAddr>;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddr::new(self.0, self.1).to_socket_addrs()
    }
}

impl ToSocketAddrs for (Ipv4Addr, u16) {
    type Iter = option::IntoIter<SocketAddr>;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddrV4::new(self.0, self.1).to_socket_addrs()
    }
}

impl ToSocketAddrs for (Ipv6Addr, u16) {
    type Iter = option::IntoIter<SocketAddr>;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddrV6::new(self.0, self.1, 0, 0).to_socket_addrs()
    }
}

impl ToSocketAddrs for (&str, u16) {
    type Iter = vec::IntoIter<SocketAddr>;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        if let Ok(ip) = self.0.parse::<IpAddr>() {
            return Ok(crate::vec![SocketAddr::new(ip, self.1)].into_iter());
        }
        unsupported()
    }
}

impl ToSocketAddrs for str {
    type Iter = vec::IntoIter<SocketAddr>;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        if let Ok(addr) = self.parse::<SocketAddr>() {
            return Ok(crate::vec![addr].into_iter());
        }
        unsupported()
    }
}

impl<T: ToSocketAddrs + ?Sized> ToSocketAddrs for &T {
    type Iter = T::Iter;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        (**self).to_socket_addrs()
    }
}

/// A TCP stream between a local and a remote socket (unsupported on JS).
#[derive(Debug)]
pub struct TcpStream {
    _private: (),
}

impl TcpStream {
    /// Opens a TCP connection to a remote host (`Unsupported`).
    pub fn connect<A: ToSocketAddrs>(_addr: A) -> io::Result<TcpStream> {
        unsupported()
    }

    /// Opens a TCP connection to a remote host with a timeout (`Unsupported`).
    pub fn connect_timeout(_addr: &SocketAddr, _timeout: Duration) -> io::Result<TcpStream> {
        unsupported()
    }

    /// Returns the socket address of the remote peer.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        unsupported()
    }

    /// Returns the socket address of the local half of this TCP connection.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        unsupported()
    }

    /// Shuts down the read, write, or both halves of this connection.
    pub fn shutdown(&self, _how: Shutdown) -> io::Result<()> {
        unsupported()
    }

    /// Creates a new independently owned handle to the underlying socket.
    pub fn try_clone(&self) -> io::Result<TcpStream> {
        unsupported()
    }

    /// Sets the read timeout on the socket.
    pub fn set_read_timeout(&self, _dur: Option<Duration>) -> io::Result<()> {
        unsupported()
    }

    /// Sets the write timeout on the socket.
    pub fn set_write_timeout(&self, _dur: Option<Duration>) -> io::Result<()> {
        unsupported()
    }

    /// Moves this TCP stream into or out of nonblocking mode.
    pub fn set_nonblocking(&self, _nonblocking: bool) -> io::Result<()> {
        unsupported()
    }
}

impl Read for TcpStream {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        unsupported()
    }
}

impl Read for &TcpStream {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        unsupported()
    }
}

impl Write for TcpStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        unsupported()
    }

    fn flush(&mut self) -> io::Result<()> {
        unsupported()
    }
}

impl Write for &TcpStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        unsupported()
    }

    fn flush(&mut self) -> io::Result<()> {
        unsupported()
    }
}

/// A TCP socket server, listening for connections (unsupported on JS).
#[derive(Debug)]
pub struct TcpListener {
    _private: (),
}

impl TcpListener {
    /// Creates a new `TcpListener` bound to the specified address (`Unsupported`).
    pub fn bind<A: ToSocketAddrs>(_addr: A) -> io::Result<TcpListener> {
        unsupported()
    }

    /// Returns the local socket address of this listener.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        unsupported()
    }

    /// Accepts a new incoming connection from this listener.
    pub fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        unsupported()
    }

    /// Returns an iterator over the connections being received on this listener.
    pub fn incoming(&self) -> Incoming<'_> {
        Incoming { listener: self }
    }

    /// Moves this TCP stream into or out of nonblocking mode.
    pub fn set_nonblocking(&self, _nonblocking: bool) -> io::Result<()> {
        unsupported()
    }
}

/// An iterator that infinitely accepts connections on a [`TcpListener`].
#[derive(Debug)]
pub struct Incoming<'a> {
    listener: &'a TcpListener,
}

impl Iterator for Incoming<'_> {
    type Item = io::Result<TcpStream>;

    fn next(&mut self) -> Option<io::Result<TcpStream>> {
        Some(self.listener.accept().map(|(s, _)| s))
    }
}

/// A UDP socket (unsupported on JS).
#[derive(Debug)]
pub struct UdpSocket {
    _private: (),
}

impl UdpSocket {
    /// Creates a UDP socket from the given address (`Unsupported`).
    pub fn bind<A: ToSocketAddrs>(_addr: A) -> io::Result<UdpSocket> {
        unsupported()
    }

    /// Receives a single datagram message on the socket.
    pub fn recv_from(&self, _buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        unsupported()
    }

    /// Sends data on the socket to the given address.
    pub fn send_to<A: ToSocketAddrs>(&self, _buf: &[u8], _addr: A) -> io::Result<usize> {
        unsupported()
    }

    /// Returns the socket address that this socket was created from.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        unsupported()
    }

    /// Connects this UDP socket to a remote address.
    pub fn connect<A: ToSocketAddrs>(&self, _addr: A) -> io::Result<()> {
        unsupported()
    }

    /// Sends data on the socket to the remote address to which it is connected.
    pub fn send(&self, _buf: &[u8]) -> io::Result<usize> {
        unsupported()
    }

    /// Receives a single datagram message on the socket from the remote address.
    pub fn recv(&self, _buf: &mut [u8]) -> io::Result<usize> {
        unsupported()
    }

    /// Moves this UDP socket into or out of nonblocking mode.
    pub fn set_nonblocking(&self, _nonblocking: bool) -> io::Result<()> {
        unsupported()
    }
}
