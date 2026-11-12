//! Export through the GPU's own video encoder on any vendor's card: AMD (VCN / AMF), Intel (Quick
//! Sync) and others, reached as Media Foundation hardware encoder MFTs. NVIDIA cards keep using
//! [`crate::nvenc`], which is registered in front of this factory and takes their exports first.
//!
//! - H.264: like NVENC, only when Export ▸ Hardware encoding is Auto; whatever the MFT does not take
//!   falls through (`None`) to the software encoder.
//! - H.265 (HEVC Main, 8-bit 4:2:0 SDR BT.709): there is no software encoder, so choosing the format
//!   is the opt-in and a request the hardware cannot take is an error that says why. 10-bit / HDR
//!   stays NVENC-only: an HDR sequence is tone-mapped to SDR on other GPUs, as before.
//!
//! Hardware encoder MFTs are asynchronous: the transform says when it wants a picture
//! (`METransformNeedInput`) and when it has a packet (`METransformHaveOutput`). The events are polled
//! without blocking and with a deadline, so a driver that stops answering ends the export with an
//! error instead of hanging it. Pictures go in as NV12 in system memory (the MFT uploads them), the
//! packets come out as Annex B and are rewritten to the MP4's length-prefixed NAL units, with the
//! parameter sets moved into the sample entry (the same helpers as NVENC).
//!
//! FFI module (docs/adr/0001-platform-ffi.md): every `unsafe` block has a `// SAFETY:` comment, and
//! the public items are safe.

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use filmcraft_export::{
    BitrateMode, ColorSignal, EncodedPacket, EncoderFrame, ExportError, ExportSettings, FieldOrder, Format, H264Pass, H264Profile, HardwareEncoding, Result,
    VideoEncoder,
};
use filmcraft_isobmff::{AvcConfig, HevcConfig, SampleEntry};
use filmcraft_time::FrameRate;
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncCommonMaxBitRate, CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonQualityVsSpeed, CODECAPI_AVEncCommonRateControlMode,
    CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize, CODECAPI_AVLowLatencyMode, ICodecAPI, IMFActivate, IMFMediaEventGenerator, IMFMediaType,
    IMFSample, IMFTransform, MEError, METransformDrainComplete, METransformHaveOutput, METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_EVENT_FLAG_NO_WAIT, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MAX_KEYFRAME_SPACING, MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO,
    MF_MT_SUBTYPE, MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK, MFMediaType_Video, MFSampleExtension_CleanPoint, MFSampleExtension_DecodeTimestamp,
    MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_END_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFVideoFormat_H264, MFVideoFormat_HEVC, MFVideoFormat_NV12,
    MFVideoInterlace_Progressive,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{GUID, Interface};

use super::gpu::ensure_com;
use super::mft::{MfApi, api, friendly_name};
use crate::nvenc::{Codec, annex_b_to_length_prefixed_for, split_hevc_parameter_sets, split_parameter_sets};

/// How long the encoder may stay silent (no event at all) before the export is ended with an error.
const STALL: Duration = Duration::from_secs(20);
/// Hundred-nanosecond units per second (Media Foundation's clock).
const HNS: i128 = 10_000_000;
/// Environment variable naming the encoder to prefer (part of its name: "AMD", "Intel", ...).
const PREFER_ENV: &str = "FILMCRAFT_HW_ENCODER";

/// What the hardware encoder is asked to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    /// Frame rate as num / den.
    pub fps: (u32, u32),
    pub kbps: u32,
    pub max_kbps: u32,
    pub cbr: bool,
    pub keyint: u32,
    /// `MF_MT_MPEG2_PROFILE`: H.264 profile_idc (66 / 77 / 100), or 1 (`eAVEncH265VProfile_Main_420_8`).
    pub profile: u32,
}

