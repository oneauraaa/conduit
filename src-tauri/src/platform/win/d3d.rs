//! Direct3D 11 plumbing for Windows.Graphics.Capture.
//!
//! WGC hands back frames as GPU textures. Getting pixels out of one means
//! copying it into a CPU-readable *staging* texture and mapping that — a GPU
//! texture cannot be read directly by the CPU, which is the whole reason this
//! file exists.

use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::core::Interface;

pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    /// The same device, in the shape WinRT's capture API wants.
    pub winrt_device: IDirect3DDevice,
}

impl Gpu {
    pub fn new() -> Result<Self, String> {
        // BGRA_SUPPORT is required: WGC delivers frames as B8G8R8A8, and
        // without the flag device creation succeeds but the frame pool refuses
        // that format later, several layers away from the real cause.
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;

        // WARP is the software rasteriser. Falling back to it keeps capture
        // working in a VM or over RDP, where there may be no usable GPU at all.
        let mut last = String::new();
        for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
            let hr = unsafe {
                D3D11CreateDevice(
                    None,
                    driver,
                    windows::Win32::Foundation::HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
            };
            match hr {
                Ok(()) => break,
                Err(e) => last = e.to_string(),
            }
        }

        let (Some(device), Some(context)) = (device, context) else {
            return Err(format!("could not create a Direct3D device: {last}"));
        };

        let dxgi: IDXGIDevice = device
            .cast()
            .map_err(|e| format!("could not reach the DXGI device: {e}"))?;

        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .map_err(|e| format!("could not wrap the device for WinRT: {e}"))?;
        let winrt_device: IDirect3DDevice = inspectable
            .cast()
            .map_err(|e| format!("could not cast the WinRT device: {e}"))?;

        Ok(Gpu {
            device,
            context,
            winrt_device,
        })
    }

    /// Copies `region` out of a GPU texture and returns it as tightly-packed
    /// RGBA bytes.
    ///
    /// `region` is in texture pixels. Passing `None` copies the whole thing.
    pub fn read_back(
        &self,
        source: &ID3D11Texture2D,
        region: Option<(u32, u32, u32, u32)>,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { source.GetDesc(&mut desc) };

        let (rx, ry, rw, rh) = region.unwrap_or((0, 0, desc.Width, desc.Height));
        // Clamp rather than fail: a region a pixel past the edge is a rounding
        // artefact, not a reason to refuse the whole screenshot.
        let rx = rx.min(desc.Width.saturating_sub(1));
        let ry = ry.min(desc.Height.saturating_sub(1));
        let rw = rw.clamp(1, desc.Width - rx);
        let rh = rh.clamp(1, desc.Height - ry);

        // A staging texture is the only kind the CPU may map.
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: rw,
            Height: rh,
            MipLevels: 1,
            ArraySize: 1,
            Format: desc.Format,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };

        let mut staging: Option<ID3D11Texture2D> = None;
        unsafe { self.device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }
            .map_err(|e| format!("could not create a staging texture: {e}"))?;
        let staging = staging.ok_or("the staging texture was not created")?;

        let box_ = D3D11_BOX {
            left: rx,
            top: ry,
            front: 0,
            right: rx + rw,
            bottom: ry + rh,
            back: 1,
        };

        unsafe {
            self.context.CopySubresourceRegion(
                &staging,
                0,
                0,
                0,
                0,
                source,
                0,
                if region.is_some() { Some(&box_) } else { None },
            );
        }

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
            .map_err(|e| format!("could not map the captured pixels: {e}"))?;

        let mut out = vec![0u8; (rw as usize) * (rh as usize) * 4];
        unsafe {
            let src = mapped.pData as *const u8;
            // RowPitch is almost never width * 4 — the driver pads rows for
            // alignment. Copying the buffer as one block produces an image
            // sheared diagonally, which looks like an encoder bug and is not.
            let pitch = mapped.RowPitch as usize;
            let row_bytes = (rw as usize) * 4;

            for y in 0..rh as usize {
                let src_row = std::slice::from_raw_parts(src.add(y * pitch), row_bytes);
                let dst_row = &mut out[y * row_bytes..(y + 1) * row_bytes];
                dst_row.copy_from_slice(src_row);

                // WGC gives BGRA; the PNG encoder wants RGBA. Swapping in
                // place here avoids a second pass over the whole image.
                for px in dst_row.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
            }

            self.context.Unmap(&staging, 0);
        }

        Ok((out, rw, rh))
    }
}
