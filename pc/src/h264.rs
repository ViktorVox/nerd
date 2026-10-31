// Кодирование кадров в H.264 средствами Windows (Media Foundation, кодер Microsoft без задержки на буферизацию)
use crate::log;
use std::mem::ManuallyDrop;
use std::time::{Duration, Instant};
use windows::core::{Interface, GUID};
use windows::Win32::Foundation::VARIANT_BOOL;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0_0, VT_BOOL, VT_UI4};

fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

fn var_u32(v: u32) -> VARIANT {
    let mut inner = VARIANT_0_0::default();
    inner.vt = VT_UI4;
    inner.Anonymous.ulVal = v;
    let mut var = VARIANT::default();
    var.Anonymous.Anonymous = ManuallyDrop::new(inner);
    var
}

fn var_bool(v: bool) -> VARIANT {
    let mut inner = VARIANT_0_0::default();
    inner.vt = VT_BOOL;
    inner.Anonymous.boolVal = VARIANT_BOOL(if v { -1 } else { 0 });
    let mut var = VARIANT::default();
    var.Anonymous.Anonymous = ManuallyDrop::new(inner);
    var
}

fn chk<T>(r: windows::core::Result<T>, what: &str) -> Option<T> {
    r.map_err(|e| log!("H.264: {what}: {e}")).ok()
}

pub struct H264 {
    mft: IMFTransform,
    // Есть только у кодера видеокарты: он асинхронный и сообщает о готовности событиями
    events: Option<IMFMediaEventGenerator>,
    codec: Option<ICodecAPI>,
    w: usize,
    h: usize,
    duration: i64,
    pts: i64,
    // Размер выходного буфера, если кодер сам кадры не выдаёт
    out_size: Option<u32>,
    nv12: Vec<u8>,
    need_input: u32,
    pending: Vec<u8>,
    pub name: String,
}

