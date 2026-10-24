// Streams the PC's audio (WASAPI loopback capture of the default render
// endpoint) to one client as an ADTS wrapped AAC stream (see
// lib/encode_audio.c). One thread per HTTP connection: it stops when the
// browser closes or stops reading the stream. Silence is encoded whenever
// the desktop is quiet so the client keeps buffering and never stalls.

use std::os::raw::{c_int, c_uchar, c_void};

use tracing::warn;

use crate::cerror::CError;

extern "C" {
    fn init_audio_encoder(rust_ctx: *mut c_void) -> *mut c_void;
    fn init_audio_sink(handle: *mut c_void, sample_rate_in: c_int, channels_in: c_int, err: *mut CError);
    fn destroy_audio_encoder(handle: *mut c_void);
    fn get_audio_frame_size(handle: *mut c_void) -> c_int;
    fn encode_audio_chunk(handle: *mut c_void, samples: *const c_void, frames: c_int, err: *mut CError);
}

pub struct AudioEncoder {
    handle: *mut c_void,
    frame_size: usize,
    write_data: Box<dyn FnMut(&[u8])>,
}

impl AudioEncoder {
    pub fn new(
        sample_rate_in: usize,
        channels_in: usize,
        write_data: impl FnMut(&[u8]) + 'static,
    ) -> Result<Box<Self>, CError> {
        let mut encoder = Box::new(Self {
            handle: std::ptr::null_mut(),
            frame_size: 0,
            write_data: Box::new(write_data),
        });
        let handle = unsafe { init_audio_encoder(encoder.as_mut() as *mut _ as *mut c_void) };
        encoder.handle = handle;
        let mut err = CError::new();
        unsafe {
            init_audio_sink(encoder.handle, sample_rate_in as c_int, channels_in as c_int, &mut err)
        };
        if err.is_err() {
            unsafe { destroy_audio_encoder(encoder.handle) };
            return Err(err);
        }
        encoder.frame_size = unsafe { get_audio_frame_size(encoder.handle) } as usize;
        Ok(encoder)
    }

    // samples: interleaved f32 in the capture format or empty for silence
    pub fn encode(&mut self, samples: &[f32], frames: usize) {
        let mut err = CError::new();
        let ptr = if frames > 0 {
            samples.as_ptr() as *const c_void
        } else {
            std::ptr::null()
        };
        unsafe { encode_audio_chunk(self.handle, ptr, frames as c_int, &mut err) };
        if err.is_err() {
            warn!("Failed to encode audio: {}", err);
        }
    }
}

impl Drop for AudioEncoder {
    fn drop(&mut self) {
        unsafe { destroy_audio_encoder(self.handle) };
    }
}

// this callback is called from lib/encode_audio.c for every encoded packet
#[no_mangle]
fn write_audio_packet(encoder: *mut c_void, buf: *const c_uchar, buf_size: c_int) -> c_int {
    let encoder = unsafe { (encoder as *mut AudioEncoder).as_mut().unwrap() };
    (encoder.write_data)(unsafe {
        std::slice::from_raw_parts(buf as *const u8, buf_size as usize)
    });
    0
}

#[cfg(windows)]
pub fn audio_loopback(sender: tokio::sync::mpsc::Sender<bytes::Bytes>) {
    unsafe {
        if let Err(err) = wasapi_capture::capture_loop(sender) {
            warn!("Audio capture stopped: {}", err);
        }
    }
}

#[cfg(not(windows))]
pub fn audio_loopback(_sender: tokio::sync::mpsc::Sender<bytes::Bytes>) {}

#[cfg(windows)]
mod wasapi_capture {
    use std::os::raw::c_void;
    use std::ptr::null_mut;
    use std::slice;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use bytes::Bytes;
    use tokio::sync::mpsc;
    use winapi::shared::guiddef::GUID;
    use winapi::shared::minwindef::FALSE;
    use winapi::shared::mmreg::{WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVE_FORMAT_EXTENSIBLE};
    use winapi::um::audioclient::{
        IAudioCaptureClient, IAudioClient, IID_IAudioCaptureClient, IID_IAudioClient,
    };
    use winapi::um::audiosessiontypes::AUDCLNT_SHAREMODE_SHARED;
    use winapi::um::combaseapi::{CLSCTX_ALL, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize};
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::mmdeviceapi::{
        CLSID_MMDeviceEnumerator, IMMDevice, IMMDeviceEnumerator, eMultimedia, eRender,
    };
    use winapi::um::objbase::COINIT_MULTITHREADED;
    use winapi::um::synchapi::{CreateEventW, WaitForSingleObject};

