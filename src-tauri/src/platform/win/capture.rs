//! Screen capture via Windows.Graphics.Capture.
//!
//! The legacy GDI path (`BitBlt` from the desktop DC) is not an option, for the
//! same reason `CGDisplayCreateImage` is rejected on macOS: it returns black
//! for hardware-accelerated and DirectComposition content — a playing video, a
//! Chrome tab, a game — and it does so *silently*. A screenshot that is
//! plausibly wrong is worse for an agent than an error, because it will reason
//! confidently about a black rectangle.
//!
//! Capture is synchronous and blocking, so callers run it on a blocking thread.

use std::sync::mpsc;
use std::time::Duration;

use image::{ImageEncoder, codecs::png::PngEncoder};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::Interface;

use super::com::Com;
use super::d3d::Gpu;
use crate::platform::types::{Display, Shot};

/// How long to wait for the compositor to deliver a frame.
///
/// Generous: the first frame after a session starts can take a few hundred ms
/// while DWM wires up the capture, and a spuriously failed screenshot costs the
/// agent a whole retry cycle.
const FRAME_TIMEOUT: Duration = Duration::from_secs(3);

/// Captures `display`, optionally cropping to `region` and scaling the result.
///
/// `region` is in conduit's coordinate space — physical pixels here — and is
/// relative to the display's own origin.
pub fn capture(
    display: &Display,
    region: Option<(f64, f64, f64, f64)>,
    scale: f64,
) -> Result<Shot, String> {
    // tokio hands out blocking threads from a pool, so this thread may or may
    // not already have a COM apartment. `Com` treats "already initialised" as
    // success — see its docs for why that matters here specifically.
    let _com = Com::init();

    let monitor = super::screen::monitor_handle(display.index)?;

    // GraphicsCaptureItem has no public constructor; the interop interface on
    // the activation factory is the documented way to build one for a monitor.
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| format!("could not reach the capture interop factory: {e}"))?;

    let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(monitor) }
        .map_err(|e| format!("could not open that display for capture: {e}"))?;

    let size: SizeInt32 = item
        .Size()
        .map_err(|e| format!("could not read the capture size: {e}"))?;

    let gpu = Gpu::new()?;

    // CreateFreeThreaded, not Create. The plain constructor requires the
    // calling thread to have a DispatcherQueue, which a tokio blocking thread
    // does not — and the resulting error reads like a COM apartment problem
    // rather than a missing queue, which is a long afternoon.
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &gpu.winrt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        1,
        size,
    )
    .map_err(|e| format!("could not create the capture frame pool: {e}"))?;

    let session = pool
        .CreateCaptureSession(&item)
        .map_err(|e| format!("could not start a capture session: {e}"))?;

    // The agent's cursor is drawn by the overlay, so a captured system arrow
    // would be a stale duplicate. Matches the macOS side.
    let _ = session.SetIsCursorCaptureEnabled(false);
    // Windows 11 22000+ only; on older builds the yellow capture border stays
    // and there is nothing to be done about it. The readiness card says so.
    let _ = session.SetIsBorderRequired(false);

    let (tx, rx) = mpsc::channel();
    pool.FrameArrived(&TypedEventHandler::new(
        move |pool: windows::core::Ref<Direct3D11CaptureFramePool>, _| {
            if let Some(pool) = pool.as_ref() {
                if let Ok(frame) = pool.TryGetNextFrame() {
                    // A send failure just means the receiver already got its
                    // frame and went away; nothing to do about it.
                    let _ = tx.send(frame);
                }
            }
            Ok(())
        },
    ))
    .map_err(|e| format!("could not subscribe to capture frames: {e}"))?;

    session
        .StartCapture()
        .map_err(|e| format!("could not start capturing: {e}"))?;

    let frame = rx
        .recv_timeout(FRAME_TIMEOUT)
        .map_err(|_| "the display did not produce a frame in time".to_string())?;

    // The frame's texture is only valid until the frame is closed, and the
    // session must stop or the compositor keeps feeding a pool nobody reads.
    let result = (|| {
        let surface = frame
            .Surface()
            .map_err(|e| format!("the captured frame had no surface: {e}"))?;
        let access: IDirect3DDxgiInterfaceAccess = surface
            .cast()
            .map_err(|e| format!("could not reach the frame's texture: {e}"))?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface() }
            .map_err(|e| format!("could not read the frame's texture: {e}"))?;

        // The pool's textures are sized to the *pool*, which is often larger
        // than the content — the surplus is uninitialised, and cropping to the
        // texture instead of the content yields black bars down two edges.
        let content = frame.ContentSize().unwrap_or(size);
        let (cw, ch) = (content.Width.max(0) as u32, content.Height.max(0) as u32);

        let (rx_, ry_, rw, rh) = match region {
            Some((x, y, w, h)) => {
                let x = x.max(0.0).round() as u32;
                let y = y.max(0.0).round() as u32;
                let w = w.max(1.0).round() as u32;
                let h = h.max(1.0).round() as u32;
                (x.min(cw), y.min(ch), w, h)
            }
            None => (0, 0, cw, ch),
        };

        gpu.read_back(&texture, Some((rx_, ry_, rw, rh)))
    })();

    let _ = session.Close();
    let _ = pool.Close();
    let _ = frame.Close();

    let (rgba, w, h) = result?;
    encode(rgba, w, h, scale)
}

/// Scales if asked, then encodes to PNG.
fn encode(rgba: Vec<u8>, w: u32, h: u32, scale: f64) -> Result<Shot, String> {
    let (out_w, out_h, pixels) = if (scale - 1.0).abs() < f64::EPSILON {
        (w, h, rgba)
    } else {
        let tw = ((w as f64 * scale).round() as u32).max(1);
        let th = ((h as f64 * scale).round() as u32).max(1);

        let src = image::RgbaImage::from_raw(w, h, rgba)
            .ok_or("the captured buffer did not match its dimensions")?;
        // Triangle: good enough for screenshots and markedly cheaper than
        // Lanczos on a 4K frame, which is a per-call latency the agent feels.
        let resized =
            image::imageops::resize(&src, tw, th, image::imageops::FilterType::Triangle);
        (tw, th, resized.into_raw())
    };

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&pixels, out_w, out_h, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("could not encode png: {e}"))?;

    Ok(Shot {
        png,
        width: out_w,
        height: out_h,
    })
}