/// The encoder configuration for an export, or why the hardware path does not take it.
pub fn config(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> std::result::Result<Config, String> {
    let codec = match format {
        Format::H264 => Codec::H264,
        Format::Hevc => Codec::Hevc,
        _ => return Err("only MP4 / MOV H.264 and H.265 exports".into()),
    };
    if s.format.is_mxf() {
        return Err("MXF carries an Annex B stream".into());
    }
    if s.signal.is_hdr() {
        return Err("HDR (10-bit) needs NVENC; this GPU encoder writes 8-bit SDR".into());
    }
    if codec == Codec::Hevc && s.signal != ColorSignal::default() {
        return Err(format!("the colour signal {:?} (H.265 here is BT.709)", s.signal));
    }
    if !matches!(s.h264_pass, H264Pass::Single) || s.bitrate_mode == BitrateMode::Vbr2Pass {
        return Err("two-pass VBR".into());
    }
    if s.bitrate_mode == BitrateMode::Crf {
        return Err("CRF (constant quality)".into());
    }
    if s.pixel_aspect.is_some_and(|(n, d)| n != d) {
        return Err("non-square pixels".into());
    }
    if s.field_order != FieldOrder::Progressive {
        return Err("interlaced output".into());
    }
    if w == 0 || h == 0 || !w.is_multiple_of(2) || !h.is_multiple_of(2) {
        return Err("the GPU encoder needs nonzero even dimensions".into());
    }
    if w > 8192 || h > 8192 {
        return Err(format!("{w}x{h} is larger than GPU encoders take"));
    }
    let (Ok(num), Ok(den)) = (u32::try_from(rate.num), u32::try_from(rate.den)) else {
        return Err("frame rate".into());
    };
    if num == 0 || den == 0 {
        return Err("frame rate".into());
    }
    let kbps = s.bitrate_kbps.clamp(100, 4_000_000);
    let fps = f64::from(num) / f64::from(den);
    Ok(Config {
        codec,
        width: w,
        height: h,
        fps: (num, den),
        kbps,
        max_kbps: s.max_bitrate_kbps.filter(|m| *m >= kbps).unwrap_or_else(|| kbps.saturating_add(kbps / 2)).min(4_000_000),
        cbr: s.bitrate_mode == BitrateMode::Cbr,
        keyint: s.keyframe_distance.filter(|k| *k > 0).unwrap_or_else(|| (fps * 2.0).round().clamp(1.0, 1000.0) as u32),
        profile: match (codec, s.h264_profile) {
            (Codec::Hevc, _) => 1,
            (_, H264Profile::Baseline) => 66,
            (_, H264Profile::Main) => 77,
            (_, H264Profile::High) => 100,
        },
    })
}

/// Picture `index` at `fps` in hundred-nanosecond units (saturating).
fn time_of(index: u64, fps: (u32, u32)) -> i64 {
    let t = i128::from(index).saturating_mul(HNS).saturating_mul(i128::from(fps.1)) / i128::from(fps.0.max(1));
    i64::try_from(t).unwrap_or(i64::MAX)
}

/// A Media Foundation time back to the nearest picture index at `fps`.
fn frame_of(t: i64, fps: (u32, u32)) -> i64 {
    let den = HNS.saturating_mul(i128::from(fps.1.max(1)));
    let n = i128::from(t).saturating_mul(i128::from(fps.0));
    let f = (n + den / 2).div_euclid(den);
    i64::try_from(f).unwrap_or(if f < 0 { i64::MIN } else { i64::MAX })
}

/// `(pts - dts) × den` as an MP4 composition offset, saturating.
fn composition_offset(pts: i64, dts: i64, den: u32) -> i32 {
    let o = i128::from(pts).saturating_sub(i128::from(dts)).saturating_mul(i128::from(den));
    i32::try_from(o).unwrap_or(if o < 0 { i32::MIN } else { i32::MAX })
}

/// Encoders in the order they are tried: the one named by `FILMCRAFT_HW_ENCODER` first, then AMD,
/// Intel, and the rest (NVIDIA last: NVENC normally takes those exports before this factory).
fn rank(name: &str, prefer: Option<&str>) -> u8 {
    let n = name.to_ascii_lowercase();
    if prefer.is_some_and(|p| !p.is_empty() && n.contains(&p.to_ascii_lowercase())) {
        return 0;
    }
    if n.contains("amd") || n.contains("radeon") || n.contains("amf") {
        1
    } else if n.contains("intel") || n.contains("quick sync") {
        2
    } else if n.contains("nvidia") {
        4
    } else {
        3
    }
}

/// Status of the transform's event queue.
enum Event {
    NeedInput,
    HaveOutput,
    DrainComplete,
    Other,
}

/// A configured hardware encoder MFT.
struct Transform {
    activate: IMFActivate,
    transform: IMFTransform,
    events: IMFMediaEventGenerator,
    name: String,
}

// SAFETY: hardware encoder MFTs are free-threaded COM objects; a `Transform` is owned by one
// export encoder and used through `&mut self` only, so from one thread at a time.
unsafe impl Send for Transform {}

impl Drop for Transform {
    fn drop(&mut self) {
        // SAFETY: plain COM calls on live interfaces; errors are irrelevant while tearing down.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            // asynchronous MFTs keep worker threads until their activation object is shut down
            let _ = self.activate.ShutdownObject();
        }
    }
}

