// Захват экрана через видеокарту (Desktop Duplication): кадр остаётся в памяти GPU,
// на процессор копируется только нужная область.
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BOX,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

pub struct Dxgi {
    ctx: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    last: ID3D11Texture2D,
    staging: ID3D11Texture2D,
    w: i32,
    h: i32,
    have_image: bool,
}

impl Dxgi {
    // mon = (x0, y0, w, h): прямоугольник монитора на рабочем столе
    // seed: полный снимок монитора (BGRA) на случай, если видеокарта долго не присылает картинку
    pub fn new(mon: (i32, i32, i32, i32), seed: impl FnOnce() -> Option<Vec<u8>>) -> Option<Self> {
        let (x0, y0, w, h) = mon;
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
            let mut found = None;
            let mut i = 0;
            'outer: while let Ok(adapter) = factory.EnumAdapters1(i) {
                let mut j = 0;
                while let Ok(output) = adapter.EnumOutputs(j) {
                    let r = output.GetDesc().ok()?.DesktopCoordinates;
                    if r.left == x0 && r.top == y0 {
                        found = Some((adapter, output));
                        break 'outer;
                    }
                    j += 1;
                }
                i += 1;
            }
            let (adapter, output) = found?;

            let adapter0: IDXGIAdapter = adapter.cast().ok()?;
            let mut device: Option<ID3D11Device> = None;
            let mut ctx: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                &adapter0,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
            .ok()?;
            let (device, ctx) = (device?, ctx?);

            let output1: IDXGIOutput1 = output.cast().ok()?;
            let dup = output1.DuplicateOutput(&device).ok()?;

            let mut desc = D3D11_TEXTURE2D_DESC {
                Width: w as u32,
                Height: h as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                ..Default::default()
            };
            let mut last = None;
            device.CreateTexture2D(&desc, None, Some(&mut last)).ok()?;
            desc.Usage = D3D11_USAGE_STAGING;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            device.CreateTexture2D(&desc, None, Some(&mut staging)).ok()?;

            let mut me = Self { ctx, dup, last: last?, staging: staging?, w, h, have_image: false };
            // Настоящая картинка приходит после первой перерисовки; на неподвижном экране её нет - берём снимок
            let started = std::time::Instant::now();
            while !me.have_image && started.elapsed() < std::time::Duration::from_millis(150) {
                me.refresh(50)?;
            }
            if !me.have_image {
                let buf = seed()?;
                me.ctx.UpdateSubresource(&me.last, 0, None, buf.as_ptr() as *const _, (w * 4) as u32, 0);
                me.have_image = true;
            }
            Some(me)
        }
    }

    // Ok(false): новых кадров нет; None: захват потерян, нужно создать заново
    fn refresh(&mut self, timeout_ms: u32) -> Option<bool> {
        unsafe {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res: Option<IDXGIResource> = None;
            match self.dup.AcquireNextFrame(timeout_ms, &mut info, &mut res) {
                Ok(()) => {
                    let mut fresh = false;
                    if info.LastPresentTime != 0 {
                        if let Some(tex) = res.and_then(|r| r.cast::<ID3D11Texture2D>().ok()) {
                            self.ctx.CopyResource(&self.last, &tex);
                            fresh = true;
                            self.have_image = true;
                        }
                    }
                    let _ = self.dup.ReleaseFrame();
                    Some(fresh)
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => Some(false),
                Err(_) => None,
            }
        }
    }

    // Область (sx, sy, sw, sh относительно монитора), приведённая к ow x oh, пиксели BGRA
    pub fn grab(&mut self, sx: i32, sy: i32, sw: i32, sh: i32, ow: i32, oh: i32) -> Option<Vec<u8>> {
        self.refresh(0)?;
        let left = sx.clamp(0, self.w - 1);
        let top = sy.clamp(0, self.h - 1);
        let right = (sx + sw).clamp(left + 1, self.w);
        let bottom = (sy + sh).clamp(top + 1, self.h);
        let b = D3D11_BOX {
            left: left as u32,
            top: top as u32,
            front: 0,
            right: right as u32,
            bottom: bottom as u32,
            back: 1,
        };
        unsafe {
            self.ctx.CopySubresourceRegion(&self.staging, 0, 0, 0, 0, &self.last, 0, Some(&b));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).ok()?;
            let out = resample(
                mapped.pData as *const u8,
                mapped.RowPitch as usize,
                (right - left) as usize,
                (bottom - top) as usize,
                ow as usize,
                oh as usize,
            );
            self.ctx.Unmap(&self.staging, 0);
            Some(out)
        }
    }
}

struct SrcPtr(*const u8);
unsafe impl Send for SrcPtr {}
unsafe impl Sync for SrcPtr {}

// Усреднение пикселей исходной области на каждый пиксель результата (при увеличении получается ближайший сосед).
// Строки результата делятся между потоками.
unsafe fn resample(src: *const u8, pitch: usize, sw: usize, sh: usize, ow: usize, oh: usize) -> Vec<u8> {
    let mut dst = vec![0u8; ow * oh * 4];
    let cols: Vec<(usize, usize)> = (0..ow)
        .map(|x| {
            let a = x * sw / ow;
            (a, ((x + 1) * sw / ow).max(a + 1).min(sw))
        })
        .collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let rows = oh.div_ceil(threads);
    let src = SrcPtr(src);
    let (src, cols) = (&src, &cols);
    std::thread::scope(|scope| {
        for (t, part) in dst.chunks_mut(rows * ow * 4).enumerate() {
            scope.spawn(move || {
                let base = src.0;
                for (i, line) in part.chunks_mut(ow * 4).enumerate() {
                    let oy = t * rows + i;
                    let ya = oy * sh / oh;
                    let yb = ((oy + 1) * sh / oh).max(ya + 1).min(sh);
                    for (ox, &(xa, xb)) in cols.iter().enumerate() {
                        let (mut b, mut g, mut r) = (0u32, 0u32, 0u32);
                        for y in ya..yb {
                            let row = base.add(y * pitch);
                            for x in xa..xb {
                                let p = row.add(x * 4);
                                b += *p as u32;
                                g += *p.add(1) as u32;
                                r += *p.add(2) as u32;
                            }
                        }
                        let n = ((yb - ya) * (xb - xa)) as u32;
                        line[ox * 4] = (b / n) as u8;
                        line[ox * 4 + 1] = (g / n) as u8;
                        line[ox * 4 + 2] = (r / n) as u8;
                        line[ox * 4 + 3] = 255;
                    }
                }
            });
        }
    });
    dst
}
