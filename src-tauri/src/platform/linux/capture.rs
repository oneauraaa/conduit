//! Screen capture, off the PipeWire streams the portal session already owns.
//!
//! ## Why a live stream and not a screenshot per call
//!
//! `org.freedesktop.portal.Screenshot` exists and is three lines to call. It is
//! the wrong tool here: it raises its own consent dialog (a second one), it
//! renders to a PNG on disk which conduit then has to read back, and on KDE it
//! is not guaranteed to stay non-interactive. An agent takes a screenshot
//! before nearly every action, so per-call file I/O is paid hundreds of times a
//! session.
//!
//! The portal session conduit *already needs* for absolute pointer coordinates
//! (see [`super::portal`]) comes with PipeWire streams attached. Consuming them
//! makes `capture` a memcpy out of the newest frame — no dialog, no disk, no
//! per-call negotiation.
//!
//! ## The cost, stated plainly
//!
//! A live stream means the compositor is compositing frames conduit mostly
//! throws away. That is why the framerate is negotiated low: a screen-reading
//! agent does not need 60fps, and [`TARGET_FPS`] is the knob that keeps this
//! from being a background GPU tax.
//!
//! ## Traps
//!
//! - **`stride` is almost never `width * 4`.** It is padded to the GPU's
//!   alignment — 2560px wide came back with a 10240-byte stride here, which
//!   happens to match, and on other widths does not. Copy row by row or the
//!   image comes out sheared. Exactly the same trap as `RowPitch` on Windows.
//! - **Ask for packed formats only.** If the client advertises DMA-BUF support
//!   the compositor will happily hand over GPU buffers, which `MAP_BUFFERS`
//!   cannot map and which need EGL to read. Advertising only BGRx/RGBx/BGRA/
//!   RGBA is what keeps the server on memfd.
//! - **The PipeWire main loop owns its thread.** It is `!Send`, so it gets a
//!   dedicated thread and talks to the rest of conduit through [`FRAMES`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use image::{ImageEncoder, codecs::png::PngEncoder};
use parking_lot::RwLock;
use pipewire as pw;
use pw::spa;
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::video::{VideoFormat, VideoInfoRaw};
use spa::pod::Pod;
use spa::utils::Direction;

use crate::platform::types::{Display, Shot};

/// Frames per second to negotiate.
///
/// Deliberately low. Every frame the compositor produces for conduit is one it
/// composites on top of its real work, and an agent reading the screen wants a
/// *recent* frame, not a fresh one. 10fps keeps the newest frame under 100ms
/// old — well inside the time an agent spends deciding what to do with it.
const TARGET_FPS: u32 = 10;

/// How long `capture` waits for the first frame of a stream that has only just
/// started. After this it reports honestly rather than returning a black image.
const FIRST_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The newest frame from each stream, keyed by PipeWire node id.
static FRAMES: RwLock<Option<HashMap<u32, Frame>>> = RwLock::new(None);
/// Guards against starting the capture thread twice.
static STARTED: AtomicBool = AtomicBool::new(false);

/// One decoded frame, already converted to RGBA and tightly packed.
#[derive(Clone)]
struct Frame {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/* ── the public surface ───────────────────────────────────────── */

/// Starts the capture thread. Idempotent; called once consent is in.
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::Builder::new()
        .name("conduit-capture".into())
        .spawn(|| {
            if let Err(e) = run() {
                tracing::warn!("screen capture is unavailable: {e}");
                // Let a later attempt retry rather than wedging capture for the
                // life of the process.
                STARTED.store(false, Ordering::SeqCst);
            }
        })
        .ok();
}