fn set_codec_value(codec: &ICodecAPI, key: &GUID, value: VARIANT) {
    // SAFETY: both pointers refer to live values for the duration of the call. Settings an
    // encoder does not support are skipped: they are refinements, the media type carries the rest.
    unsafe {
        if codec.IsSupported(key).is_ok() {
            let _ = codec.SetValue(key, &value);
        }
    }
}

impl Transform {
    /// Activate and configure `activate` for `cfg`.
    fn open(api: &MfApi, activate: IMFActivate, cfg: &Config) -> std::result::Result<Transform, String> {
        let name = friendly_name(&activate).unwrap_or_else(|| "a hardware encoder".into());
        // SAFETY: plain COM calls on live interfaces.
        let transform: IMFTransform = unsafe { activate.ActivateObject() }.map_err(|e| format!("{name}: cannot be created: {e}"))?;
        let events: IMFMediaEventGenerator = transform.cast().map_err(|e| format!("{name}: has no event queue: {e}"))?;
        let t = Transform { activate, transform, events, name };
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let attrs = t.transform.GetAttributes().map_err(|e| format!("{}: no attributes: {e}", t.name))?;
            if attrs.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) != 0 {
                attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1).map_err(|e| format!("{}: cannot be unlocked: {e}", t.name))?;
            }
        }
        t.settings(cfg);
        t.set_types(api, cfg)?;
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            t.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).map_err(|e| format!("{}: cannot start: {e}", t.name))?;
            t.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).map_err(|e| format!("{}: cannot start: {e}", t.name))?;
        }
        Ok(t)
    }

    /// Rate control, GOP, no B-frames (so packets come in presentation order), quality over speed.
    fn settings(&self, cfg: &Config) {
        let Ok(codec) = self.transform.cast::<ICodecAPI>() else { return };
        let bps = |kbps: u32| kbps.saturating_mul(1000);
        // eAVEncCommonRateControlMode: 0 CBR, 1 peak-constrained VBR
        set_codec_value(&codec, &CODECAPI_AVEncCommonRateControlMode, VARIANT::from(if cfg.cbr { 0u32 } else { 1u32 }));
        set_codec_value(&codec, &CODECAPI_AVEncCommonMeanBitRate, VARIANT::from(bps(cfg.kbps)));
        if !cfg.cbr {
            set_codec_value(&codec, &CODECAPI_AVEncCommonMaxBitRate, VARIANT::from(bps(cfg.max_kbps)));
        }
        set_codec_value(&codec, &CODECAPI_AVEncMPVGOPSize, VARIANT::from(cfg.keyint));
        set_codec_value(&codec, &CODECAPI_AVEncMPVDefaultBPictureCount, VARIANT::from(0u32));
        set_codec_value(&codec, &CODECAPI_AVLowLatencyMode, VARIANT::from(false));
        set_codec_value(&codec, &CODECAPI_AVEncCommonQualityVsSpeed, VARIANT::from(80u32));
    }

    fn set_types(&self, api: &MfApi, cfg: &Config) -> std::result::Result<(), String> {
        let size = (u64::from(cfg.width) << 32) | u64::from(cfg.height);
        let rate = (u64::from(cfg.fps.0) << 32) | u64::from(cfg.fps.1);
        let subtype = match cfg.codec {
            Codec::H264 => MFVideoFormat_H264,
            Codec::Hevc => MFVideoFormat_HEVC,
        };
        let common = |t: &IMFMediaType, sub: &GUID| -> windows::core::Result<()> {
            // SAFETY: plain COM calls on a live media type.
            unsafe {
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                t.SetGUID(&MF_MT_SUBTYPE, sub)?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, size)?;
                t.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
                t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
                t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            }
        };
        // encoders take their output type first
        let out = api.media_type()?;
        common(&out, &subtype).map_err(|e| e.to_string())?;
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            out.SetUINT32(&MF_MT_AVG_BITRATE, cfg.kbps.saturating_mul(1000)).map_err(|e| e.to_string())?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, cfg.profile).map_err(|e| e.to_string())?;
            out.SetUINT32(&MF_MT_MAX_KEYFRAME_SPACING, cfg.keyint).map_err(|e| e.to_string())?;
            self.transform.SetOutputType(0, &out, 0).map_err(|e| format!("{}: does not take this output ({}x{}): {e}", self.name, cfg.width, cfg.height))?;
        }
        let input = api.media_type()?;
        common(&input, &MFVideoFormat_NV12).map_err(|e| e.to_string())?;
        // SAFETY: a plain COM call on live interfaces.
        unsafe { self.transform.SetInputType(0, &input, 0) }.map_err(|e| format!("{}: does not take NV12 input: {e}", self.name))
    }

    /// The next event without waiting (`None` when the queue is empty).
    fn poll_event(&self) -> std::result::Result<Option<Event>, String> {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            match self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
                Ok(ev) => {
                    let ty = ev.GetType().map_err(|e| e.to_string())?;
                    let status = ev.GetStatus().unwrap_or_default();
                    if ty == MEError.0 as u32 || status.is_err() {
                        return Err(format!("{}: the encoder reported an error ({status:?})", self.name));
                    }
                    Ok(Some(match ty {
                        t if t == METransformNeedInput.0 as u32 => Event::NeedInput,
                        t if t == METransformHaveOutput.0 as u32 => Event::HaveOutput,
                        t if t == METransformDrainComplete.0 as u32 => Event::DrainComplete,
                        _ => Event::Other,
                    }))
                }
                Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => Ok(None),
                Err(e) => Err(format!("{}: event queue failed: {e}", self.name)),
            }
        }
    }

    /// The next event, waiting at most [`STALL`].
    fn wait_event(&self) -> std::result::Result<Event, String> {
        let start = Instant::now();
        loop {
            if let Some(e) = self.poll_event()? {
                return Ok(e);
            }
            if start.elapsed() > STALL {
                return Err(format!("{}: the encoder stopped responding", self.name));
            }
            std::thread::sleep(Duration::from_micros(500));
        }
    }

    /// Take one encoded sample (`None` when the transform had nothing after all or changed format).
    fn output(&self, api: &MfApi, size_hint: usize) -> std::result::Result<Option<IMFSample>, String> {
        // SAFETY: plain COM calls on live interfaces.
        let info = unsafe { self.transform.GetOutputStreamInfo(0) }.map_err(|e| format!("{}: no output stream info: {e}", self.name))?;
        let provides = info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
        let own = if provides {
            None
        } else {
            let len = (info.cbSize as usize).max(size_hint).min(256 << 20);
            Some(api.sample_of(&vec![0u8; len], 0)?)
        };
        let mut out = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(own), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
        let mut status = 0u32;
        // SAFETY: one output buffer descriptor (with our sample when the encoder does not provide
        // its own) and a live status out-pointer.
        let r = unsafe { self.transform.ProcessOutput(0, &mut out, &mut status) };
        // SAFETY: the descriptor's fields were initialised above and are taken exactly once.
        let (sample, _events) = unsafe { (ManuallyDrop::take(&mut out[0].pSample), ManuallyDrop::take(&mut out[0].pEvents)) };
        match r {
            Ok(()) => Ok(sample),
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(None),
            Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                // SAFETY: plain COM calls on live interfaces.
                unsafe {
                    let t = self.transform.GetOutputAvailableType(0, 0).map_err(|e| format!("{}: no output type after a change: {e}", self.name))?;
                    self.transform.SetOutputType(0, &t, 0).map_err(|e| format!("{}: refused its own output type: {e}", self.name))?;
                }
                Ok(None)
            }
            Err(e) => Err(format!("{}: ProcessOutput failed: {e}", self.name)),
        }
    }

    /// The parameter sets the output type carries (`MF_MT_MPEG_SEQUENCE_HEADER`), as Annex B.
    fn sequence_header(&self) -> Option<Vec<u8>> {
        // SAFETY: plain COM calls; `GetBlob` writes at most `len` bytes into the buffer of that size.
        unsafe {
            let t = self.transform.GetOutputCurrentType(0).ok()?;
            let len = t.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER).ok()? as usize;
            if len == 0 || len > 1 << 20 {
                return None;
            }
            let mut buf = vec![0u8; len];
            let mut got = 0u32;
            t.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut buf, Some(&mut got)).ok()?;
            buf.truncate(got as usize);
            Some(buf)
        }
    }
}

