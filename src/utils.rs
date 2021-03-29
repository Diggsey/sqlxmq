use std::any::Any;
use std::fmt::{self, Debug};
use std::ops::{Deref, DerefMut};

use tokio::task::JoinHandle;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Opaque<T: Any>(pub T);

impl<T: Any> Debug for Opaque<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <dyn Any>::fmt(&self.0, f)
    }
}

impl<T: Any> Deref for Opaque<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Any> DerefMut for Opaque<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A handle to a background task which will be automatically cancelled if
/// the handle is dropped. Extract the inner join handle to prevent this
/// behaviour.
#[derive(Debug)]
pub struct OwnedTask(pub JoinHandle<()>);

impl Drop for OwnedTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
