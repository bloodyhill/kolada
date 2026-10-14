//! Desktop Duplication capture.
//!
//! Records a luminance thumbnail of every desktop frame, stamped with the QPC
//! time DWM presented it. Thumbnails are the desktop box-filtered by 2^MIP on
//! the GPU (GenerateMips), so thin text strokes still register as changes.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_RESOURCE_MISC_GENERATE_MIPS, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_UNSUPPORTED, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
    IDXGIOutputDuplication, IDXGIResource,
};
use windows::core::Interface;

use crate::analyze::{Region, changed_in};
use crate::clock;

/// Thumbnails are the desktop downsampled by 2^MIP in each dimension.
pub const MIP: u32 = 3;

pub struct Frame {
    pub qpc: i64,
    pub lum: Vec<u8>,
}

pub struct Output {
    pub adapter: String,
    pub device: String,
    pub desktop: RECT,
}

pub struct Settings {
    pub monitor: usize,
    /// Screen area that decides when a run has settled (the work area, so
    /// taskbar animations and the clock don't count).
    pub watch: RECT,
    pub pixel_delta: u8,
    pub min_pixels: usize,
    pub settle_ms: f64,
    pub timeout_ms: f64,
}

/// What the capture thread reports once it holds the baseline frame.
struct Ready {
    width: u32,
    height: u32,
    desktop: RECT,
}

pub struct Capture {
    launch_qpc: Arc<AtomicI64>,
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Result<Vec<Frame>, String>>,
    pub width: u32,
    pub height: u32,
    pub desktop: RECT,
}

impl Capture {
    /// Starts capturing and returns once a real pre-launch baseline frame
    /// exists. `nudge` is called while waiting; it must cause a repaint,
    /// because a static desktop produces no frames.
    pub fn start(settings: Settings, nudge: impl Fn()) -> Result<Capture, String> {
        let (tx, rx) = mpsc::channel();
        let launch_qpc = Arc::new(AtomicI64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (launch_qpc, stop) = (launch_qpc.clone(), stop.clone());
            std::thread::spawn(move || run(settings, &launch_qpc, &stop, &tx))
        };
        loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(ready) => {
                    return Ok(Capture {
                        launch_qpc,
                        stop,
                        handle,
                        width: ready.width,
                        height: ready.height,
                        desktop: ready.desktop,
                    });
                }
                Err(RecvTimeoutError::Timeout) => nudge(),
                // The thread exited before the baseline: surface its error.
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(handle
                        .join()
                        .map_err(|_| "capture thread panicked".to_string())?
                        .err()
                        .unwrap_or_else(|| "capture stopped early".into()));
                }
            }
        }
    }

    /// Marks the launch time. The capture stops by itself once the screen
    /// has been quiet for `settle_ms` after a change, or after `timeout_ms`.
    pub fn arm(&self, launch_qpc: i64) {
        self.launch_qpc.store(launch_qpc, Ordering::SeqCst);
    }

    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    pub fn finish(self) -> Result<Vec<Frame>, String> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle
            .join()
            .map_err(|_| "capture thread panicked".to_string())?
    }
}

/// Monitors attached to the desktop, in DXGI enumeration order.
pub fn outputs() -> Result<Vec<Output>, String> {
    Ok(enumerate()?.into_iter().map(|(_, _, o)| o).collect())
}

fn enumerate() -> Result<Vec<(IDXGIAdapter1, IDXGIOutput, Output)>, String> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
    let mut list = Vec::new();
    let mut a = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(a) } {
        a += 1;
        let Ok(adapter_desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let mut o = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(o) } {
            o += 1;
            let Ok(desc) = (unsafe { output.GetDesc() }) else {
                continue;
            };
            if desc.AttachedToDesktop.as_bool() {
                let info = Output {
                    adapter: wide_to_string(&adapter_desc.Description),
                    device: wide_to_string(&desc.DeviceName),
                    desktop: desc.DesktopCoordinates,
                };
                list.push((adapter.clone(), output, info));
            }
        }
    }
    Ok(list)
}

pub fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

fn run(
    settings: Settings,
    launch_qpc: &AtomicI64,
    stop: &AtomicBool,
    ready: &mpsc::Sender<Ready>,
) -> Result<Vec<Frame>, String> {
    let mut dup = Duplicator::new(settings.monitor)?;

    // Only a presented frame is a trustworthy baseline: the image handed
    // back before anything repaints can be black.
    let baseline = loop {
        if let Some(frame) = dup.next(100)? {
            break frame;
        }
    };
    let mut frames = vec![baseline];
    let _ = ready.send(Ready {
        width: dup.width,
        height: dup.height,
        desktop: dup.desktop,
    });

    let settle = clock::ticks(settings.settle_ms);
    let timeout = clock::ticks(settings.timeout_ms);
    let watch = Region::from_window(settings.watch, dup.desktop, dup.width, dup.height)
        .unwrap_or(Region::full(dup.width, dup.height));
    let stride = dup.width as usize;
    let mut last_change: Option<i64> = None;
    while !stop.load(Ordering::SeqCst) {
        let t0 = launch_qpc.load(Ordering::SeqCst);
        if t0 != 0 {
            let now = clock::now();
            if now - t0 > timeout || last_change.is_some_and(|t| now - t > settle) {
                break;
            }
        }
        let Some(frame) = dup.next(16)? else {
            continue;
        };
        let prev = &frames[frames.len() - 1].lum;
        if *prev == frame.lum {
            continue;
        }
        if t0 != 0
            && frame.qpc > t0
            && changed_in(prev, &frame.lum, stride, &watch, settings.pixel_delta)
                >= settings.min_pixels
        {
            last_change = Some(frame.qpc);
        }
        frames.push(frame);
    }
    Ok(frames)
}