/// The bytes, presentation time, decode time and key flag of an encoded sample.
fn sample_bytes(sample: &IMFSample) -> std::result::Result<(Vec<u8>, i64, i64, bool), String> {
    // SAFETY: plain COM calls; `Lock` gives `cur` readable bytes valid until `Unlock`, and they are
    // copied out before unlocking.
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer().map_err(|e| format!("encoded sample has no buffer: {e}"))?;
        let (mut ptr, mut cur) = (std::ptr::null_mut::<u8>(), 0u32);
        buffer.Lock(&mut ptr, None, Some(&mut cur)).map_err(|e| format!("cannot read the encoded sample: {e}"))?;
        let data = if ptr.is_null() { Vec::new() } else { std::slice::from_raw_parts(ptr.cast_const(), cur as usize).to_vec() };
        let _ = buffer.Unlock();
        let pts = sample.GetSampleTime().unwrap_or(0);
        let dts = sample.GetUINT64(&MFSampleExtension_DecodeTimestamp).ok().and_then(|d| i64::try_from(d).ok()).unwrap_or(pts);
        let key = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
        Ok((data, pts, dts, key))
    }
}

/// The parameter sets of a stream, once seen.
enum Params {
    H264 { sps: Vec<u8>, pps: Vec<u8> },
    Hevc(HevcConfig),
}

