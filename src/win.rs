//! Small Windows helpers: global hotkey, notification sound, work area.
//! On other platforms everything is a no-op so the crate still builds.

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Global hotkey Ctrl+Alt+Space, handled on its own thread with its own message loop.
pub struct Hotkey {
    pub fired: Arc<AtomicBool>,
    pub ok: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    #[allow(dead_code)]
    thread_id: Arc<std::sync::atomic::AtomicU32>,
}

impl Hotkey {
    pub fn start(ctx: egui::Context, enabled: bool) -> Self {
        let hk = Self {
            fired: Arc::new(AtomicBool::new(false)),
            ok: Arc::new(AtomicBool::new(false)),
            enabled: Arc::new(AtomicBool::new(enabled)),
            thread_id: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        };
        #[cfg(windows)]
        {
            let fired = hk.fired.clone();
            let ok = hk.ok.clone();
            let en = hk.enabled.clone();
            let tid = hk.thread_id.clone();
            std::thread::spawn(move || imp::hotkey_loop(ctx, fired, ok, en, tid));
        }
        #[cfg(not(windows))]
        {
            let _ = ctx;
        }
        hk
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
        #[cfg(windows)]
        imp::wake(self.thread_id.load(Ordering::SeqCst));
    }

    pub fn take(&self) -> bool {
        self.fired.swap(false, Ordering::SeqCst)
    }

    pub fn registered(&self) -> bool {
        self.ok.load(Ordering::SeqCst)
    }
}

/// soft two-note chime, generated once, played asynchronously
pub fn chime() {
    #[cfg(windows)]
    imp::play(chime_wav());
}

#[allow(dead_code)]
fn chime_wav() -> &'static [u8] {
    static WAV: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    WAV.get_or_init(|| {
        let sr = 44_100u32;
        let len = (sr as f32 * 1.4) as usize;
        // E5 then B5, gentle attack, long exponential tail, a quiet octave partial for warmth
        let notes = [(659.25_f32, 0.0_f32, 0.55_f32), (987.77, 0.13, 0.45)];
        let mut pcm: Vec<i16> = Vec::with_capacity(len);
        for i in 0..len {
            let t = i as f32 / sr as f32;
            let mut v = 0.0_f32;
            for (f, start, amp) in notes {
                let dt = t - start;
                if dt < 0.0 {
                    continue;
                }
                let attack = (dt / 0.012).min(1.0);
                let env = attack * (-dt * 3.2).exp();
                let w = std::f32::consts::TAU;
                v += amp * env * ((w * f * dt).sin() + 0.18 * (w * 2.0 * f * dt).sin());
            }
            // fade out the very end to avoid a click
            let tail = ((len - i) as f32 / (sr as f32 * 0.05)).min(1.0);
            let s = (v * 0.16 * tail).clamp(-1.0, 1.0);
            pcm.push((s * i16::MAX as f32) as i16);
        }
        let data_len = (pcm.len() * 2) as u32;
        let mut b = Vec::with_capacity(44 + data_len as usize);
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&1u16.to_le_bytes()); // mono
        b.extend_from_slice(&sr.to_le_bytes());
        b.extend_from_slice(&(sr * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for s in pcm {
            b.extend_from_slice(&s.to_le_bytes());
        }
        b
    })
}

/// work area of the primary monitor (screen minus taskbar), in points
#[cfg(windows)]
pub fn work_area(pixels_per_point: f32) -> Option<egui::Rect> {
    let (l, t, r, b) = imp::work_area_px()?;
    let k = pixels_per_point.max(0.5);
    let rect = egui::Rect::from_min_max(
        egui::pos2(l as f32 / k, t as f32 / k),
        egui::pos2(r as f32 / k, b as f32 / k),
    );
    (rect.width() > 200.0 && rect.height() > 200.0).then_some(rect)
}

#[cfg(not(windows))]
pub fn work_area(_pixels_per_point: f32) -> Option<egui::Rect> {
    None
}

#[cfg(windows)]
mod imp {
    use eframe::egui;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_SPACE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMessageW, PostThreadMessageW, SystemParametersInfoW, MSG, SPI_GETWORKAREA, WM_APP,
        WM_HOTKEY,
    };

    const HK_ID: i32 = 0x4153; // "AS"

    pub fn hotkey_loop(
        ctx: egui::Context,
        fired: Arc<AtomicBool>,
        ok: Arc<AtomicBool>,
        enabled: Arc<AtomicBool>,
        tid: Arc<AtomicU32>,
    ) {
        unsafe {
            tid.store(GetCurrentThreadId(), Ordering::SeqCst);
            let mut registered = false;
            loop {
                let want = enabled.load(Ordering::SeqCst);
                if want && !registered {
                    registered = RegisterHotKey(
                        std::ptr::null_mut(),
                        HK_ID,
                        MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                        VK_SPACE as u32,
                    ) != 0;
                    ok.store(registered, Ordering::SeqCst);
                    ctx.request_repaint();
                } else if !want && registered {
                    UnregisterHotKey(std::ptr::null_mut(), HK_ID);
                    registered = false;
                    ok.store(false, Ordering::SeqCst);
                }
                let mut msg: MSG = std::mem::zeroed();
                let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if r == 0 || r == -1 {
                    break;
                }
                if msg.message == WM_HOTKEY && registered {
                    fired.store(true, Ordering::SeqCst);
                    ctx.request_repaint();
                }
                // WM_APP only wakes the loop to re-check `enabled`
            }
        }
    }

    pub fn wake(tid: u32) {
        if tid != 0 {
            unsafe {
                PostThreadMessageW(tid, WM_APP, 0, 0);
            }
        }
    }

    pub fn play(wav: &'static [u8]) {
        unsafe {
            PlaySoundW(
                wav.as_ptr() as *const u16,
                std::ptr::null_mut(),
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }

    pub fn work_area_px() -> Option<(i32, i32, i32, i32)> {
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            let ok = SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                &mut r as *mut RECT as *mut core::ffi::c_void,
                0,
            );
            if ok != 0 {
                Some((r.left, r.top, r.right, r.bottom))
            } else {
                None
            }
        }
    }
}
