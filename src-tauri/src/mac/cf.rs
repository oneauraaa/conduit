//! Small Core Foundation helpers.
//!
//! `CFDictionaryGetValue` and `CFArrayGetValueAtIndex` both hand back raw,
//! *borrowed* pointers with no type information. Doing the null check, the
//! retain and the downcast at every call site would bury the window-listing
//! code in ceremony, so it lives here once.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFNumberType, CFRetained, CFString, CFType,
};

/// Looks up `key` and takes ownership of the result.
///
/// CFDictionaryGetValue follows the Get rule (the dictionary keeps owning the
/// value), so we retain before handing it out.
pub fn dict_value(dict: &CFDictionary, key: &str) -> Option<CFRetained<CFType>> {
    let cf_key = CFString::from_str(key);
    let raw = unsafe { dict.value(CFRetained::as_ptr(&cf_key).as_ptr().cast::<c_void>()) };
    let ptr = NonNull::new(raw.cast_mut())?.cast::<CFType>();
    Some(unsafe { CFRetained::retain(ptr) })
}

pub fn dict_string(dict: &CFDictionary, key: &str) -> Option<String> {
    let v = dict_value(dict, key)?;
    v.downcast_ref::<CFString>().map(|s| s.to_string())
}

pub fn dict_i64(dict: &CFDictionary, key: &str) -> Option<i64> {
    let v = dict_value(dict, key)?;
    let n = v.downcast_ref::<CFNumber>()?;
    let mut out: i64 = 0;
    let ok = unsafe {
        n.value(
            CFNumberType::SInt64Type,
            (&mut out as *mut i64).cast::<c_void>(),
        )
    };
    ok.then_some(out)
}

pub fn dict_f64(dict: &CFDictionary, key: &str) -> Option<f64> {
    let v = dict_value(dict, key)?;
    let n = v.downcast_ref::<CFNumber>()?;
    let mut out: f64 = 0.0;
    let ok = unsafe {
        n.value(
            CFNumberType::Float64Type,
            (&mut out as *mut f64).cast::<c_void>(),
        )
    };
    ok.then_some(out)
}

pub fn dict_dict(dict: &CFDictionary, key: &str) -> Option<CFRetained<CFDictionary>> {
    let v = dict_value(dict, key)?;
    v.downcast::<CFDictionary>().ok()
}

/// Retains and returns the element at `index`, or None if it is null.
pub fn array_at(array: &CFArray, index: isize) -> Option<CFRetained<CFType>> {
    let raw = unsafe { array.value_at_index(index) };
    let ptr = NonNull::new(raw.cast_mut())?.cast::<CFType>();
    Some(unsafe { CFRetained::retain(ptr) })
}

/// The raw pointer at `index`, without retaining.
///
/// Used for `AXUIElementRef`, which is a CFType but has no `ConcreteType` impl
/// to downcast through, and which the AX functions want as a bare pointer.
pub fn array_ptr_at(array: &CFArray, index: isize) -> Option<*mut c_void> {
    let raw = unsafe { array.value_at_index(index) };
    (!raw.is_null()).then(|| raw.cast_mut())
}