    const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x00020000;
    const AUDCLNT_STREAMFLAGS_EVENTCALLBACK: u32 = 0x00040000;
    const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x00000002;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_FAILED: u32 = 0xFFFFFFFF;
    // reference time is in 100 ns units: 20 ms capture buffer
    const CAPTURE_BUFFER: i64 = 200_000;

    // {A95664D2-9614-4F35-A746-DE8DB63617E6}
    const IID_IMMDeviceEnumerator: GUID = GUID {
        Data1: 0xA95664D2,
        Data2: 0x9614,
        Data3: 0x4F35,
        Data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
    };

    pub unsafe fn capture_loop(sender: mpsc::Sender<Bytes>) -> Result<(), String> {
        let _ = CoInitializeEx(null_mut(), COINIT_MULTITHREADED);

        let mut enumerator: *mut IMMDeviceEnumerator = null_mut();
        let result = CoCreateInstance(
            &CLSID_MMDeviceEnumerator,
            null_mut(),
            CLSCTX_ALL,
            &IID_IMMDeviceEnumerator,
            &mut enumerator as *mut *mut IMMDeviceEnumerator as *mut *mut c_void,
        );
        if result != 0 || enumerator.is_null() {
            return Err(format!("Failed to create MMDeviceEnumerator: hr={:#08x}", result));
        }

        let mut device: *mut IMMDevice = null_mut();
        let result = ((*(*enumerator).lpVtbl).GetDefaultAudioEndpoint)(
            enumerator,
            eRender,
            eMultimedia,
            &mut device,
        );
        if result != 0 || device.is_null() {
            return Err(format!("Failed to get default render endpoint: hr={:#08x}", result));
        }

        let mut audio_client: *mut IAudioClient = null_mut();
        let result = ((*(*device).lpVtbl).Activate)(
            device,
            &IID_IAudioClient,
            CLSCTX_ALL,
            null_mut(),
            &mut audio_client as *mut *mut IAudioClient as *mut *mut c_void,
        );
        if result != 0 || audio_client.is_null() {
            return Err(format!("Failed to activate IAudioClient: hr={:#08x}", result));
        }

        let mut wfex_ptr: *mut WAVEFORMATEX = null_mut();
        let result = ((*(*audio_client).lpVtbl).GetMixFormat)(audio_client, &mut wfex_ptr);
        if result != 0 || wfex_ptr.is_null() {
            return Err(format!("Failed to get mix format: hr={:#08x}", result));
        }

        // WASAPI shared mode loopback returns the mix format which on
        // practically every machine is packed floating point, but verify
        // anyway since the encoder expects exactly that.
        let sample_rate = (*wfex_ptr).nSamplesPerSec as usize;
        let channels = (*wfex_ptr).nChannels as usize;
        let is_float = if (*wfex_ptr).wFormatTag == WAVE_FORMAT_EXTENSIBLE {
            // WAVEFORMATEXTENSIBLE is packed: copy the GUID via read_unaligned
            let sub = std::ptr::addr_of!((*(wfex_ptr as *const WAVEFORMATEXTENSIBLE)).SubFormat)
                .read_unaligned();
            guid_eq(&sub, &winapi::shared::ksmedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT)
        } else {
            (*wfex_ptr).wFormatTag == 0x0003 // WAVE_FORMAT_IEEE_FLOAT
        };
        if !is_float || channels == 0 || channels > 8 {
            let tag = (*wfex_ptr).wFormatTag;
            CoTaskMemFree(wfex_ptr as *mut c_void);
            return Err(format!(
                "Unsupported mix format (tag={:#06x}, {} Hz, {} channels)!",
                tag, sample_rate, channels
            ));
        }

        let event = CreateEventW(null_mut(), FALSE, FALSE, null_mut());
        if event.is_null() {
            CoTaskMemFree(wfex_ptr as *mut c_void);
            return Err("Failed to create capture event!".to_owned());
        }

        let result = ((*(*audio_client).lpVtbl).Initialize)(
            audio_client,
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            CAPTURE_BUFFER,
            0,
            wfex_ptr,
            null_mut(),
        );
        CoTaskMemFree(wfex_ptr as *mut c_void);
        if result != 0 {
            return Err(format!("Failed to initialize loopback capture: hr={:#08x}", result));
        }

        let result = ((*(*audio_client).lpVtbl).SetEventHandle)(audio_client, event);
        if result != 0 {
            return Err(format!("Failed to set capture event: hr={:#08x}", result));
        }

        let mut capture: *mut IAudioCaptureClient = null_mut();
        let result = ((*(*audio_client).lpVtbl).GetService)(
            audio_client,
            &IID_IAudioCaptureClient,
            &mut capture as *mut *mut IAudioCaptureClient as *mut *mut c_void,
        );
        if result != 0 || capture.is_null() {
            return Err(format!("Failed to get capture client: hr={:#08x}", result));
        }

        let result = ((*(*audio_client).lpVtbl).Start)(audio_client);
        if result != 0 {
            return Err(format!("Failed to start capture: hr={:#08x}", result));
        }

        let stopped = Arc::new(AtomicBool::new(false));
        let stop_flag = stopped.clone();
        let mut encoder = match super::AudioEncoder::new(sample_rate, channels, move |data: &[u8]| {
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }
            if sender.blocking_send(Bytes::copy_from_slice(data)).is_err() {
                stop_flag.store(true, Ordering::Relaxed);
            }
        }) {
            Ok(encoder) => encoder,
            Err(err) => {
                return Err(format!("Failed to init audio encoder: {}", err));
            }
        };