/// The export encoder.
struct MfEncoder {
    t: Transform,
    api: &'static MfApi,
    cfg: Config,
    signal: ColorSignal,
    params: Option<Params>,
    /// Input requests the encoder has made and not yet been given a picture for.
    credits: u32,
    pending: Vec<EncodedPacket>,
    /// The first packet's presentation − decode time, in pictures (an edit list when positive).
    start_delay: Option<i64>,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    nv12: Vec<u8>,
}

impl MfEncoder {
    fn learn_params(&mut self, annex_b: &[u8]) {
        if self.params.is_some() {
            return;
        }
        let (w, h) = (self.cfg.width, self.cfg.height);
        let from = |b: &[u8], codec: Codec| -> Option<Params> {
            match codec {
                Codec::H264 => split_parameter_sets(b).ok().map(|(sps, pps)| Params::H264 { sps, pps }),
                Codec::Hevc => {
                    let (vps, sps, pps) = split_hevc_parameter_sets(b).ok()?;
                    crate::nvenc::hevc::hevc_config(&vps, &sps, &pps, (w, h), 8).ok().map(Params::Hevc)
                }
            }
        };
        self.params = from(annex_b, self.cfg.codec).or_else(|| self.t.sequence_header().and_then(|b| from(&b, self.cfg.codec)));
    }

    /// Take every packet the encoder has ready (one per `METransformHaveOutput`).
    fn take_output(&mut self) -> Result<()> {
        let hint = (self.cfg.width as usize).saturating_mul(self.cfg.height as usize).saturating_mul(2).max(1 << 20);
        let Some(sample) = self.t.output(self.api, hint).map_err(ExportError::Encode)? else { return Ok(()) };
        let (bytes, pts, dts, key) = sample_bytes(&sample).map_err(ExportError::Encode)?;
        if bytes.is_empty() {
            return Ok(());
        }
        self.learn_params(&bytes);
        let data = annex_b_to_length_prefixed_for(self.cfg.codec, &bytes).map_err(|e| ExportError::Encode(format!("{}: {e}", self.t.name)))?;
        let (pf, df) = (frame_of(pts, self.cfg.fps), frame_of(dts, self.cfg.fps));
        self.start_delay.get_or_insert(pf.saturating_sub(df));
        // an IDR / IRAP picture is a sync sample even when the encoder does not flag it
        let key = key || is_irap(self.cfg.codec, &bytes);
        self.pending.push(EncodedPacket { data, key, duration: self.cfg.fps.1, composition_offset: composition_offset(pf, df, self.cfg.fps.1) });
        Ok(())
    }

