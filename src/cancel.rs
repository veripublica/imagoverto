//! Stopping a conversion that is under way.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::{Duration, Instant};

/// A way to stop a conversion before it finishes: from another thread with
/// [`cancel`](Cancel::cancel), or by a deadline ([`Cancel::after`]).
/// imagoverto checks it before decoding and before encoding, and the WebP
/// encoder checks it as it works, so a stopped conversion returns
/// [`Error::Cancelled`](crate::Error) soon and frees its thread. The JPEG XL encoder (jxl-encoder 0.3.4) takes the token
/// but never checks it, so a JPEG XL encode that has started runs to the
/// end; [`Options::max_pixels`](crate::Options) bounds how long that is. The
/// JPEG and PNG encoders and the decoders do not check it either; they are
/// fast next to WebP and JPEG XL.
///
/// Clones share one flag: cancelling any of them cancels all.
#[derive(Debug, Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    deadline: Option<Instant>,
}

impl Cancel {
    /// A token that stops only when cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// A token that also stops once `limit` has passed from now. Not on
    /// `wasm32-unknown-unknown`, which has no clock; cancel from a timer
    /// there instead.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub fn after(limit: Duration) -> Self {
        Cancel {
            flag: Arc::default(),
            deadline: Instant::now().checked_add(limit),
        }
    }

    /// Stop every conversion that holds this token or a clone of it.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// Whether the token has been cancelled or its deadline has passed.
    pub fn is_cancelled(&self) -> bool {
        if self.flag.load(Ordering::Relaxed) {
            return true;
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return true;
        }
        false
    }

    /// `Err(Cancelled)` once the token has stopped.
    pub(crate) fn check(&self) -> Result<(), crate::Error> {
        if self.is_cancelled() {
            Err(crate::Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Two tokens are equal when they share a flag and a deadline.
impl PartialEq for Cancel {
    fn eq(&self, other: &Self) -> bool {
        let same = Arc::ptr_eq(&self.flag, &other.flag);
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        let same = same && self.deadline == other.deadline;
        same
    }
}

impl Eq for Cancel {}

/// The trait the codecs check for cancellation.
impl enough::Stop for Cancel {
    fn check(&self) -> Result<(), enough::StopReason> {
        if self.flag.load(Ordering::Relaxed) {
            return Err(enough::StopReason::Cancelled);
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(enough::StopReason::TimedOut);
        }
        Ok(())
    }
}