/// Captures `display`, optionally cropping to `region` and scaling the result.
///
/// `region` is relative to the display's own origin, in the same space the rest
/// of conduit uses. `scale` downsamples the final image — screens are large and
/// models are billed per pixel.
pub fn capture(
    display: &Display,
    region: Option<(f64, f64, f64, f64)>,
    scale: f64,
) -> Result<Shot, String> {
    let session = super::portal::session().map_err(|e| e.message())?;

    // Displays and streams are both ordered top-to-bottom, left-to-right, so
    // index lines up. Falling back to the stream the point *contains* covers a
    // monitor hot-plugged since the session started.
    let stream = session
        .streams()
        .get(display.index)
        .or_else(|| {
            session
                .streams()
                .iter()
                .find(|s| s.contains(display.x, display.y))
        })
        .or_else(|| session.streams().first())
        .ok_or("the portal session is sharing no screens")?;

    start();
    let frame = await_frame(stream.node_id)?;

    let (rx, ry, rw, rh) = region.unwrap_or((0.0, 0.0, display.width, display.height));

    // The frame is in stream pixels; the region is in conduit's space, which
    // for the stream's own display is the same origin but may differ in scale
    // if the compositor is downscaling the capture. Map through the ratio
    // rather than assuming 1:1, or a fractional-scaling desktop crops the
    // wrong rectangle.
    let fx = frame.width as f64 / display.width.max(1.0);
    let fy = frame.height as f64 / display.height.max(1.0);

    let px = ((rx * fx).round() as i64).clamp(0, frame.width as i64) as u32;
    let py = ((ry * fy).round() as i64).clamp(0, frame.height as i64) as u32;
    let pw_ = ((rw * fx).round() as i64).clamp(1, (frame.width - px).max(1) as i64) as u32;
    let ph = ((rh * fy).round() as i64).clamp(1, (frame.height - py).max(1) as i64) as u32;

    let cropped = crop(&frame, px, py, pw_, ph);

    let out_w = (((pw_ as f64) * scale).round() as u32).max(1);
    let out_h = (((ph as f64) * scale).round() as u32).max(1);

    let final_rgba = if out_w == pw_ && out_h == ph {
        cropped
    } else {
        resize(&cropped, pw_, ph, out_w, out_h)
    };

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &final_rgba,
            out_w,
            out_h,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("could not encode png: {e}"))?;

    Ok(Shot {
        png,
        width: out_w,
        height: out_h,
    })
}

/// Whether at least one frame has arrived — the readiness card's "screenshots
/// work" signal, without taking one.
pub fn has_frames() -> bool {
    FRAMES
        .read()
        .as_ref()
        .map(|f| !f.is_empty())
        .unwrap_or(false)
}