    fn handle(&mut self, e: Event) -> Result<bool> {
        match e {
            Event::NeedInput => self.credits = self.credits.saturating_add(1),
            Event::HaveOutput => self.take_output()?,
            Event::DrainComplete => return Ok(true),
            Event::Other => {}
        }
        Ok(false)
    }

    fn picture(&mut self, f: &EncoderFrame) -> Result<IMFSample> {
        let (w, h) = (self.cfg.width as usize, self.cfg.height as usize);
        filmcraft_export::timed(filmcraft_export::Stage::Convert, || {
            filmcraft_export::rgba_to_yuv420_8(f.rgba, w, h, &mut self.y, &mut self.u, &mut self.v);
            let (cw, ch) = (w / 2, h / 2);
            self.nv12.clear();
            self.nv12.reserve(w * h + cw * ch * 2);
            self.nv12.extend_from_slice(self.y.get(..w * h).unwrap_or(&[]));
            for i in 0..cw * ch {
                self.nv12.push(self.u.get(i).copied().unwrap_or(128));
                self.nv12.push(self.v.get(i).copied().unwrap_or(128));
            }
        });
        if self.nv12.len() != w * h + (w / 2) * (h / 2) * 2 {
            return Err(ExportError::Encode(format!("{}: picture conversion produced the wrong size", self.t.name)));
        }
        let time = time_of(f.index, self.cfg.fps);
        let sample = self.api.sample_of(&self.nv12, time).map_err(ExportError::Encode)?;
        // SAFETY: a plain COM call on a live sample.
        unsafe { sample.SetSampleDuration(time_of(1, self.cfg.fps)) }.map_err(|e| ExportError::Encode(e.to_string()))?;
        Ok(sample)
    }
}

/// Whether an Annex B access unit holds an IDR (H.264 type 5) or IRAP (HEVC 16..=23) picture.
fn is_irap(codec: Codec, annex_b: &[u8]) -> bool {
    // the start code (0 0 1) is followed by the NAL header
    annex_b.windows(4).any(|w| match w {
        [0, 0, 1, header] => match codec {
            Codec::H264 => header & 0x1f == 5,
            Codec::Hevc => matches!((header >> 1) & 0x3f, 16..=23),
        },
        _ => false,
    })
}

impl VideoEncoder for MfEncoder {
    fn sample_entry(&self) -> SampleEntry {
        let (w, h) = (u16::try_from(self.cfg.width).unwrap_or(u16::MAX), u16::try_from(self.cfg.height).unwrap_or(u16::MAX));
        match (&self.params, self.cfg.codec) {
            (Some(Params::H264 { sps, pps }), _) => SampleEntry::avc(AvcConfig::new(vec![sps.clone()], vec![pps.clone()], 4), w, h),
            (Some(Params::Hevc(c)), _) => {
                let mut e = SampleEntry::hevc(c.clone(), w, h);
                self.signal.apply_to(&mut e, false);
                e
            }
            (None, Codec::H264) => SampleEntry::avc(AvcConfig::new(Vec::new(), Vec::new(), 4), w, h),
            (None, Codec::Hevc) => SampleEntry::hevc(HevcConfig::default(), w, h),
        }
    }

    fn timescale(&self) -> u32 {
        self.cfg.fps.0
    }

    fn encode(&mut self, f: &EncoderFrame) -> Result<Vec<EncodedPacket>> {
        if f.width != self.cfg.width || f.height != self.cfg.height {
            return Err(ExportError::Encode(format!(
                "{}: a {}x{} picture for a {}x{} encoder",
                self.t.name, f.width, f.height, self.cfg.width, self.cfg.height
            )));
        }
        if f.hdr.is_some() {
            return Err(ExportError::Unsupported(format!("{} writes 8-bit SDR only", self.t.name)));
        }
        ensure_com().map_err(ExportError::Encode)?;
        let sample = self.picture(f)?;
        while self.credits == 0 {
            let e = self.t.wait_event().map_err(ExportError::Encode)?;
            self.handle(e)?;
        }
        // SAFETY: a plain COM call on live interfaces.
        unsafe { self.t.transform.ProcessInput(0, &sample, 0) }.map_err(|e| ExportError::Encode(format!("{}: ProcessInput failed: {e}", self.t.name)))?;
        self.credits = self.credits.saturating_sub(1);
        // collect whatever is ready without waiting
        while let Some(e) = self.t.poll_event().map_err(ExportError::Encode)? {
            self.handle(e)?;
        }
        filmcraft_export::note_hw_encode_frame();
        Ok(std::mem::take(&mut self.pending))
    }

    fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
        ensure_com().map_err(ExportError::Encode)?;
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            self.t.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0).map_err(|e| ExportError::Encode(format!("{}: {e}", self.t.name)))?;
            self.t.transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0).map_err(|e| ExportError::Encode(format!("{}: drain failed: {e}", self.t.name)))?;
        }
        loop {
            let e = self.t.wait_event().map_err(ExportError::Encode)?;
            if self.handle(e)? {
                break;
            }
        }
        Ok(std::mem::take(&mut self.pending))
    }

    fn media_start(&self) -> Option<i64> {
        self.start_delay.filter(|d| *d > 0).map(|d| d.saturating_mul(i64::from(self.cfg.fps.1)))
    }
}

/// Open the first hardware encoder that takes `cfg`.
fn open(cfg: &Config) -> std::result::Result<(Transform, &'static MfApi), String> {
    ensure_com()?;
    let api = api()?;
    let subtype = match cfg.codec {
        Codec::H264 => MFVideoFormat_H264,
        Codec::Hevc => MFVideoFormat_HEVC,
    };
    let mut list = api.hardware_encoders(subtype)?;
    if list.is_empty() {
        return Err("no GPU video encoder found".into());
    }
    let prefer = std::env::var(PREFER_ENV).ok();
    list.sort_by_key(|a| rank(&friendly_name(a).unwrap_or_default(), prefer.as_deref()));
    let mut why = Vec::new();
    for a in list {
        match Transform::open(api, a, cfg) {
            Ok(t) => {
                log::info!("hardware encoding with {}", t.name);
                return Ok((t, api));
            }
            Err(e) => why.push(e),
        }
    }
    Err(why.join("; "))
}

fn new_encoder(t: Transform, api: &'static MfApi, cfg: Config, signal: ColorSignal) -> MfEncoder {
    MfEncoder {
        t,
        api,
        cfg,
        signal,
        params: None,
        credits: 0,
        pending: Vec::new(),
        start_delay: None,
        y: Vec::new(),
        u: Vec::new(),
        v: Vec::new(),
        nv12: Vec::new(),
    }
}

static HEVC_PROBE: OnceLock<bool> = OnceLock::new();

/// Whether a GPU encoder here writes H.265 (asked once by opening a small session).
pub fn hevc_available() -> bool {
    *HEVC_PROBE.get_or_init(|| {
        let cfg = Config { codec: Codec::Hevc, width: 640, height: 360, fps: (30, 1), kbps: 2000, max_kbps: 3000, cbr: false, keyint: 60, profile: 1 };
        match open(&cfg) {
            Ok((t, _)) => {
                log::info!("H.265 export through {}", t.name);
                true
            }
            Err(why) => {
                log::info!("no Media Foundation H.265 encoder: {why}");
                false
            }
        }
    })
}

/// Answer [`hevc_available`] on a thread of its own, ahead of the first draw of the format list.
pub fn warm_hevc_probe() {
    let _ = std::thread::Builder::new().name("mf-hevc-probe".into()).spawn(|| {
        let _ = std::panic::catch_unwind(hevc_available);
    });
}