impl H264 {
    // hardware: true - кодер видеокарты (NVENC, Quick Sync, AMD), false - программный кодер Microsoft
    pub fn new(w: i32, h: i32, fps: u32, kbps: u32, hardware: bool) -> Option<Self> {
        let (w, h) = ((w & !1).max(16) as usize, (h & !1).max(16) as usize);
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            chk(MFStartup(MF_VERSION, MFSTARTUP_FULL), "запуск Media Foundation")?;
            let tin = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_NV12 };
            let tout = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
            let flags = if hardware { MFT_ENUM_FLAG_HARDWARE } else { MFT_ENUM_FLAG_SYNCMFT } | MFT_ENUM_FLAG_SORTANDFILTER;
            let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;
            chk(MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags, Some(&tin), Some(&tout), &mut list, &mut count), "поиск кодера")?;
            if list.is_null() || count == 0 {
                log!("H.264: {} кодер не найден", if hardware { "аппаратный" } else { "программный" });
                return None;
            }
            let acts = std::slice::from_raw_parts_mut(list, count as usize);
            let mut found: Option<(IMFTransform, String)> = None;
            for a in acts.iter().flatten() {
                let mut buf = [0u16; 128];
                let _ = a.GetString(&MFT_FRIENDLY_NAME_Attribute, &mut buf, None);
                let name = String::from_utf16_lossy(&buf).trim_end_matches('\0').to_string();
                if let Ok(m) = a.ActivateObject::<IMFTransform>() {
                    found = Some((m, name));
                    break;
                }
            }
            for a in acts.iter_mut() {
                a.take();
            }
            CoTaskMemFree(Some(list as *const _));
            let (mft, name) = found?;

            let events = if hardware {
                let attrs = chk(mft.GetAttributes(), "атрибуты кодера")?;
                chk(attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1), "разблокировка асинхронного кодера")?;
                let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
                Some(chk(mft.cast::<IMFMediaEventGenerator>(), "события кодера")?)
            } else {
                None
            };

            let codec = mft.cast::<ICodecAPI>().ok();
            if let Some(c) = &codec {
                let set = |g: &GUID, v: VARIANT| {
                    if let Err(e) = c.SetValue(g, &v) {
                        log!("H.264: параметр кодера не принят: {e}");
                    }
                };
                set(&CODECAPI_AVLowLatencyMode, var_bool(true));
                set(&CODECAPI_AVEncCommonRealTime, var_bool(true));
                set(&CODECAPI_AVEncCommonRateControlMode, var_u32(0));
                set(&CODECAPI_AVEncCommonMeanBitRate, var_u32(kbps * 1000));
                set(&CODECAPI_AVEncMPVGOPSize, var_u32(fps * 10));
                set(&CODECAPI_AVEncMPVDefaultBPictureCount, var_u32(0));
                if !hardware {
                    set(&CODECAPI_AVEncCommonQualityVsSpeed, var_u32(30));
                }
            }

            let video = |sub: &GUID| -> windows::core::Result<IMFMediaType> {
                let t = MFCreateMediaType()?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                t.SetGUID(&MF_MT_SUBTYPE, sub)?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, pack(w as u32, h as u32))?;
                t.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
                t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
                t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                Ok(t)
            };
            // Сначала выходной тип, потом входной
            let out = chk(video(&MFVideoFormat_H264), "тип H.264")?;
            chk(out.SetUINT32(&MF_MT_AVG_BITRATE, kbps * 1000), "битрейт")?;
            chk(out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32), "профиль")?;
            chk(mft.SetOutputType(0, &out, 0), "выходной тип")?;
            let inp = chk(video(&MFVideoFormat_NV12), "тип NV12")?;
            chk(mft.SetInputType(0, &inp, 0), "входной тип")?;
            chk(mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0), "начало потока")?;
            chk(mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0), "старт потока")?;
            let info = chk(mft.GetOutputStreamInfo(0), "параметры выхода")?;
            let out_size = (info.dwFlags & 0x100 == 0).then(|| info.cbSize.max(1 << 20));
            Some(H264 {
                mft,
                events,
                codec,
                w,
                h,
                duration: 10_000_000 / fps.max(1) as i64,
                pts: 0,
                out_size,
                nv12: Vec::new(),
                need_input: 0,
                pending: Vec::new(),
                name,
            })
        }
    }

    pub fn hardware(&self) -> bool {
        self.events.is_some()
    }

    // bgra - кадр w x h. Возвращает поток H.264 (Annex B); None - кадр не получился
    pub fn encode(&mut self, bgra: &[u8], key: bool) -> Option<Vec<u8>> {
        if bgra.len() < self.w * self.h * 4 {
            return None;
        }
        bgra_to_nv12(bgra, self.w, self.h, &mut self.nv12);
        unsafe {
            let len = self.nv12.len() as u32;
            let buf = chk(MFCreateMemoryBuffer(len), "буфер кадра")?;
            let mut p = std::ptr::null_mut();
            chk(buf.Lock(&mut p, None, None), "доступ к буферу")?;
            std::ptr::copy_nonoverlapping(self.nv12.as_ptr(), p, self.nv12.len());
            chk(buf.Unlock(), "освобождение буфера")?;
            chk(buf.SetCurrentLength(len), "длина буфера")?;
            let sample = chk(MFCreateSample(), "образец")?;
            chk(sample.AddBuffer(&buf), "добавление буфера")?;
            chk(sample.SetSampleTime(self.pts), "время кадра")?;
            chk(sample.SetSampleDuration(self.duration), "длительность кадра")?;
            self.pts += self.duration;
            if key {
                if let Some(c) = &self.codec {
                    let v = var_u32(1);
                    let _ = c.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &v);
                }
            }
            if self.events.is_some() {
                self.encode_async(&sample)
            } else {
                chk(self.mft.ProcessInput(0, &sample, 0), "подача кадра")?;
                let mut data = Vec::new();
                loop {
                    match self.pull() {
                        Ok(Some(d)) => data.extend_from_slice(&d),
                        Ok(None) => break,
                        Err(e) => {
                            log!("H.264: выдача кадра: {e}");
                            return None;
                        }
                    }
                }
                (!data.is_empty()).then_some(data)
            }
        }
    }

    // Кодер видеокарты: ждём события «готов принять кадр» и «кадр готов»; не дольше 300 мс
    unsafe fn encode_async(&mut self, sample: &IMFSample) -> Option<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_millis(300);
        while self.need_input == 0 {
            if !self.wait_event(deadline)? {
                log!("H.264: кодер видеокарты не готов принять кадр");
                return None;
            }
        }
        chk(self.mft.ProcessInput(0, sample, 0), "подача кадра")?;
        self.need_input -= 1;
        while self.pending.is_empty() {
            if !self.wait_event(deadline)? {
                log!("H.264: кодер видеокарты не выдал кадр");
                return None;
            }
        }
        Some(std::mem::take(&mut self.pending))
    }

    // Some(true) - событие разобрано, Some(false) - время вышло, None - ошибка
    unsafe fn wait_event(&mut self, deadline: Instant) -> Option<bool> {
        let events = self.events.clone()?;
        loop {
            match events.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
                Ok(ev) => {
                    let ty = chk(ev.GetType(), "тип события")?;
                    if ty == METransformNeedInput.0 as u32 {
                        self.need_input += 1;
                    } else if ty == METransformHaveOutput.0 as u32 {
                        match self.pull() {
                            Ok(Some(d)) => self.pending.extend_from_slice(&d),
                            Ok(None) => {}
                            Err(e) => {
                                log!("H.264: выдача кадра: {e}");
                                return None;
                            }
                        }
                    }
                    return Some(true);
                }
                Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => {
                    if Instant::now() >= deadline {
                        return Some(false);
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
                Err(e) => {
                    log!("H.264: события кодера: {e}");
                    return None;
                }
            }
        }
    }

    // Один вызов ProcessOutput: Some - готовые данные, None - кодеру нужен следующий кадр
    unsafe fn pull(&mut self) -> windows::core::Result<Option<Vec<u8>>> {
        let out_sample = match self.out_size {
            Some(size) => {
                let s = MFCreateSample()?;
                s.AddBuffer(&MFCreateMemoryBuffer(size)?)?;
                Some(s)
            }
            None => None,
        };
        let mut out = [MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: ManuallyDrop::new(out_sample),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        }];
        let mut status = 0u32;
        let r = self.mft.ProcessOutput(0, &mut out, &mut status);
        let got = ManuallyDrop::take(&mut out[0].pSample);
        ManuallyDrop::drop(&mut out[0].pEvents);
        match r {
            Ok(()) => {
                let Some(s) = got else { return Ok(None) };
                let b = s.ConvertToContiguousBuffer()?;
                let (mut q, mut n) = (std::ptr::null_mut(), 0u32);
                b.Lock(&mut q, None, Some(&mut n))?;
                let data = std::slice::from_raw_parts(q, n as usize).to_vec();
                b.Unlock()?;
                Ok(Some(data))
            }
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Drop for H264 {
    fn drop(&mut self) {
        unsafe {
            let _ = self.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = MFShutdown();
        }
    }
}

// Яркость и цвет по BT.601 (ограниченный диапазон); цвет берётся средним из четырёх пикселей
fn convert_band(src: &[u8], w: usize, first: usize, y: &mut [u8], uv: &mut [u8]) {
    for pair in 0..y.len() / w / 2 {
        let r0 = (first + pair * 2) * w * 4;
        let r1 = r0 + w * 4;
        for x in (0..w).step_by(2) {
            let (mut su, mut sv) = (0i32, 0i32);
            for (dy, row) in [r0, r1].into_iter().enumerate() {
                for dx in 0..2 {
                    let i = row + (x + dx) * 4;
                    let (b, g, r) = (src[i] as i32, src[i + 1] as i32, src[i + 2] as i32);
                    y[(pair * 2 + dy) * w + x + dx] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
                    su += ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                    sv += ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                }
            }
            uv[pair * w + x] = (su / 4).clamp(0, 255) as u8;
            uv[pair * w + x + 1] = (sv / 4).clamp(0, 255) as u8;
        }
    }
}

fn bgra_to_nv12(src: &[u8], w: usize, h: usize, dst: &mut Vec<u8>) {
    dst.resize(w * h * 3 / 2, 0);
    let (y, uv) = dst.split_at_mut(w * h);
    let bands = 4.min(h / 2).max(1);
    let pairs = (h / 2).div_ceil(bands);
    std::thread::scope(|s| {
        for (i, (yb, uvb)) in y.chunks_mut(w * 2 * pairs).zip(uv.chunks_mut(w * pairs)).enumerate() {
            s.spawn(move || convert_band(src, w, i * pairs * 2, yb, uvb));
        }
    });
}
