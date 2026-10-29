// Снимок области монитора: через видеокарту (dxgi.rs) или запасной способ GDI, затем JPEG
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
    SelectObject, SetStretchBltMode, StretchBlt, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, COLORONCOLOR,
    DIB_RGB_COLORS, SRCCOPY,
};

// Запасной способ: снимок области монитора средствами GDI, ow x oh, пиксели BGRA
pub fn capture_gdi(sx: i32, sy: i32, sw: i32, sh: i32, ow: i32, oh: i32) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; (ow * oh * 4) as usize];
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, ow, oh);
        let old = SelectObject(mem, bmp.into());
        SetStretchBltMode(mem, COLORONCOLOR);
        let ok = StretchBlt(mem, 0, 0, ow, oh, Some(screen), sx, sy, sw, sh, SRCCOPY).as_bool();
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ow,
            biHeight: -oh,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let lines = GetDIBits(mem, bmp, 0, oh as u32, Some(buf.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if !ok || lines == 0 {
            return None;
        }
    }
    Some(buf)
}

pub fn encode_jpeg(buf: &[u8], ow: i32, oh: i32, quality: u8) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(64 * 1024);
    jpeg_encoder::Encoder::new(&mut out, quality)
        .encode(buf, ow as u16, oh as u16, jpeg_encoder::ColorType::Bgra)
        .ok()?;
    Some(out)
}