/// The Export encoder factory (see the module documentation). Registered behind NVENC.
pub fn factory(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Option<Result<Box<dyn VideoEncoder>>> {
    let hevc = match format {
        Format::H264 if s.hardware_encoding == HardwareEncoding::Auto => false,
        Format::Hevc => true,
        _ => return None,
    };
    let declined = |why: String| {
        log::info!("GPU encoder declined: {why}");
        filmcraft_export::note_hw_encode_declined();
        hevc.then(|| Err(ExportError::Unsupported(format!("H.265 export with the GPU encoder: {why}"))))
    };
    let cfg = match config(format, w, h, rate, s) {
        Ok(c) => c,
        Err(why) => return declined(why),
    };
    // no HEVC encoder at all: the export's own "encoder not available" error
    if hevc && !hevc_available() {
        return None;
    }
    match open(&cfg) {
        Ok((t, api)) => {
            filmcraft_export::note_hw_encode_session();
            Some(Ok(Box::new(new_encoder(t, api, cfg, s.signal))))
        }
        Err(why) => declined(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip_at_ntsc_rates() {
        for fps in [(24, 1), (30000, 1001), (60000, 1001), (25, 1), (60, 1)] {
            for i in [0u64, 1, 2, 999, 123_456] {
                assert_eq!(frame_of(time_of(i, fps), fps), i as i64, "{fps:?} {i}");
            }
        }
        assert_eq!(time_of(u64::MAX, (1, 1)), i64::MAX);
        assert_eq!(time_of(5, (0, 1)), 50_000_000);
        assert_eq!(composition_offset(3, 1, 1001), 2002);
        assert_eq!(composition_offset(i64::MAX, i64::MIN, 1001), i32::MAX);
    }

    #[test]
    fn config_takes_sdr_mp4_and_declines_the_rest() {
        let ok = ExportSettings { format: Format::Hevc, bitrate_kbps: 60_000, ..Default::default() };
        let c = config(Format::Hevc, 3840, 2160, FrameRate::FPS_24, &ok).unwrap();
        assert_eq!((c.codec, c.profile, c.keyint, c.max_kbps), (Codec::Hevc, 1, 48, 90_000));
        let h = ExportSettings { format: Format::H264, h264_profile: H264Profile::High, ..Default::default() };
        assert_eq!(config(Format::H264, 1920, 1080, FrameRate::FPS_24, &h).unwrap().profile, 100);
        for (what, s) in [
            ("HDR", ExportSettings { signal: ColorSignal::PQ, ..ok.clone() }),
            ("two-pass", ExportSettings { bitrate_mode: BitrateMode::Vbr2Pass, ..ok.clone() }),
            ("CRF", ExportSettings { bitrate_mode: BitrateMode::Crf, ..ok.clone() }),
            ("interlaced", ExportSettings { field_order: FieldOrder::UpperFirst, ..ok.clone() }),
            ("non-square", ExportSettings { pixel_aspect: Some((4, 3)), ..ok.clone() }),
            ("MXF", ExportSettings { format: Format::MxfOp1a, ..ok.clone() }),
        ] {
            assert!(config(Format::Hevc, 1280, 720, FrameRate::FPS_24, &s).is_err(), "{what}");
        }
        assert!(config(Format::ProRes, 1280, 720, FrameRate::FPS_24, &ok).is_err());
        assert!(config(Format::Hevc, 1281, 720, FrameRate::FPS_24, &ok).is_err());
        assert!(config(Format::Hevc, 9000, 720, FrameRate::FPS_24, &ok).is_err());
        for rate in [FrameRate { num: 0, den: 1 }, FrameRate { num: 24, den: 0 }, FrameRate { num: -24, den: 1 }, FrameRate { num: i64::MAX, den: 1 }] {
            assert!(config(Format::Hevc, 1280, 720, rate, &ok).is_err(), "{rate:?}");
        }
    }

    #[test]
    fn amd_and_intel_are_tried_before_others() {
        assert!(rank("AMDh264Encoder", None) < rank("Intel® Quick Sync Video H.264 Encoder MFT", None));
        assert!(rank("Intel® Quick Sync Video H.264 Encoder MFT", None) < rank("NVIDIA H.264 Encoder MFT", None));
        assert_eq!(rank("Intel® Quick Sync Video H.264 Encoder MFT", Some("intel")), 0);
        assert_eq!(rank("AMDh264Encoder", Some("")), 1);
    }

    #[test]
    fn irap_detection() {
        assert!(is_irap(Codec::H264, &[0, 0, 0, 1, 0x67, 1, 0, 0, 1, 0x65, 9]));
        assert!(!is_irap(Codec::H264, &[0, 0, 1, 0x41, 9]));
        assert!(is_irap(Codec::Hevc, &[0, 0, 1, 19 << 1, 1, 9]));
        assert!(!is_irap(Codec::Hevc, &[0, 0, 1, 1 << 1, 1, 9]));
        assert!(!is_irap(Codec::Hevc, &[]));
    }
}
