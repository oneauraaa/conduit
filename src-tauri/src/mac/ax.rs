//! Reading the screen through the accessibility tree.
//!
//! This is the tool agents should reach for first. Pixels force a model to
//! guess at text and button edges; the AX tree hands over the actual strings
//! and the actual bounds, so a click lands on the real centre of the real
//! control. It is also dramatically cheaper than a screenshot.

use std::ffi::c_void;

use accessibility_sys::{
    AXUIElementCopyAttributeValue, AXUIElementCreateApplication, AXUIElementRef, AXValueGetValue,
    kAXChildrenAttribute, kAXDescriptionAttribute, kAXErrorSuccess, kAXPositionAttribute,
    kAXRoleAttribute, kAXSizeAttribute, kAXTitleAttribute, kAXValueAttribute,
    kAXValueTypeCGPoint, kAXValueTypeCGSize,
};
use objc2_app_kit::NSWorkspace;
use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType, CGPoint, CGSize};
use serde::Serialize;

/// Depth cap. Real UIs nest deeply (a Finder window is ~15 levels); beyond this
/// the payload grows faster than its usefulness.
const MAX_DEPTH: usize = 18;
/// Hard cap on returned elements, so one pathological app can't blow the
/// model's context window.
const MAX_ELEMENTS: usize = 400;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub role: String,
    /// Whatever label the control actually presents: title, value or description.
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Centre point, ready to hand straight to `click`.
    pub center_x: f64,
    pub center_y: f64,
}

fn attr_ptr(element: AXUIElementRef, attr: &str) -> Option<*mut c_void> {
    let key = CFString::from_str(attr);
    let mut value: *mut c_void = std::ptr::null_mut();
    let err = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            CFRetained::as_ptr(&key).as_ptr().cast(),
            &mut value as *mut _ as *mut _,
        )
    };
    (err == kAXErrorSuccess && !value.is_null()).then_some(value)
}

fn attr_string(element: AXUIElementRef, attr: &str) -> Option<String> {
    let v = attr_ptr(element, attr)?;
    let cf = unsafe { CFRetained::from_raw(std::ptr::NonNull::new(v.cast::<CFType>())?) };
    // AXValue attributes are not always strings — a checkbox's value is a
    // number. Only take it when it really is text.
    let s = cf.downcast_ref::<CFString>()?;
    let text = s.to_string();
    (!text.trim().is_empty()).then_some(text)
}

fn attr_point(element: AXUIElementRef, attr: &str) -> Option<(f64, f64)> {
    let v = attr_ptr(element, attr)?;
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let ok = unsafe {
        AXValueGetValue(
            v as _,
            kAXValueTypeCGPoint,
            &mut point as *mut _ as *mut c_void,
        )
    };
    ok.then_some((point.x, point.y))
}

fn attr_size(element: AXUIElementRef, attr: &str) -> Option<(f64, f64)> {
    let v = attr_ptr(element, attr)?;
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok = unsafe {
        AXValueGetValue(
            v as _,
            kAXValueTypeCGSize,
            &mut size as *mut _ as *mut c_void,
        )
    };
    ok.then_some((size.width, size.height))
}

fn children(element: AXUIElementRef) -> Vec<AXUIElementRef> {
    let Some(v) = attr_ptr(element, kAXChildrenAttribute) else {
        return Vec::new();
    };
    let Some(ptr) = std::ptr::NonNull::new(v.cast::<CFArray>()) else {
        return Vec::new();
    };
    let array = unsafe { CFRetained::retain(ptr) };
    (0..array.count())
        .filter_map(|i| super::cf::array_ptr_at(&array, i).map(|p| p as AXUIElementRef))
        .collect()
}

fn walk(element: AXUIElementRef, depth: usize, out: &mut Vec<Element>) {
    if depth > MAX_DEPTH || out.len() >= MAX_ELEMENTS {
        return;
    }

    let role = attr_string(element, kAXRoleAttribute).unwrap_or_default();

    // A control is worth reporting when it says something. Prefer the title,
    // then the value (text fields), then the description (icon-only buttons
    // whose only label is their accessibility description).
    let text = attr_string(element, kAXTitleAttribute)
        .or_else(|| attr_string(element, kAXValueAttribute))
        .or_else(|| attr_string(element, kAXDescriptionAttribute));

    if let Some(text) = text {
        if let (Some((x, y)), Some((w, h))) = (
            attr_point(element, kAXPositionAttribute),
            attr_size(element, kAXSizeAttribute),
        ) {
            // Zero-sized and offscreen elements exist in the tree but can't be
            // clicked, so they're noise.
            if w >= 1.0 && h >= 1.0 {
                out.push(Element {
                    role: role.clone(),
                    text,
                    x,
                    y,
                    width: w,
                    height: h,
                    center_x: x + w / 2.0,
                    center_y: y + h / 2.0,
                });
            }
        }
    }

    for child in children(element) {
        walk(child, depth + 1, out);
    }
}

fn pid_for_app(app_name: Option<&str>) -> Result<i32, String> {
    let workspace = NSWorkspace::sharedWorkspace();

    match app_name {
        Some(name) => workspace.runningApplications()
            .iter()
            .find(|a| {
                a.localizedName()
                    .map(|n| n.to_string().eq_ignore_ascii_case(name))
                    .unwrap_or(false)
            })
            .map(|a| a.processIdentifier())
            .ok_or_else(|| format!("{name} is not running")),
        None => workspace.frontmostApplication()
            .map(|a| a.processIdentifier())
            .ok_or_else(|| "no frontmost application".to_string()),
    }
}

/// Every labelled, clickable element in `app` (or the frontmost app).
pub fn read_screen(app_name: Option<&str>) -> Result<Vec<Element>, String> {
    if !super::permissions::accessibility_granted() {
        return Err(
            "conduit does not have Accessibility permission yet. grant it in the Server tab."
                .into(),
        );
    }

    let pid = pid_for_app(app_name)?;
    let ax_app = unsafe { AXUIElementCreateApplication(pid) };
    if ax_app.is_null() {
        return Err("could not open an accessibility handle for that app".into());
    }

    let mut out = Vec::new();
    walk(ax_app, 0, &mut out);
    Ok(out)
}

/// Elements whose text contains `query`, best match first.
///
/// Ranking rather than raw filtering matters: searching "Save" in a save dialog
/// hits both "Save" and "Save As…", and the agent wants the exact one.
pub fn find_element(query: &str, app_name: Option<&str>) -> Result<Vec<Element>, String> {
    let needle = query.trim().to_lowercase();
    let mut matches: Vec<(u8, Element)> = read_screen(app_name)?
        .into_iter()
        .filter_map(|e| {
            let hay = e.text.to_lowercase();
            let rank = if hay == needle {
                0
            } else if hay.starts_with(&needle) {
                1
            } else if hay.contains(&needle) {
                2
            } else {
                return None;
            };
            Some((rank, e))
        })
        .collect();

    matches.sort_by_key(|(rank, _)| *rank);
    Ok(matches.into_iter().map(|(_, e)| e).take(20).collect())
}
