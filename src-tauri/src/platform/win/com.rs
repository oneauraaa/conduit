//! COM and WinRT plumbing — the Windows counterpart of `mac/cf.rs`.

use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
};

/// Initialises COM for the current thread, uninitialising on drop.
///
/// `CoInitializeEx` returns `S_FALSE` when the thread already has an apartment
/// and `RPC_E_CHANGED_MODE` when it has one of a *different* kind. Both mean
/// "COM is usable here" and neither is an error — treating them as one would
/// make every other screenshot fail with no discernible pattern, because tokio
/// hands out blocking threads from a pool and some of them have been used
/// before.
#[allow(dead_code)]
pub struct Com {
    owned: bool,
}

impl Com {
    pub fn init() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // Only uninitialise what this guard actually initialised.
        Com { owned: hr.is_ok() }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.owned {
            unsafe { CoUninitialize() };
        }
    }
}
