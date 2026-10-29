use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

pub enum AnyStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AnyStream {
    pub fn is_tls(&self) -> bool {
        matches!(self, AnyStream::Tls(_))
    }

    pub fn raw_fd(&self) -> Option<i32> {
        #[cfg(unix)]
        match self {
            AnyStream::Plain(s) => Some(s.as_raw_fd()),
            AnyStream::Tls(s) => Some(s.get_ref().0.as_raw_fd()),
        }
        #[cfg(not(unix))]
        None
    }
}

impl AsyncRead for AnyStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AnyStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            AnyStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for AnyStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            AnyStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            AnyStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AnyStream::Plain(s) => Pin::new(s).poll_flush(cx),
            AnyStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AnyStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            AnyStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}
