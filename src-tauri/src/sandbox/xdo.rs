//! xdotool command lines for the sandbox's input tools.
//!
//! Each input tool is a single `xdotool` invocation whose commands are chained
//! (`mousemove … sleep … mousemove … click …`), so one `docker exec` carries a
//! whole glide and the pointer never pauses between steps waiting for the next
//! round trip. The glide itself is `platform::tween::path` — the same curve the
//! host cursor follows, so sandbox recordings and host sessions feel alike.
//!
//! `mousemove --sync` is never used: it waits for a motion event that never
//! comes when the pointer is already at the target, and hangs.

use crate::platform::tween;
use crate::platform::types::Button;

/// Step interval between glide points, matching `tween::STEP`.
const STEP_SECONDS: &str = "0.008";

pub fn button_number(button: Button) -> u8 {
    match button {
        Button::Left => 1,
        Button::Middle => 2,
        Button::Right => 3,
    }
}

fn point(x: f64, y: f64) -> [String; 2] {
    [
        format!("{}", x.round() as i64),
        format!("{}", y.round() as i64),
    ]
}

/// `mousemove` steps from `from` to `to`, ending exactly on `to`.
pub fn glide_chain(from: (f64, f64), to: (f64, f64)) -> Vec<String> {
    let mut out = Vec::new();
    for (x, y) in tween::path(from.0, from.1, to.0, to.1) {
        out.push("mousemove".into());
        out.extend(point(x, y));
        out.push("sleep".into());
        out.push(STEP_SECONDS.into());
    }
    out.push("mousemove".into());
    out.extend(point(to.0, to.1));
    out
}

pub fn click_chain(button: Button, count: u8) -> Vec<String> {
    vec![
        "click".into(),
        "--repeat".into(),
        count.clamp(1, 3).to_string(),
        "--delay".into(),
        "90".into(),
        button_number(button).to_string(),
    ]
}

/// Press at `from`, glide to `to`, release. The short holds on either side
/// are what make toolkits recognise a drag rather than a click.
pub fn drag_chain(from: (f64, f64), to: (f64, f64), button: Button) -> Vec<String> {
    let b = button_number(button).to_string();
    let mut out = vec!["mousemove".into()];
    out.extend(point(from.0, from.1));
    out.extend(["mousedown".into(), b.clone(), "sleep".into(), "0.09".into()]);
    out.extend(glide_chain(from, to));
    out.extend(["sleep".into(), "0.06".into(), "mouseup".into(), b]);
    out
}

/// Whole wheel detents for a scroll in points, exactly as the Linux host
/// counts them: magnitude first, at least one, at most twenty.
pub fn detents(v: i32) -> i32 {
    if v == 0 {
        return 0;
    }
    (v.abs() / 50).clamp(1, 20) * v.signum()
}

/// Wheel clicks for a scroll. conduit's positive `dy` moves content up, which
/// X spells as button 4; positive `dx` scrolls right, button 7.
pub fn scroll_chain(dx: i32, dy: i32) -> Vec<String> {
    let mut out = Vec::new();
    let mut wheel = |n: i32, positive: u8, negative: u8| {
        if n == 0 {
            return;
        }
        let button = if n > 0 { positive } else { negative };
        out.extend([
            "click".into(),
            "--repeat".into(),
            n.abs().to_string(),
            "--delay".into(),
            "30".into(),
            button.to_string(),
        ]);
    };
    wheel(detents(dy), 4, 5);
    wheel(detents(dx), 7, 6);
    out
}

/// Parses `xdotool getmouselocation --shell`.
pub fn parse_location(stdout: &str) -> Option<(f64, f64)> {
    let mut x = None;
    let mut y = None;
    for line in stdout.lines() {
        if let Some(v) = line.strip_prefix("X=") {
            x = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("Y=") {
            y = v.trim().parse().ok();
        }
    }
    Some((x?, y?))
}

/// Parses `xdotool getdisplaygeometry`: `1280 800`.
pub fn parse_geometry(stdout: &str) -> Option<(u32, u32)> {
    let mut parts = stdout.split_whitespace();
    let w = parts.next()?.parse().ok()?;
    let h = parts.next()?.parse().ok()?;
    Some((w, h))
}

pub fn with_xdotool(chain: Vec<String>) -> Vec<String> {
    let mut argv = vec!["xdotool".to_string()];
    argv.extend(chain);
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glides_end_exactly_on_target_and_never_sync() {
        let chain = glide_chain((0.0, 0.0), (800.0, 600.0));
        let n = chain.len();
        assert_eq!(&chain[n - 3..], ["mousemove", "800", "600"]);
        assert!(!chain.iter().any(|a| a == "--sync"));
        // Intermediate steps exist, each followed by a short sleep.
        assert!(chain.iter().filter(|a| *a == "mousemove").count() > 10);
        assert!(chain.iter().any(|a| a == "0.008"));
        // A trip shorter than a pixel is a single move.
        assert_eq!(glide_chain((5.0, 5.0), (5.2, 5.1)), ["mousemove", "5", "5"]);
    }

    #[test]
    fn buttons_map_to_x_numbers() {
        assert_eq!(
            click_chain(Button::Left, 1),
            ["click", "--repeat", "1", "--delay", "90", "1"]
        );
        assert_eq!(click_chain(Button::Right, 2)[5], "3");
        assert_eq!(click_chain(Button::Middle, 1)[5], "2");
        assert_eq!(click_chain(Button::Left, 9)[2], "3", "clamped");
    }

    #[test]
    fn scroll_signs_and_detents_match_the_host() {
        assert_eq!(
            scroll_chain(0, 120),
            ["click", "--repeat", "2", "--delay", "30", "4"]
        );
        assert_eq!(scroll_chain(0, -120)[5], "5");
        assert_eq!(scroll_chain(60, 0)[5], "7");
        assert_eq!(scroll_chain(-60, 0)[5], "6");
        assert_eq!(scroll_chain(0, 10)[2], "1", "a tiny scroll still moves");
        assert_eq!(
            scroll_chain(0, 100_000)[2],
            "20",
            "one call cannot fling a page"
        );
        assert!(scroll_chain(0, 0).is_empty());
    }

    #[test]
    fn drags_press_glide_release() {
        let chain = drag_chain((10.0, 10.0), (200.0, 10.0), Button::Left);
        assert_eq!(&chain[..5], ["mousemove", "10", "10", "mousedown", "1"]);
        assert_eq!(&chain[chain.len() - 2..], ["mouseup", "1"]);
        let release = chain.iter().position(|a| a == "mouseup").unwrap();
        assert_eq!(&chain[release - 5..release - 2], ["mousemove", "200", "10"]);
    }

    #[test]
    fn xdotool_output_is_parsed() {
        assert_eq!(
            parse_location("X=640\nY=400\nSCREEN=0\nWINDOW=123\n"),
            Some((640.0, 400.0))
        );
        assert_eq!(parse_location("garbage"), None);
        assert_eq!(parse_geometry("1280 800\n"), Some((1280, 800)));
        assert_eq!(parse_geometry(""), None);
    }
}