        let mut pcm: Vec<f32> = Vec::with_capacity(8192 * if channels > 2 { 2 } else { channels });

        // run until the client disconnects, which turns every send into an
        // error and flips the flag
        while !stopped.load(Ordering::Relaxed) {
            let wait = WaitForSingleObject(event, 25);
            if wait == WAIT_FAILED {
                break;
            }

            // drain everything the capture queue currently holds
            loop {
                let mut packet_size: u32 = 0;
                ((*(*capture).lpVtbl).GetNextPacketSize)(capture, &mut packet_size);
                if packet_size == 0 {
                    break;
                }
                let mut data: *mut u8 = null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;
                let result = ((*(*capture).lpVtbl).GetBuffer)(
                    capture,
                    &mut data,
                    &mut frames,
                    &mut flags,
                    null_mut(),
                    null_mut(),
                );
                if result != 0 {
                    break;
                }
                let sample_count = frames as usize * channels;
                if flags & AUDCLNT_BUFFERFLAGS_SILENT != 0 {
                    pcm.resize(pcm.len() + sample_count, 0.0);
                } else {
                    pcm.extend_from_slice(slice::from_raw_parts(data as *const f32, sample_count));
                }
                ((*(*capture).lpVtbl).ReleaseBuffer)(capture, frames);
            }

            if pcm.len() >= channels {
                let frame_count = pcm.len() / channels;
                encoder.encode(&pcm, frame_count);
                pcm.clear();
            } else {
                // desktop is completely silent (no loopback packets at
                // all): keep feeding silent frames so the browser
                // continues buffering the stream
                encoder.encode(&[], 0);
            }
        }

        ((*(*audio_client).lpVtbl).Stop)(audio_client);
        CloseHandle(event);
        drop(encoder);
        let capture_unk = capture as *mut winapi::um::unknwnbase::IUnknown;
        ((*(*capture_unk).lpVtbl).Release)(capture_unk);
        ((*(*audio_client).lpVtbl).parent.Release)(audio_client as *mut winapi::um::unknwnbase::IUnknown);
        ((*(*device).lpVtbl).parent.Release)(device as *mut winapi::um::unknwnbase::IUnknown);
        ((*(*enumerator).lpVtbl).parent.Release)(enumerator as *mut winapi::um::unknwnbase::IUnknown);
        CoUninitialize();
        Ok(())
    }

    fn guid_eq(a: &GUID, b: &GUID) -> bool {
        a.Data1 == b.Data1 && a.Data2 == b.Data2 && a.Data3 == b.Data3 && a.Data4 == b.Data4
    }
}
