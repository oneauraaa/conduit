//! Reading the screen through UI Automation.
//!
//! This is the tool agents should reach for first. Pixels force a model to
//! guess at text and button edges; the UIA tree hands over the actual strings
//! and the actual bounds, so a click lands on the real centre of the real
//! control. It is also dramatically cheaper than a screenshot.
//!
//! ## Three decisions that are all about speed
//!
//! UIA is a cross-process COM protocol: every uncached property read is a
//! round trip to the target application, and a browser has tens of thousands of
//! elements. Read naively, `read_screen_text` on Chrome takes many seconds.
//!
//!   1. **Walk level by level, never `TreeScope_Subtree`.** A subtree
//!      `FindAll` materialises the entire tree *before* returning, so the
//!      element cap below can't help — the cost is already paid. Walking
//!      children lets the cap actually stop the work.
//!   2. **Cached properties.** One batched request per level instead of four
//!      cross-process round trips per element.
//!   3. **A transaction timeout**, because UIA ships without one: a hung
//!      application would otherwise block the call forever.
//!
//! ## Why not `AutomationElementMode_None`
//!
//! It is the obvious fourth optimisation and it is a trap. Elements returned
//! under that mode carry cached data but *no live reference*, so calling
//! `FindAllBuildCache` on one fails — and this walk would stop dead one level
//! below the window, returning five elements for a screen with two hundred.
//! It fails silently, which is the worst way for a tool like this to fail: the
//! agent gets a short list that looks like a complete one and concludes the
//! button it wants isn't on screen.
//!
//! The element cap is what actually bounds the cost here, and it only works
//! because the walk can navigate.

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2,
    IUIAutomationCacheRequest, IUIAutomationCondition, IUIAutomationElement,
    IUIAutomationValuePattern, TreeScope_Children, UIA_BoundingRectanglePropertyId,
    UIA_CONTROLTYPE_ID, UIA_ControlTypePropertyId, UIA_IsOffscreenPropertyId, UIA_NamePropertyId,
    UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::core::Interface;

use super::com::Com;
use crate::platform::types::{Element, rank_matches};

/// Depth cap. Real UIs nest deeply (a browser page is easily 20 levels); beyond
/// this the payload grows faster than its usefulness.
const MAX_DEPTH: usize = 18;
/// Hard cap on returned elements, so one pathological app can't blow the
/// model's context window.
const MAX_ELEMENTS: usize = 400;
/// How long any single UIA call may block. UIA ships with no default timeout,
/// so without this a hung application hangs the tool call indefinitely.
const TRANSACTION_TIMEOUT_MS: u32 = 2000;

/// Every labelled, clickable element in `app_name` (or the frontmost app).
pub fn read_screen(app_name: Option<&str>) -> Result<Vec<Element>, String> {
    let _com = Com::init();

    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| format!("could not start UI Automation: {e}"))?;

    // IUIAutomation2 is Windows 8+; every supported target has it, but degrade
    // rather than fail if the cast ever doesn't take.
    if let Ok(a2) = automation.cast::<IUIAutomation2>() {
        let _ = unsafe { a2.SetTransactionTimeout(TRANSACTION_TIMEOUT_MS) };
    }

    let hwnd = target_window(app_name)?;
    let root = unsafe { automation.ElementFromHandle(hwnd) }
        .map_err(|e| format!("could not read that window's accessibility tree: {e}"))?;

    let condition = unsafe { automation.CreateTrueCondition() }
        .map_err(|e| format!("could not build a UI Automation query: {e}"))?;
    let cache = build_cache(&automation)?;

    let mut out = Vec::new();
    walk(&root, &condition, &cache, 0, &mut out);
    Ok(out)
}

/// Elements whose text contains `query`, best match first.
pub fn find_element(query: &str, app_name: Option<&str>) -> Result<Vec<Element>, String> {
    Ok(rank_matches(query, read_screen(app_name)?))
}

/// The window to read: the named app's frontmost window, or whatever is in
/// front right now.
fn target_window(app_name: Option<&str>) -> Result<HWND, String> {
    let Some(name) = app_name else {
        let hwnd = unsafe { GetForegroundWindow() };
        return if hwnd.is_invalid() {
            Err("no window is in the foreground".into())
        } else {
            Ok(hwnd)
        };
    };

    // `list_windows` is already front-to-back, so the first match is the app's
    // frontmost window — the one the user is looking at.
    super::apps::list_windows()
        .into_iter()
        .find(|w| w.app.eq_ignore_ascii_case(name) || w.app.to_lowercase().contains(&name.to_lowercase()))
        .map(|w| HWND(w.id as usize as *mut _))
        .ok_or_else(|| format!("{name} is not running, or has no window"))
}

/// One batched request for every property this module reads.
fn build_cache(automation: &IUIAutomation) -> Result<IUIAutomationCacheRequest, String> {
    let cache = unsafe { automation.CreateCacheRequest() }
        .map_err(|e| format!("could not build a UI Automation cache request: {e}"))?;

    unsafe {
        for property in [
            UIA_NamePropertyId,
            UIA_ControlTypePropertyId,
            UIA_BoundingRectanglePropertyId,
            UIA_IsOffscreenPropertyId,
        ] {
            let _ = cache.AddProperty(property);
        }
        // A text field's *contents* come from the Value pattern rather than a
        // property. Read through the pattern rather than
        // `GetCachedPropertyValue`, which hands back a `VARIANT` — the one
        // Win32 type that has moved modules between `windows` releases, and so
        // the import most likely to break on a version bump.
        let _ = cache.AddPattern(UIA_ValuePatternId);

        // `AutomationElementMode` is deliberately left at its default of
        // `Full`. Setting it to `None` is the tempting optimisation and it
        // breaks the walk — see the module note.
    }

    Ok(cache)
}

