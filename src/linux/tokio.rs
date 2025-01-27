use std::io;
use std::mem::{size_of_val, MaybeUninit};

use ::tokio::io::unix::{AsyncFd, AsyncFdReadyGuard};
use ::tokio::runtime;
use ::tokio::select;
use heveanly::errno::EAGAIN;
use heveanly::{AsUninitBytes, Fd};

use super::{signals_all, signals_benign, signals_deadly, signals_new, Signal};

async fn read_sigfd(
   mut guard: AsyncFdReadyGuard<'_, Fd>,
   info: &mut MaybeUninit<libc::signalfd_siginfo>,
) -> Option<io::Result<Signal>> {
   match guard.get_inner().read(info.as_uninit_bytes_mut()) {
      Ok(len) => Some(match len == size_of_val(info) {
         true => Ok(unsafe { info.assume_init_ref() }.ssi_signo as Signal),
         false => Err(io::ErrorKind::InvalidData.into()),
      }),
      Err(EAGAIN) => {
         guard.clear_ready();
         None
      },
      Err(ec) => Some(Err(ec.into())),
   }
}

async fn next(sigfd: &AsyncFd<Fd>) -> io::Result<Signal> {
   let mut info = MaybeUninit::<libc::signalfd_siginfo>::uninit();
   loop {
      match read_sigfd(sigfd.readable().await?, &mut info).await {
         None => continue,
         Some(r) => return r,
      }
   }
}

async fn next_with_sigint(sigint_efd: &AsyncFd<Fd>, sigfd: &AsyncFd<Fd>) -> io::Result<Signal> {
   let mut info = MaybeUninit::<libc::signalfd_siginfo>::uninit();
   loop {
      select! {
         g = sigint_efd.readable() => {
            let mut guard = g?;
            match guard.get_inner().read(MaybeUninit::<[u8; 8]>::uninit().as_uninit_bytes_mut()) {
               Ok(_) => return Ok(libc::SIGINT),
               Err(EAGAIN) => {
                  guard.clear_ready();
                  continue;
               },
               Err(e) => return Err(e.into()),
            }
         },
         g = sigfd.readable() => match read_sigfd(g?, &mut info).await {
            None => continue,
            Some(r) => return r,
         },
      }
   }
}

enum Era {
   Bc(super::Signals),
   Ad { sigint_efd: Option<AsyncFd<Fd>>, sigfd: AsyncFd<Fd> },
}

// `size_of::<AsyncFd<Fd>>() == size_of::<Option<AsyncFd<Fd>>>()` on stable
// and nightly, currently at least, but I don't want to static assert it
// since I don't care enough to break the build if it ever stops being the
// case. If only there were a `static_warn`...
pub struct Signals {
   era: Era,
}

impl Signals {
   fn from_sigset(sigs: &mut libc::sigset_t) -> Self {
      if runtime::Handle::try_current().is_ok() {
         panic!("`macluhan::tokio::Signals` must be created before starting the Tokio runtime");
      }
      Self { era: Era::Bc(super::Signals::from_sigset(sigs)) }
   }

   pub fn new(sigs: &[Signal]) -> Self {
      signals_new(sigs, Self::from_sigset)
   }

   pub fn all() -> Self {
      signals_all(Self::from_sigset)
   }

   pub fn deadly() -> Self {
      signals_deadly(Self::from_sigset)
   }

   pub fn benign() -> Self {
      signals_benign(Self::from_sigset)
   }

   async fn init_and_next(&mut self, sigfd: Fd, sigint_efd: i32) -> io::Result<Signal> {
      let sigfd = AsyncFd::new(sigfd)?;
      let (sig, sigint_efd) = if sigint_efd < 0 {
         (next(&sigfd).await, None)
      } else {
         let fd = AsyncFd::new(unsafe { Fd::new_unchecked(sigint_efd) })?;
         (next_with_sigint(&fd, &sigfd).await, Some(fd))
      };
      self.era = Era::Ad { sigint_efd, sigfd };
      sig
   }

   // Too lazy to implement `Stream`, and let's be real--the only place
   // where this is ever going is into a `select!`.
   pub async fn next(&mut self) -> io::Result<Signal> {
      match &self.era {
         Era::Bc(s) => self.init_and_next(s.sigfd, s.sigint_efd).await,
         Era::Ad { sigint_efd: None, sigfd } => next(sigfd).await,
         Era::Ad { sigint_efd: Some(sigint_efd), sigfd } => {
            next_with_sigint(sigint_efd, sigfd).await
         },
      }
   }
}

impl Drop for Signals {
   fn drop(&mut self) {
      if let Era::Ad { sigfd, .. } = &self.era {
         let _ = sigfd.get_ref().close();
      }
   }
}