fn await_frame(node_id: u32) -> Result<Frame, String> {
    let deadline = std::time::Instant::now() + FIRST_FRAME_TIMEOUT;
    loop {
        if let Some(frame) = FRAMES
            .read()
            .as_ref()
            .and_then(|frames| frames.get(&node_id))
            .cloned()
        {
            return Ok(frame);
        }
        if std::time::Instant::now() >= deadline {
            return Err(
                "no frames have arrived from the screen-sharing stream yet. if the \
                 sharing dialog is still open, answer it and try again."
                    .into(),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn crop(frame: &Frame, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    if x == 0 && y == 0 && w == frame.width && h == frame.height {
        return frame.rgba.clone();
    }
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    let src_stride = frame.width as usize * 4;
    let dst_stride = w as usize * 4;
    for row in 0..h as usize {
        let src = (y as usize + row) * src_stride + x as usize * 4;
        let dst = row * dst_stride;
        out[dst..dst + dst_stride].copy_from_slice(&frame.rgba[src..src + dst_stride]);
    }
    out
}

/// Nearest-neighbour downscale.
///
/// Box filtering would look better, but this output is read by a model, not a
/// person: text legibility is what matters and nearest neighbour keeps glyph
/// edges hard rather than smearing them into grey. It is also several times
/// cheaper on a 4K frame.
fn resize(rgba: &[u8], w: u32, h: u32, out_w: u32, out_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (out_w as usize) * (out_h as usize) * 4];
    for y in 0..out_h as usize {
        let sy = (y as u64 * h as u64 / out_h as u64).min(h as u64 - 1) as usize;
        for x in 0..out_w as usize {
            let sx = (x as u64 * w as u64 / out_w as u64).min(w as u64 - 1) as usize;
            let src = (sy * w as usize + sx) * 4;
            let dst = (y * out_w as usize + x) * 4;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    out
}

/* ── the PipeWire thread ──────────────────────────────────────── */

/// Per-stream state the callbacks share. One of these per node.
struct StreamState {
    node_id: u32,
    info: Option<VideoInfoRaw>,
}

fn run() -> Result<(), String> {
    let session = super::portal::session().map_err(|e| e.message())?;
    let fd = session.pipewire_fd()?;
    let nodes: Vec<u32> = session.streams().iter().map(|s| s.node_id).collect();

    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|e| format!("could not create a pipewire loop: {e}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|e| format!("could not create a pipewire context: {e}"))?;
    let core = context
        .connect_fd_rc(unsafe { std::os::fd::FromRawFd::from_raw_fd(fd) }, None)
        .map_err(|e| format!("could not connect to pipewire: {e}"))?;

    *FRAMES.write() = Some(HashMap::new());

    // Streams and their listeners must outlive the loop, so they are kept in
    // scope here rather than dropped at the end of a helper.
    let mut kept = Vec::new();

    for node_id in nodes {
        let stream = pw::stream::StreamBox::new(
            &core,
            "conduit-capture",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )
        .map_err(|e| format!("could not create a capture stream: {e}"))?;

        let listener = stream
            .add_local_listener_with_user_data(StreamState {
                node_id,
                info: None,
            })
            .state_changed(|_, state, old, new| {
                tracing::debug!(node = state.node_id, ?old, ?new, "capture stream state");
            })
            .param_changed(|_, state, id, param| {
                let Some(param) = param else { return };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param)
                else {
                    return;
                };
                if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                    return;
                }
                let mut info = VideoInfoRaw::default();
                if info.parse(param).is_err() {
                    return;
                }
                tracing::info!(
                    node = state.node_id,
                    width = info.size().width,
                    height = info.size().height,
                    format = ?info.format(),
                    "capture format negotiated"
                );
                state.info = Some(info);
            })
            .process(|stream, state| {
                let Some(info) = state.info else { return };
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }

                let stride = datas[0].chunk().stride() as usize;
                let Some(bytes) = datas[0].data() else {
                    // No mapped pointer means the server handed over a DMA-BUF
                    // despite the format ask. Nothing to do but skip; the
                    // readiness card reports "no frames" rather than lying.
                    return;
                };

                let (w, h) = (info.size().width, info.size().height);
                let Some(rgba) = to_rgba(bytes, stride, w, h, info.format()) else {
                    return;
                };

                if let Some(frames) = FRAMES.write().as_mut() {
                    frames.insert(
                        state.node_id,
                        Frame {
                            rgba,
                            width: w,
                            height: h,
                        },
                    );
                }
            })
            .register()
            .map_err(|e| format!("could not register a capture listener: {e}"))?;

        let values = format_pod();
        let pod = Pod::from_bytes(&values).ok_or("could not build a capture format")?;
        let mut params = [pod];

        stream
            .connect(
                Direction::Input,
                Some(node_id),
                pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
                &mut params,
            )
            .map_err(|e| format!("could not connect a capture stream: {e}"))?;

        kept.push((stream, listener));
    }

    tracing::info!(streams = kept.len(), "screen capture running");
    mainloop.run();
    Ok(())
}

/// The format conduit will accept.
///
/// Packed 32-bit RGB only, and no DMA-BUF modifiers — see the module header.
/// Framerate is a range with [`TARGET_FPS`] as the default so the compositor
/// can settle lower if it wants, but never runs flat out for conduit's benefit.
fn format_pod() -> Vec<u8> {
    let obj = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::RGBx,
            VideoFormat::BGRA,
            VideoFormat::RGBA
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            spa::utils::Rectangle {
                width: 16384,
                height: 16384
            }
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction {
                num: TARGET_FPS,
                denom: 1
            },
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction {
                num: TARGET_FPS,
                denom: 1
            }
        ),
    );

    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .unwrap_or_default()
}

/// Repacks a captured frame into tightly-packed RGBA.
///
/// Two things happen here at once, and both are mandatory: the row stride is
/// removed, and the channel order is corrected. `BGRx` is what compositors
/// overwhelmingly hand back; treating it as RGBA is the classic "why is
/// everything blue" bug.
fn to_rgba(bytes: &[u8], stride: usize, w: u32, h: u32, format: VideoFormat) -> Option<Vec<u8>> {
    let (w, h) = (w as usize, h as usize);
    if stride < w * 4 || bytes.len() < stride * h {
        tracing::warn!(stride, w, h, len = bytes.len(), "short capture buffer");
        return None;
    }

    // Which source byte feeds each of R, G, B.
    let (r, g, b) = match format {
        VideoFormat::RGBx | VideoFormat::RGBA => (0, 1, 2),
        // BGRx/BGRA and anything unexpected: BGR order is the overwhelming
        // default, and guessing it is better than returning nothing.
        _ => (2, 1, 0),
    };

    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let row = &bytes[y * stride..y * stride + w * 4];
        let dst = y * w * 4;
        for x in 0..w {
            let p = &row[x * 4..x * 4 + 4];
            let o = dst + x * 4;
            out[o] = p[r];
            out[o + 1] = p[g];
            out[o + 2] = p[b];
            // The `x` in BGRx is padding, not alpha, and comes through as
            // zero — using it would make every screenshot fully transparent.
            out[o + 3] = 255;
        }
    }
    Some(out)
}