/// Depth-first walk, one level of children at a time.
fn walk(
    element: &IUIAutomationElement,
    condition: &IUIAutomationCondition,
    cache: &IUIAutomationCacheRequest,
    depth: usize,
    out: &mut Vec<Element>,
) {
    if depth >= MAX_DEPTH || out.len() >= MAX_ELEMENTS {
        return;
    }

    let Ok(children) = (unsafe { element.FindAllBuildCache(TreeScope_Children, condition, cache) })
    else {
        return;
    };
    let Ok(count) = (unsafe { children.Length() }) else {
        return;
    };

    for i in 0..count {
        if out.len() >= MAX_ELEMENTS {
            return;
        }
        let Ok(child) = (unsafe { children.GetElement(i) }) else {
            continue;
        };

        if let Some(el) = describe(&child) {
            out.push(el);
        }
        walk(&child, condition, cache, depth + 1, out);
    }
}

/// Turns a UIA element into conduit's shape, or `None` if it isn't worth
/// reporting.
fn describe(element: &IUIAutomationElement) -> Option<Element> {
    unsafe {
        // Scrolled out of view, or on a collapsed tab. Its bounds are real but
        // clicking them would hit whatever is actually there instead.
        if element.CachedIsOffscreen().map(|b| b.as_bool()).unwrap_or(false) {
            return None;
        }

        let rect: RECT = element.CachedBoundingRectangle().ok()?;
        let (w, h) = ((rect.right - rect.left) as f64, (rect.bottom - rect.top) as f64);
        // Zero-sized elements are layout containers, not click targets.
        if w < 1.0 || h < 1.0 {
            return None;
        }

        // Name is the label; Value is what a text field actually contains.
        // Preferring Name matches the macOS side's title-then-value order.
        let name = element.CachedName().map(|b| b.to_string()).unwrap_or_default();
        let text = if name.trim().is_empty() {
            element
                .GetCachedPattern(UIA_ValuePatternId)
                .ok()
                .and_then(|p| p.cast::<IUIAutomationValuePattern>().ok())
                .and_then(|v| v.CachedValue().ok())
                .map(|b| b.to_string())
                .unwrap_or_default()
        } else {
            name
        };

        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }

        let control_type = element.CachedControlType().unwrap_or(UIA_CONTROLTYPE_ID(0));

        Some(Element {
            role: role_name(control_type).to_string(),
            text,
            x: rect.left as f64,
            y: rect.top as f64,
            width: w,
            height: h,
            center_x: rect.left as f64 + w / 2.0,
            center_y: rect.top as f64 + h / 2.0,
        })
    }
}

/// Maps a UIA control type onto the role vocabulary the macOS backend emits.
///
/// The names are deliberately the AX ones with the `AX` prefix dropped, so a
/// prompt or a heuristic written against one platform reads the same on the
/// other. An agent looking for `"button"` should not have to know it is talking
/// to UIA rather than the accessibility API.
fn role_name(control_type: UIA_CONTROLTYPE_ID) -> &'static str {
    use windows::Win32::UI::Accessibility::*;

    match control_type {
        UIA_ButtonControlTypeId => "button",
        UIA_CheckBoxControlTypeId => "checkbox",
        UIA_ComboBoxControlTypeId => "combobox",
        UIA_EditControlTypeId => "textfield",
        UIA_HyperlinkControlTypeId => "link",
        UIA_ImageControlTypeId => "image",
        UIA_ListItemControlTypeId => "listitem",
        UIA_ListControlTypeId => "list",
        UIA_MenuControlTypeId | UIA_MenuBarControlTypeId => "menu",
        UIA_MenuItemControlTypeId => "menuitem",
        UIA_ProgressBarControlTypeId => "progressindicator",
        UIA_RadioButtonControlTypeId => "radiobutton",
        UIA_ScrollBarControlTypeId => "scrollbar",
        UIA_SliderControlTypeId => "slider",
        UIA_SpinnerControlTypeId => "stepper",
        UIA_StatusBarControlTypeId => "statusbar",
        UIA_TabControlTypeId => "tabgroup",
        UIA_TabItemControlTypeId => "tab",
        UIA_TextControlTypeId => "text",
        UIA_ToolBarControlTypeId => "toolbar",
        UIA_ToolTipControlTypeId => "tooltip",
        UIA_TreeControlTypeId => "outline",
        UIA_TreeItemControlTypeId => "row",
        UIA_WindowControlTypeId => "window",
        UIA_GroupControlTypeId => "group",
        UIA_DocumentControlTypeId => "document",
        UIA_TableControlTypeId | UIA_DataGridControlTypeId => "table",
        UIA_DataItemControlTypeId => "row",
        UIA_HeaderControlTypeId => "header",
        UIA_SplitButtonControlTypeId => "button",
        UIA_CalendarControlTypeId => "calendar",
        UIA_PaneControlTypeId => "group",
        _ => "element",
    }
}