struct Duplicator {
    dup: IDXGIOutputDuplication,
    context: ID3D11DeviceContext,
    mips: ID3D11Texture2D,
    mips_view: ID3D11ShaderResourceView,
    staging: ID3D11Texture2D,
    width: u32,
    height: u32,
    desktop: RECT,
    _device: ID3D11Device,
}

impl Duplicator {
    fn new(monitor: usize) -> Result<Duplicator, String> {
        let mut all = enumerate()?;
        let count = all.len();
        if monitor >= count {
            return Err(format!("monitor {monitor} not found ({count} attached)"));
        }
        let (adapter, output, info) = all.swap_remove(monitor);

        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|e| format!("D3D11CreateDevice on {}: {e}", info.adapter))?;
        let device = device.ok_or("no D3D11 device")?;
        let context = context.ok_or("no D3D11 context")?;

        let output1: IDXGIOutput1 = output.cast().map_err(|e| format!("IDXGIOutput1: {e}"))?;
        let dup = unsafe { output1.DuplicateOutput(&device) }.map_err(|e| {
            if e.code() == DXGI_ERROR_UNSUPPORTED {
                format!(
                    "DuplicateOutput unsupported on {}: the capture must run on the GPU \
                     that drives this screen. Set kolada-bench to 'Power saving' in \
                     Settings > Display > Graphics, or pick another --monitor.",
                    info.adapter
                )
            } else {
                format!("DuplicateOutput: {e}")
            }
        })?;
        let mode = unsafe { dup.GetDesc() }.ModeDesc;
        let (w, h) = (mode.Width, mode.Height);
        let (tw, th) = ((w >> MIP).max(1), (h >> MIP).max(1));

        let mips_desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: MIP + 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
        };
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: tw,
            Height: th,
            MipLevels: 1,
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
            ..mips_desc
        };
        let mut mips = None;
        let mut staging = None;
        let mut mips_view = None;
        unsafe {
            device
                .CreateTexture2D(&mips_desc, None, Some(&mut mips))
                .map_err(|e| format!("mip texture: {e}"))?;
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .map_err(|e| format!("staging texture: {e}"))?;
            let mips = mips.as_ref().ok_or("no mip texture")?;
            device
                .CreateShaderResourceView(mips, None, Some(&mut mips_view))
                .map_err(|e| format!("mip view: {e}"))?;
        }

        Ok(Duplicator {
            dup,
            context,
            mips: mips.ok_or("no mip texture")?,
            mips_view: mips_view.ok_or("no mip view")?,
            staging: staging.ok_or("no staging texture")?,
            width: tw,
            height: th,
            desktop: info.desktop,
            _device: device,
        })
    }

    /// Waits up to `timeout_ms` for a newly presented desktop frame.
    /// Pointer-only updates (no present time) are skipped.
    fn next(&mut self, timeout_ms: u32) -> Result<Option<Frame>, String> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        if let Err(e) = unsafe { self.dup.AcquireNextFrame(timeout_ms, &mut info, &mut resource) } {
            if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                return Ok(None);
            }
            return Err(format!("AcquireNextFrame: {e}"));
        }
        let frame = if info.LastPresentTime == 0 {
            Ok(None)
        } else {
            let qpc = info.LastPresentTime;
            self.thumbnail(resource).map(|lum| Some(Frame { qpc, lum }))
        };
        let _ = unsafe { self.dup.ReleaseFrame() };
        frame
    }

    fn thumbnail(&self, resource: Option<IDXGIResource>) -> Result<Vec<u8>, String> {
        let texture: ID3D11Texture2D = resource
            .ok_or("frame without desktop image")?
            .cast()
            .map_err(|e| format!("desktop texture: {e}"))?;
        let (tw, th) = (self.width as usize, self.height as usize);
        let mut lum = vec![0u8; tw * th];
        unsafe {
            self.context
                .CopySubresourceRegion(&self.mips, 0, 0, 0, 0, &texture, 0, None);
            self.context.GenerateMips(&self.mips_view);
            self.context
                .CopySubresourceRegion(&self.staging, 0, 0, 0, 0, &self.mips, MIP, None);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            // Map waits for the GPU, so the desktop image is fully read
            // before the caller releases the frame back to DWM.
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| format!("Map: {e}"))?;
            for y in 0..th {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    tw * 4,
                );
                for (x, px) in row.as_chunks::<4>().0.iter().enumerate() {
                    let (b, g, r) = (px[0] as u32, px[1] as u32, px[2] as u32);
                    lum[y * tw + x] = ((r * 54 + g * 183 + b * 19) >> 8) as u8;
                }
            }
            self.context.Unmap(&self.staging, 0);
        }
        Ok(lum)
    }
}
