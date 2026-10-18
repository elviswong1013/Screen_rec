// pipefault — S1b spike: named-pipe protocol fault injection (spike tooling, NOT production code).
//
// Replicates the S1 pipebench writer pipeline (paced 30fps synthetic BGRA -> 1080p canvas -> NV12 ->
// named pipe -> ffmpeg rawvideo/h264_qsv -> 5s MKV segments) and injects faults:
//   slowread         - ffmpeg reads slowly (-readrate 0.25) -> writer backpressure
//   suspend-resume   - ffmpeg reader threads suspended mid-run -> hard stall -> resume -> catch-up
//   suspend-cancel   - reader suspended, blocked WriteFile cancelled via CancelIoEx -> teardown
//   kill             - taskkill /F ffmpeg while reader active -> WriteFile error + teardown
//   kill-stalled     - taskkill /F ffmpeg while writer blocked (reader suspended first)
//   connect-timeout  - no client connects -> blocking ConnectNamedPipe + CancelIoEx + reuse test
//   eof              - writer closes pipe early (EOF) -> ffmpeg finalize behaviour
//   restart          - repeated start/stop loop (default 12x) -> handle/thread/memory leak check
//   bufsz            - plain paced run at a given --pipe-buf (8MiB vs 16MiB comparison)
//
// Per-run JSON + ffmpeg stderr land in docs/spikes/s1b/raw/; encode outputs go to a temp dir.

use std::f64::consts::TAU;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, WriteFile, PIPE_ACCESS_OUTBOUND};use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessHandleCount, OpenThread, ResumeThread, SuspendThread,
    THREAD_SUSPEND_RESUME,
};

const CANVAS_W: usize = 1920;
const CANVAS_H: usize = 1080;
const NV12_FRAME_BYTES: usize = CANVAS_W * CANVAS_H * 3 / 2;
const CREATE_NO_WINDOW: u32 = 0x08000000;
const FF_DIR: &str = r"C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\tools\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin";
const RAW_DIR: &str = r"C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\spikes\s1b\raw";

// ---------- args ----------
#[derive(Clone)]
struct Args {
    case: String,
    seconds: u64, // writer target frames = seconds * fps
    fps: f64,
    pipe_buf: u32,
    label: String,
    out_root: PathBuf,
    ff: PathBuf,
    raw_dir: PathBuf,
    // fault params
    readrate: f64,                // slowread: ffmpeg -readrate factor (0 = off)
    readrate_catchup: f64,        // slowread: ffmpeg -readrate_catchup (0 = off)
    kill_at: f64,                 // kill / kill-stalled: seconds from watcher start
    suspend_before_kill: f64,     // kill-stalled: suspend ffmpeg this long before kill
    suspend_at: f64,              // suspend-*: seconds from watcher start
    resume_after: f64,            // suspend-resume: suspended duration
    cancel_write_at: f64,         // suspend-cancel: cancel blocked WriteFile at this time
    cancel_connect_after_ms: u64, // connect-timeout: CancelIoEx on ConnectNamedPipe after this
    restart_iters: u32,           // restart loop count
}

fn parse_args() -> Args {
    let mut a = Args {
        case: String::new(),
        seconds: 10,
        fps: 30.0,
        pipe_buf: 4 * 1024 * 1024,
        label: "run".to_string(),
        out_root: std::env::temp_dir().join("opencode").join("s1bout"),
        ff: PathBuf::from(FF_DIR).join("ffmpeg.exe"),
        raw_dir: PathBuf::from(RAW_DIR),
        readrate: 0.0,
        readrate_catchup: 0.0,
        kill_at: 4.5,
        suspend_before_kill: 0.0,
        suspend_at: 4.0,
        resume_after: 3.0,
        cancel_write_at: 7.0,
        cancel_connect_after_ms: 5000,
        restart_iters: 12,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--case" => a.case = it.next().unwrap_or_default(),
            "--seconds" => a.seconds = it.next().and_then(|s| s.parse().ok()).unwrap_or(10),
            "--fps" => a.fps = it.next().and_then(|s| s.parse().ok()).unwrap_or(30.0),
            "--pipe-buf" => a.pipe_buf = it.next().and_then(|s| s.parse().ok()).unwrap_or(4 * 1024 * 1024),
            "--label" => a.label = it.next().unwrap_or_else(|| "run".into()),
            "--out-root" => a.out_root = it.next().map(PathBuf::from).unwrap_or_else(|| a.out_root.clone()),
            "--ff" => a.ff = it.next().map(PathBuf::from).unwrap_or_else(|| a.ff.clone()),
            "--raw-dir" => a.raw_dir = it.next().map(PathBuf::from).unwrap_or_else(|| a.raw_dir.clone()),
            "--readrate" => a.readrate = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            "--readrate-catchup" => a.readrate_catchup = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            "--kill-at" => a.kill_at = it.next().and_then(|s| s.parse().ok()).unwrap_or(4.5),
            "--suspend-before-kill" => a.suspend_before_kill = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            "--suspend-at" => a.suspend_at = it.next().and_then(|s| s.parse().ok()).unwrap_or(4.0),
            "--resume-after" => a.resume_after = it.next().and_then(|s| s.parse().ok()).unwrap_or(3.0),
            "--cancel-write-at" => a.cancel_write_at = it.next().and_then(|s| s.parse().ok()).unwrap_or(7.0),
            "--cancel-connect-after-ms" => a.cancel_connect_after_ms = it.next().and_then(|s| s.parse().ok()).unwrap_or(5000),
            "--restart-iters" => a.restart_iters = it.next().and_then(|s| s.parse().ok()).unwrap_or(12),
            other => panic!("unknown arg {other}"),
        }
    }
    if a.case.is_empty() { panic!("--case required"); }
    a
}

// ---------- synthetic frame generation (test-only cost, copied from pipebench) ----------
fn gen_frame_bgra(buf: &mut [u8], w: usize, h: usize, t: u64) {
    let phase = (t as f64) * 0.15;
    for y in 0..h {
        let r_base: i32 = (127.0 + 60.0 * ((y as f64) * TAU / h as f64 + phase).sin()) as i32;
        let g_base: i32 = (127.0 + 60.0 * ((y as f64) * 0.01 + phase * 0.7).sin()) as i32;
        for x in 0..w {
            let sweep: i32 = ((x + t as usize * 13) % w) as i32;
            let b = 16 + (g_base / 2 + sweep / 3).rem_euclid(224);
            let g = 16 + (g_base).rem_euclid(224);
            let r = 16 + (r_base).rem_euclid(224);
            let i = (y * w + x) * 4;
            buf[i] = b as u8;
            buf[i + 1] = g as u8;
            buf[i + 2] = r as u8;
            buf[i + 3] = 255;
        }
    }
}

fn bgra_to_nv12(bgra: &[u8], w: usize, h: usize, nv12: &mut [u8]) {
    let (y_plane, uv_plane) = nv12.split_at_mut(w * h);
    let uv_w = w / 2;
    for y in (0..h).step_by(2) {
        let r0 = y * w;
        let r1 = r0 + w;
        for x in (0..w).step_by(2) {
            let i00 = (r0 + x) * 4;
            let i01 = i00 + 4;
            let i10 = (r1 + x) * 4;
            let i11 = i10 + 4;
            let (b0, g0, r0c) = (bgra[i00] as i32, bgra[i00 + 1] as i32, bgra[i00 + 2] as i32);
            let (b1, g1, r1c) = (bgra[i01] as i32, bgra[i01 + 1] as i32, bgra[i01 + 2] as i32);
            let (b2, g2, r2c) = (bgra[i10] as i32, bgra[i10 + 1] as i32, bgra[i10 + 2] as i32);
            let (b3, g3, r3c) = (bgra[i11] as i32, bgra[i11 + 1] as i32, bgra[i11 + 2] as i32);
            y_plane[r0 + x] = clamp_y((77 * r0c + 150 * g0 + 29 * b0) >> 8);
            y_plane[r0 + x + 1] = clamp_y((77 * r1c + 150 * g1 + 29 * b1) >> 8);
            y_plane[r1 + x] = clamp_y((77 * r2c + 150 * g2 + 29 * b2) >> 8);
            y_plane[r1 + x + 1] = clamp_y((77 * r3c + 150 * g3 + 29 * b3) >> 8);
            let bavg = (b0 + b1 + b2 + b3) >> 2;
            let gavg = (g0 + g1 + g2 + g3) >> 2;
            let ravg = (r0c + r1c + r2c + r3c) >> 2;
            let u_idx = (y / 2) * uv_w + x / 2;
            uv_plane[u_idx] = clamp_uv(((-43 * ravg - 85 * gavg + 128 * bavg) >> 8) + 128);
            uv_plane[uv_w * (h / 2) + u_idx] = clamp_uv(((128 * ravg - 107 * gavg - 21 * bavg) >> 8) + 128);
        }
    }
}
#[inline] fn clamp_y(v: i32) -> u8 { v.clamp(16, 235) as u8 }
#[inline] fn clamp_uv(v: i32) -> u8 { v.clamp(16, 240) as u8 }

// ---------- per-route frame prep (scale variant, same as pipebench) ----------
struct Prep {
    src_buf: Vec<u8>,
    canvas: Vec<u8>,
    nv12: Vec<u8>,
    resizer: fast_image_resize::Resizer,
    fitted_img: fast_image_resize::images::Image<'static>,
    src_w: usize,
    src_h: usize,
    fit_w: usize,
    fit_h: usize,
    off_x: usize,
    off_y: usize,
}

impl Prep {
    fn new() -> Prep {
        let (src_w, src_h) = (1600usize, 900usize); // scale variant (matches S1 paced runs)
        let s = ((CANVAS_W as f64 / src_w as f64).min(CANVAS_H as f64 / src_h as f64) * 2.0).floor() / 2.0;
        let fit_w = ((src_w as f64 * s) as usize) & !1;
        let fit_h = ((src_h as f64 * s) as usize) & !1;
        let mut canvas = vec![0u8; CANVAS_W * CANVAS_H * 4];
        for px in canvas.chunks_exact_mut(4) { px.copy_from_slice(&[16, 16, 16, 255]); } // studio black bars
        let mut nv12 = vec![0u8; NV12_FRAME_BYTES];
        nv12[..CANVAS_W * CANVAS_H].fill(16);
        nv12[CANVAS_W * CANVAS_H..].fill(128);
        let fitted_img = fast_image_resize::images::Image::new(fit_w as u32, fit_h as u32, fast_image_resize::PixelType::U8x4);
        Prep {
            src_buf: vec![0u8; src_w * src_h * 4],
            canvas, nv12,
            resizer: fast_image_resize::Resizer::new(),
            fitted_img,
            src_w, src_h, fit_w, fit_h,
            off_x: (CANVAS_W - fit_w) / 2,
            off_y: (CANVAS_H - fit_h) / 2,
        }
    }

    fn render(&mut self, t: u64) {
        gen_frame_bgra(&mut self.src_buf, self.src_w, self.src_h, t);
        let vec = std::mem::take(&mut self.src_buf);
        let src_img = fast_image_resize::images::Image::from_vec_u8(self.src_w as u32, self.src_h as u32, vec, fast_image_resize::PixelType::U8x4).expect("src image");
        let opts = fast_image_resize::ResizeOptions::new()
            .resize_alg(fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Bilinear));
        self.resizer.resize(&src_img, &mut self.fitted_img, &opts).expect("resize");
        self.src_buf = src_img.into_vec();
        if self.fit_w != CANVAS_W || self.fit_h != CANVAS_H {
            let fit_row = self.fit_w * 4;
            let canvas_row = CANVAS_W * 4;
            let fb = self.fitted_img.buffer();
            for row in 0..self.fit_h {
                let d = (self.off_y + row) * canvas_row + self.off_x * 4;
                let sr = row * fit_row;
                self.canvas[d..d + fit_row].copy_from_slice(&fb[sr..sr + fit_row]);
            }
        }
        bgra_to_nv12(&self.canvas, CANVAS_W, CANVAS_H, &mut self.nv12);
    }
}

// ---------- stage stats ----------
type Stat = (f64, f64, f64, f64); // avg, p50, p95, max (ms)

fn summary(v: &[f64], skip: usize) -> Stat {
    let mut s: Vec<f64> = if skip > 0 && v.len() > skip { v[skip..].to_vec() } else { v.to_vec() };
    if s.is_empty() { return (0.0, 0.0, 0.0, 0.0); }
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    (s.iter().sum::<f64>() / n as f64, s[n / 2], s[n * 95 / 100], s[n - 1])
}

// ---------- named pipe ----------
struct PipeServer { handle: HANDLE }

impl PipeServer {
    fn create(name_wide: &[u16], buf: u32) -> windows::core::Result<PipeServer> {
        unsafe {
            let h = CreateNamedPipeW(
                PCWSTR(name_wide.as_ptr()),
                PIPE_ACCESS_OUTBOUND,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                buf,
                buf,
                0,
                None,
            );
            Ok(PipeServer { handle: h })
        }
    }
    // blocking ConnectNamedPipe; Err(os_code) on failure (incl. cancellation)
    fn connect(&self) -> Result<(), i32> {
        unsafe {
            match ConnectNamedPipe(self.handle, None) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(()),
                Err(e) => Err(e.code().0),
            }
        }
    }
    fn write_all(&self, data: &[u8]) -> std::io::Result<()> {
        let mut off = 0usize;
        while off < data.len() {
            let mut written: u32 = 0;
            let res = unsafe { WriteFile(self.handle, Some(&data[off..]), Some(&mut written), None) };
            if res.is_err() || written == 0 {
                return Err(std::io::Error::last_os_error());
            }
            off += written as usize;
        }
        Ok(())
    }
    fn disconnect(&self) -> windows::core::Result<()> {
        unsafe { DisconnectNamedPipe(self.handle) }
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        unsafe {
            let _ = FlushFileBuffers(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

fn h_as_isize(h: HANDLE) -> isize { h.0 as isize }
fn h_from_isize(v: isize) -> HANDLE { HANDLE(v as *mut core::ffi::c_void) }

fn to_wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

fn err_name(code: i32) -> String {
    match code as u32 {
        109 => "ERROR_BROKEN_PIPE".into(),
        232 => "ERROR_NO_DATA".into(),
        995 => "ERROR_OPERATION_ABORTED".into(),
        6 => "ERROR_INVALID_HANDLE".into(),
        231 => "ERROR_PIPE_BUSY".into(),
        535 => "ERROR_PIPE_CONNECTED".into(),
        other => format!("OS_{other}"),
    }
}

// ---------- ffprobe ----------
fn ffprobe(ffprobe_exe: &Path, path: &Path) -> (Option<f64>, Option<u64>, Option<String>) {
    let out = Command::new(ffprobe_exe)
        .args(["-v", "error", "-show_entries", "format=duration", "-count_packets", "-select_streams", "v:0",
               "-show_entries", "stream=nb_read_packets,codec_name", "-of", "json"])
        .arg(path)
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            let dur = s.split("\"duration\": \"").nth(1).and_then(|x| x.split('"').next()).and_then(|x| x.parse::<f64>().ok());
            let packets = s.split("\"nb_read_packets\": \"").nth(1).and_then(|x| x.split('"').next()).and_then(|x| x.parse::<u64>().ok());
            let codec = s.split("\"codec_name\": \"").nth(1).and_then(|x| x.split('"').next().map(|x| x.to_string()));
            (dur, packets, codec)
        }
        Err(_) => (None, None, None),
    }
}

#[derive(Clone)]
struct SegInfo { path: String, duration: Option<f64>, packets: Option<u64>, codec: Option<String> }

fn probe_segments(ff: &Path, out_dir: &Path) -> Vec<SegInfo> {
    let ffprobe_exe = ff.parent().unwrap().join("ffprobe.exe");
    let mut segs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(out_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "mkv").unwrap_or(false) {
                let (duration, packets, codec) = ffprobe(&ffprobe_exe, &p);
                segs.push(SegInfo { path: p.to_string_lossy().to_string(), duration, packets, codec });
            }
        }
    }
    segs.sort_by(|a, b| a.path.cmp(&b.path));
    segs
}

// ---------- process metrics / control ----------
fn private_usage_bytes() -> u64 {
    unsafe {
        let mut pmc = PROCESS_MEMORY_COUNTERS_EX::default();
        let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut _, cb).is_ok() {
            pmc.PrivateUsage as u64
        } else { 0 }
    }
}

fn handle_count_self() -> u32 {
    unsafe {
        let mut n: u32 = 0;
        let _ = GetProcessHandleCount(GetCurrentProcess(), &mut n);
        n
    }
}

fn thread_ids_of(pid: u32) -> Vec<u32> {
    unsafe {
        let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) { Ok(h) => h, Err(_) => return Vec::new() };
        let mut e = THREADENTRY32::default();
        e.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut v = Vec::new();
        if Thread32First(snap, &mut e).is_ok() {
            loop {
                if e.th32OwnerProcessID == pid { v.push(e.th32ThreadID); }
                if Thread32Next(snap, &mut e).is_err() { break; }
            }
        }
        let _ = CloseHandle(snap);
        v
    }
}

fn thread_count_self() -> u32 { thread_ids_of(std::process::id()).len() as u32 }

// suspend all threads of pid; returns raw handles (as isize) to resume exactly what we suspended
fn suspend_threads_of(pid: u32) -> Vec<isize> {
    thread_ids_of(pid).into_iter().filter_map(|tid| unsafe {
        let h = match OpenThread(THREAD_SUSPEND_RESUME, false, tid) { Ok(h) => h, Err(_) => return None };
        if SuspendThread(h) == 0xFFFFFFFFu32 {
            let _ = CloseHandle(h);
            None
        } else {
            Some(h_as_isize(h))
        }
    }).collect()
}

fn resume_threads(handles: &[isize]) -> usize {
    handles.iter().filter(|&&h| unsafe {
        let _ = ResumeThread(h_from_isize(h));
        let _ = CloseHandle(h_from_isize(h));
        true
    }).count()
}

fn ffmpeg_proc_count() -> u32 {
    let out = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq ffmpeg.exe", "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.to_lowercase().starts_with("\"ffmpeg.exe\""))
            .count() as u32,
        Err(_) => 0,
    }
}

// ---------- sampler ----------
type SampleLog = Arc<Mutex<Vec<(f64, u64)>>>;

fn start_sampler(stop: Arc<AtomicBool>, log: SampleLog) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let t0 = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            log.lock().unwrap().push((t0.elapsed().as_secs_f64(), private_usage_bytes()));
            std::thread::sleep(Duration::from_millis(250));
        }
    })
}

// ---------- ffmpeg spawn ----------
fn spawn_ffmpeg_reader(a: &Args, out_dir: &Path, pipe_name: &str, input_opts: &[String]) -> (Child, String) {
    let seg_pattern = out_dir.join("out_%05d.mkv");
    let stderr_log = std::fs::File::create(out_dir.join("ffmpeg_stderr.log")).unwrap();
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into()];
    args.extend(input_opts.iter().cloned());
    args.extend([
        "-f".into(), "rawvideo".into(),
        "-pix_fmt".into(), "nv12".into(),
        "-s".into(), format!("{}x{}", CANVAS_W, CANVAS_H),
        "-framerate".into(), format!("{}", a.fps),
        "-i".into(), pipe_name.into(),
        "-c:v".into(), "h264_qsv".into(), "-g".into(), "60".into(),
        "-f".into(), "segment".into(), "-segment_time".into(), "5".into(),
        "-reset_timestamps".into(), "1".into(), "-segment_format".into(), "matroska".into(),
    ]);
    let cmd = format!("ffmpeg {} -y {}", args.join(" "), seg_pattern.display());
    let child = Command::new(&a.ff)
        .args(&args)
        .arg(&seg_pattern)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_log))
        .spawn()
        .expect("spawn ffmpeg failed");
    (child, cmd)
}

// ---------- paced write loop ----------
struct LoopOut {
    frames_written: u64,
    write_wait: Vec<f64>,
    late: Vec<f64>,
    error: Option<(u64, i32)>, // frame index, os error code
    blocked_first_frame: Option<u64>, // first frame with write_wait > 500ms
}

fn write_paced(prep: &mut Prep, pipe: &PipeServer, t0: Instant, n: u64, fps: f64) -> LoopOut {
    let mut out = LoopOut {
        frames_written: 0, write_wait: Vec::new(), late: Vec::new(),
        error: None, blocked_first_frame: None,
    };
    for i in 0..n {
        let target = t0 + Duration::from_secs_f64(i as f64 / fps);
        let now = Instant::now();
        if now < target { std::thread::sleep(target - now); }
        prep.render(i);
        let s = Instant::now();
        if let Err(e) = pipe.write_all(&prep.nv12) {
            out.error = Some((i, e.raw_os_error().unwrap_or(-1)));
            return out;
        }
        let d = s.elapsed().as_secs_f64() * 1000.0;
        if out.blocked_first_frame.is_none() && d > 500.0 { out.blocked_first_frame = Some(i); }
        out.write_wait.push(d);
        out.frames_written = i + 1;
        let deadline = t0 + Duration::from_secs_f64((i + 1) as f64 / fps);
        out.late.push((Instant::now() - deadline).as_secs_f64() * 1000.0);
    }
    out
}

// ---------- wait for child ----------
fn reap(child: &mut Child, timeout: Duration) -> Option<i32> {
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(code)) => return Some(code.code().unwrap_or(-1)),
            Ok(None) => {
                if t0.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None; // had to force-kill
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

fn late_stats(late: &[f64]) -> (f64, u64) {
    let m = late.iter().cloned().fold(f64::MIN, f64::max).max(0.0);
    let c = late.iter().filter(|x| **x > 0.0).count() as u64;
    (m, c)
}

fn late_stats_steady(late: &[f64], skip: usize) -> (f64, u64) {
    if late.len() > skip { late_stats(&late[skip..]) } else { (0.0, 0) }
}

// ---------- JSON helpers ----------
struct J { f: Vec<String> }
impl J {
    fn new() -> J { J { f: Vec::new() } }
    fn raw(&mut self, k: &str, v: String) { self.f.push(format!("\"{k}\": {v}")); }
    fn num<T: std::fmt::Display>(&mut self, k: &str, v: T) { self.raw(k, format!("{v}")); }
    fn f1(&mut self, k: &str, v: f64) { self.raw(k, format!("{v:.1}")); }
    fn f3(&mut self, k: &str, v: f64) { self.raw(k, format!("{v:.3}")); }
    fn s(&mut self, k: &str, v: &str) { self.raw(k, format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))); }
    fn opt(&mut self, k: &str, v: Option<impl std::fmt::Display>) {
        self.raw(k, v.map(|x| format!("{x}")).unwrap_or_else(|| "null".into()));
    }
    fn stat(&mut self, k: &str, t: Stat) { self.raw(k, format!("{{\"avg\":{:.3},\"p50\":{:.3},\"p95\":{:.3},\"max\":{:.3}}}", t.0, t.1, t.2, t.3)); }
    fn segs(&mut self, k: &str, segs: &[SegInfo]) {
        let arr: Vec<String> = segs.iter().map(|s| format!("{{\"path\":\"{}\",\"duration\":{},\"packets\":{},\"codec\":{}}}",
            s.path.replace('\\', "\\\\"),
            s.duration.map(|x| format!("{x:.3}")).unwrap_or_else(|| "null".into()),
            s.packets.map(|x| x.to_string()).unwrap_or_else(|| "null".into()),
            s.codec.as_deref().map(|c| format!("\"{c}\"")).unwrap_or_else(|| "null".into()))).collect();
        self.raw(k, format!("[{}]", arr.join(",")));
    }
    fn samples(&mut self, k: &str, v: &[(f64, u64)]) {
        let arr: Vec<String> = v.iter().map(|(t, b)| format!("[{t:.2},{b}]")).collect();
        self.raw(k, format!("[{}]", arr.join(",")));
    }
    fn write(self, raw_dir: &Path, label: &str) {
        std::fs::create_dir_all(raw_dir).unwrap();
        let json = format!("{{\n  {}\n}}\n", self.f.join(",\n  "));
        let p = raw_dir.join(format!("{label}.json"));
        std::fs::write(&p, &json).unwrap();
        println!("JSON: {}", p.display());
    }
}

fn copy_stderr(out_dir: &Path, raw_dir: &Path, name: &str) {
    let src = out_dir.join("ffmpeg_stderr.log");
    if src.exists() {
        let _ = std::fs::copy(&src, raw_dir.join(format!("{name}_ffmpeg_stderr.log")));
    }
}

// ================= cases =================

// full paced run, no injected kill/cancel (slowread / bufsz)
fn case_paced_like(a: &Args, readrate: f64) -> J {
    let n_frames = (a.seconds as f64 * a.fps).round() as u64;
    let out_dir = a.out_root.join(&a.label);
    std::fs::create_dir_all(&out_dir).unwrap();
    let pipe_name = format!(r"\\.\pipe\screenrec_s1b_{}", a.label);
    let name_wide = to_wide(&pipe_name);

    let t0 = Instant::now();
    let pipe = PipeServer::create(&name_wide, a.pipe_buf).expect("CreateNamedPipeW failed");
    let t_pipe_created = t0.elapsed();

    let mut input_opts: Vec<String> = Vec::new();
    if readrate > 0.0 {
        input_opts.push("-readrate".into());
        input_opts.push(format!("{readrate}"));
    }
    if a.readrate_catchup > 0.0 {
        input_opts.push("-readrate_catchup".into());
        input_opts.push(format!("{}", a.readrate_catchup));
    }
    let (mut child, cmd) = spawn_ffmpeg_reader(a, &out_dir, &pipe_name, &input_opts);
    pipe.connect().expect("ffmpeg did not connect to pipe");
    let connect_ms = (t0.elapsed() - t_pipe_created).as_secs_f64() * 1000.0;

    let stop = Arc::new(AtomicBool::new(false));
    let samples: SampleLog = Arc::new(Mutex::new(Vec::new()));
    let sh = start_sampler(stop.clone(), samples.clone());

    let mut prep = Prep::new();
    let t_write_start = Instant::now();
    let lp = write_paced(&mut prep, &pipe, t_write_start, n_frames, a.fps);
    let write_wall_s = t_write_start.elapsed().as_secs_f64();

    drop(pipe); // EOF
    let exit = reap(&mut child, Duration::from_secs(120));
    let wall_s = t0.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    let _ = sh.join();
    copy_stderr(&out_dir, &a.raw_dir, &a.label);

    let segs = probe_segments(&a.ff, &out_dir);
    let packets_sum: u64 = segs.iter().map(|s| s.packets.unwrap_or(0)).sum();
    let (late_max, late_over) = late_stats(&lp.late);
    let (late_max_st, late_over_st) = late_stats_steady(&lp.late, 30);
    let ww = summary(&lp.write_wait, 0);
    let ww_st = summary(&lp.write_wait, 30);
    let sam = samples.lock().unwrap().clone();
    let mem_max = sam.iter().map(|x| x.1).max().unwrap_or(0);
    let mem_min = sam.iter().map(|x| x.1).min().unwrap_or(0);

    let mut j = J::new();
    j.s("case", &a.case); j.s("label", &a.label);
    j.s("ffmpeg_cmd", &cmd);
    j.num("pipe_buf", a.pipe_buf); j.num("fps", a.fps); j.num("readrate", readrate);
    j.num("readrate_catchup", a.readrate_catchup);
    j.num("n_frames_target", n_frames); j.num("frames_written", lp.frames_written);
    j.f3("connect_ms", connect_ms);
    j.f3("write_wall_s", write_wall_s);
    j.f3("wall_s", wall_s);
    match lp.error {
        Some((i, code)) => j.raw("write_error", format!("{{\"frame\":{i},\"code\":{code},\"name\":\"{}\"}}", err_name(code))),
        None => j.raw("write_error", "null".into()),
    }
    j.raw("blocked_first_frame", lp.blocked_first_frame.map(|x| format!("{x}")).unwrap_or_else(|| "null".into()));
    j.f1("late_max_ms", late_max); j.num("late_over_budget_frames", late_over);
    j.f1("late_steady_max_ms", late_max_st); j.num("late_over_budget_steady", late_over_st);
    j.stat("write_wait_ms", ww); j.stat("write_wait_steady_ms", ww_st);
    j.num("pipe_bytes", lp.frames_written * NV12_FRAME_BYTES as u64);
    j.num("packets_sum", packets_sum);
    j.opt("ffmpeg_exit", exit);
    j.num("mem_private_min_b", mem_min);
    j.num("mem_private_max_b", mem_max);
    j.samples("mem_samples_s_b", &sam);
    j.segs("segments", &segs);
    j
}

// reader threads suspended mid-run; either resumed (backpressure stall) or its blocked
// WriteFile cancelled via CancelIoEx (stall teardown)
fn case_suspend(a: &Args, cancel: bool) -> J {
    let n_frames = (a.seconds as f64 * a.fps).round() as u64;
    let out_dir = a.out_root.join(&a.label);
    std::fs::create_dir_all(&out_dir).unwrap();
    let pipe_name = format!(r"\\.\pipe\screenrec_s1b_{}", a.label);
    let name_wide = to_wide(&pipe_name);

    let t0 = Instant::now();
    let pipe = PipeServer::create(&name_wide, a.pipe_buf).expect("CreateNamedPipeW failed");
    let t_pipe_created = t0.elapsed();
    let (mut child, cmd) = spawn_ffmpeg_reader(a, &out_dir, &pipe_name, &[]);
    let pid = child.id();
    pipe.connect().expect("ffmpeg did not connect to pipe");
    let connect_ms = (t0.elapsed() - t_pipe_created).as_secs_f64() * 1000.0;

    let stop = Arc::new(AtomicBool::new(false));
    let samples: SampleLog = Arc::new(Mutex::new(Vec::new()));
    let sh = start_sampler(stop.clone(), samples.clone());

    // watcher: suspend at --suspend-at, then resume after --resume-after (or cancel blocked
    // WriteFile at --cancel-write-at and resume 300ms later)
    let h_isize = h_as_isize(pipe.handle);
    let suspend_at = a.suspend_at;
    let resume_after = a.resume_after;
    let cancel_write_at = a.cancel_write_at;
    let watcher = std::thread::spawn(move || {
        let sw = Instant::now();
        let until = Duration::from_secs_f64(suspend_at);
        if sw.elapsed() < until { std::thread::sleep(until - sw.elapsed()); }
        let t_susp = sw.elapsed().as_secs_f64();
        let suspended = suspend_threads_of(pid);
        let n_susp = suspended.len();
        let mut cancel_res: Option<i32> = None;
        if cancel {
            let until_c = Duration::from_secs_f64(cancel_write_at);
            if sw.elapsed() < until_c { std::thread::sleep(until_c - sw.elapsed()); }
            cancel_res = match unsafe { CancelIoEx(h_from_isize(h_isize), None) } {
                Ok(()) => None,
                Err(e) => Some(e.code().0),
            };
            std::thread::sleep(Duration::from_millis(300)); // let writer observe the abort
        } else {
            let until_r = Duration::from_secs_f64(resume_after);
            std::thread::sleep(until_r);
        }
        let n_res = resume_threads(&suspended);
        (t_susp, n_susp, n_res, cancel_res)
    });

    let mut prep = Prep::new();
    let t_write_start = Instant::now();
    let lp = write_paced(&mut prep, &pipe, t_write_start, n_frames, a.fps);
    let write_wall_s = t_write_start.elapsed().as_secs_f64();

    let wres = watcher.join().unwrap();
    drop(pipe);
    let exit = reap(&mut child, Duration::from_secs(30));
    let wall_s = t0.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    let _ = sh.join();
    copy_stderr(&out_dir, &a.raw_dir, &a.label);

    let segs = probe_segments(&a.ff, &out_dir);
    let packets_sum: u64 = segs.iter().map(|s| s.packets.unwrap_or(0)).sum();
    let (late_max, late_over) = late_stats(&lp.late);
    let ww = summary(&lp.write_wait, 0);
    let sam = samples.lock().unwrap().clone();
    let mem_max = sam.iter().map(|x| x.1).max().unwrap_or(0);

    let mut j = J::new();
    j.s("case", &a.case); j.s("label", &a.label); j.s("mode", if cancel { "suspend+cancel-write" } else { "suspend+resume" });
    j.s("ffmpeg_cmd", &cmd);
    j.num("pipe_buf", a.pipe_buf); j.num("n_frames_target", n_frames); j.num("frames_written", lp.frames_written);
    j.f3("connect_ms", connect_ms);
    j.f3("suspend_at_s", wres.0);
    j.num("threads_suspended", wres.1); j.num("threads_resumed", wres.2);
    match wres.3 {
        Some(c) => j.raw("cancel_io", format!("{{\"code\":{c},\"name\":\"{}\"}}", err_name(c))),
        None => j.raw("cancel_io", format!("{{\"code\":0,\"name\":\"{}\"}}", "ok")),
    }
    match lp.error {
        Some((i, code)) => j.raw("write_error", format!("{{\"frame\":{i},\"code\":{code},\"name\":\"{}\"}}", err_name(code))),
        None => j.raw("write_error", "null".into()),
    }
    j.f3("write_wall_s", write_wall_s);
    j.f3("wall_s", wall_s);
    j.raw("blocked_first_frame", lp.blocked_first_frame.map(|x| format!("{x}")).unwrap_or_else(|| "null".into()));
    j.f1("late_max_ms", late_max); j.num("late_over_budget_frames", late_over);
    j.stat("write_wait_ms", ww);
    j.num("packets_sum", packets_sum);
    j.opt("ffmpeg_exit", exit);
    j.num("mem_private_max_b", mem_max);
    j.samples("mem_samples_s_b", &sam);
    j.segs("segments", &segs);
    j
}

// taskkill /F ffmpeg; optionally with reader suspended first (writer blocked in WriteFile)
fn case_kill(a: &Args, stalled: bool) -> J {
    let n_frames = (a.seconds as f64 * a.fps).round() as u64;
    let out_dir = a.out_root.join(&a.label);
    std::fs::create_dir_all(&out_dir).unwrap();
    let pipe_name = format!(r"\\.\pipe\screenrec_s1b_{}", a.label);
    let name_wide = to_wide(&pipe_name);

    let t0 = Instant::now();
    let pipe = PipeServer::create(&name_wide, a.pipe_buf).expect("CreateNamedPipeW failed");
    let t_pipe_created = t0.elapsed();
    let (child, cmd) = spawn_ffmpeg_reader(a, &out_dir, &pipe_name, &[]);
    let child = Arc::new(Mutex::new(child));
    let pid = child.lock().unwrap().id();
    pipe.connect().expect("ffmpeg did not connect to pipe");
    let connect_ms = (t0.elapsed() - t_pipe_created).as_secs_f64() * 1000.0;

    let stop = Arc::new(AtomicBool::new(false));
    let samples: SampleLog = Arc::new(Mutex::new(Vec::new()));
    let sh = start_sampler(stop.clone(), samples.clone());

    let kill_at = a.kill_at;
    let sbk = a.suspend_before_kill;
    let child_w = child.clone();
    let watcher = std::thread::spawn(move || {
        let sw = Instant::now();
        let mut n_susp = 0usize;
        if stalled {
            let until = Duration::from_secs_f64(kill_at - sbk);
            if sw.elapsed() < until { std::thread::sleep(until - sw.elapsed()); }
            let suspended = suspend_threads_of(pid);
            n_susp = suspended.len();
            // keep suspended until kill moment
            let until_k = Duration::from_secs_f64(kill_at);
            if sw.elapsed() < until_k { std::thread::sleep(until_k - sw.elapsed()); }
        } else {
            let until = Duration::from_secs_f64(kill_at);
            if sw.elapsed() < until { std::thread::sleep(until - sw.elapsed()); }
        }
        let kill_issue = sw.elapsed().as_secs_f64();
        let tk_sw = Instant::now();
        let tk = Command::new("taskkill").args(["/F", "/T", "/PID", &format!("{pid}")])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let taskkill_ms = tk_sw.elapsed().as_secs_f64() * 1000.0;
        // poll for actual process death (5ms resolution) to separate taskkill overhead from pipe teardown
        let mut death_rel = None;
        for _ in 0..4000 {
            {
                let mut c = child_w.lock().unwrap();
                if let Ok(Some(_)) = c.try_wait() { death_rel = Some(sw.elapsed().as_secs_f64()); break; }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        (kill_issue, taskkill_ms, tk.map(|o| o.stdout).unwrap_or_default(), n_susp, death_rel)
    });

    let mut prep = Prep::new();
    let t_write_start = Instant::now();
    let lp = write_paced(&mut prep, &pipe, t_write_start, n_frames, a.fps);
    // write_wall_s is measured from ~t0; kill_issue also from ~t0 -> difference = detection latency
    let write_wall_s = t_write_start.elapsed().as_secs_f64();

    let wres = watcher.join().unwrap();
    drop(pipe);
    let exit = { let mut c = child.lock().unwrap(); reap(&mut c, Duration::from_secs(15)) };
    let wall_s = t0.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    let _ = sh.join();
    copy_stderr(&out_dir, &a.raw_dir, &a.label);

    let segs = probe_segments(&a.ff, &out_dir);
    let packets_sum: u64 = segs.iter().map(|s| s.packets.unwrap_or(0)).sum();
    let (late_max, late_over) = late_stats(&lp.late);
    let ww = summary(&lp.write_wait, 0);
    let sam = samples.lock().unwrap().clone();

    let mut j = J::new();
    j.s("case", &a.case); j.s("label", &a.label); j.s("mode", if stalled { "kill-while-writer-blocked" } else { "kill-reader-active" });
    j.s("ffmpeg_cmd", &cmd);
    j.s("taskkill_stdout", String::from_utf8_lossy(&wres.2).trim());
    j.f3("taskkill_ms", wres.1);
    j.raw("child_death_rel_s", wres.4.map(|x| format!("{x:.3}")).unwrap_or_else(|| "null".into()));
    j.num("threads_suspended_pre_kill", wres.3);
    j.f3("kill_issue_at_s", wres.0);
    j.num("pipe_buf", a.pipe_buf); j.num("n_frames_target", n_frames); j.num("frames_written", lp.frames_written);
    j.f3("connect_ms", connect_ms);
    match lp.error {
        Some((i, code)) => {
            let lat_after_issue = write_wall_s - wres.0;
            let lat_after_death = wres.4.map(|d| write_wall_s - d);
            j.raw("write_error", format!("{{\"frame\":{i},\"code\":{code},\"name\":\"{}\",\"detect_after_kill_issue_s\":{lat_after_issue:.3},\"detect_after_child_death_s\":{}}}",
                err_name(code),
                lat_after_death.map(|x| format!("{x:.3}")).unwrap_or_else(|| "null".into())));
        }
        None => j.raw("write_error", "null".into()),
    }
    j.f3("write_wall_s", write_wall_s);
    j.f3("wall_s", wall_s);
    j.raw("blocked_first_frame", lp.blocked_first_frame.map(|x| format!("{x}")).unwrap_or_else(|| "null".into()));
    j.f1("late_max_ms", late_max); j.num("late_over_budget_frames", late_over);
    j.stat("write_wait_ms", ww);
    j.num("packets_sum", packets_sum);
    j.opt("ffmpeg_exit", exit);
    j.samples("mem_samples_s_b", &sam);
    j.segs("segments", &segs);
    j
}

// blocking ConnectNamedPipe with CancelIoEx fallback; tests in-place reuse and recreate recovery
fn connect_with_cancel(pipe: &PipeServer, timeout: Duration) -> (Duration, Option<i32>, Option<i32>) {
    let h_isize = h_as_isize(pipe.handle);
    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let jh = std::thread::spawn(move || {
        let t = Instant::now();
        let r = unsafe { ConnectNamedPipe(h_from_isize(h_isize), None) };
        let res = match r {
            Ok(()) => None,
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => None,
            Err(e) => Some(e.code().0),
        };
        done2.store(true, Ordering::SeqCst);
        (t.elapsed(), res)
    });
    std::thread::sleep(timeout);
    let mut cancel_err: Option<i32> = None;
    let mut cancelled = false;
    if !done.load(Ordering::SeqCst) {
        cancelled = true;
        if let Err(e) = unsafe { CancelIoEx(h_from_isize(h_isize), None) } { cancel_err = Some(e.code().0); }
    }
    let (dur, err) = jh.join().unwrap();
    (dur, err, if cancelled { cancel_err } else { None })
}

fn case_connect_timeout(a: &Args) -> J {
    let out_dir = a.out_root.join(&a.label);
    std::fs::create_dir_all(&out_dir).unwrap();
    let pipe_name = format!(r"\\.\pipe\screenrec_s1b_{}", a.label);
    let name_wide = to_wide(&pipe_name);
    let timeout = Duration::from_millis(a.cancel_connect_after_ms);

    // phase 1: decoy ffmpeg that never opens the pipe; blocking ConnectNamedPipe must hang until cancel
    let pipe = PipeServer::create(&name_wide, a.pipe_buf).expect("CreateNamedPipeW failed");
    let t0 = Instant::now();
    let decoy_seg = out_dir.join("decoy_%05d.mkv");
    let decoy_stderr = std::fs::File::create(out_dir.join("decoy_stderr.log")).unwrap();
    let decoy_args: Vec<String> = ["-hide_banner".into(), "-nostdin".into(), "-f".into(), "lavfi".into(),
        "-i".into(), "testsrc2=size=1920x1080:rate=30".into(), "-t".into(), "60".into(),
        "-c:v".into(), "h264_qsv".into(), "-y".into(), decoy_seg.to_string_lossy().to_string()].to_vec();
    let mut decoy = Command::new(&a.ff).args(&decoy_args)
        .stdout(Stdio::null()).stderr(Stdio::from(decoy_stderr)).spawn().expect("spawn decoy ffmpeg");
    let decoy_cmd = format!("ffmpeg {}", decoy_args.join(" "));

    let (block_dur, block_err, _cancel_err1) = connect_with_cancel(&pipe, timeout);
    let phase1_wall_s = t0.elapsed().as_secs_f64();
    let decoy_exit = reap(&mut decoy, Duration::from_secs(20));
    let _ = std::fs::copy(out_dir.join("decoy_stderr.log"), a.raw_dir.join(format!("{}_decoy_stderr.log", a.label)));

    // phase 2: in-place reuse attempt: DisconnectNamedPipe then ConnectNamedPipe again
    let dis = pipe.disconnect();
    let dis_res = match &dis { Ok(()) => "ok".to_string(), Err(e) => format!("{} ({})", e.code().0, err_name(e.code().0)) };
    let (mut child2, cmd2) = spawn_ffmpeg_reader(a, &out_dir, &pipe_name, &[]);
    let (reuse_dur, reuse_err, _cancel_err2) = connect_with_cancel(&pipe, Duration::from_secs(8));
    let reuse_ok = reuse_err.is_none();
    let reuse_desc = format!("disconnect_res={dis_res}; connect_dur={:.3}s; connect_err={reuse_err:?}", reuse_dur.as_secs_f64());
    let mut reuse_frames = 0u64;
    let mut reuse_exit: Option<i32> = None;
    let mut reuse_segs: Vec<SegInfo> = Vec::new();
    if reuse_ok {
        let mut prep = Prep::new();
        let lp = write_paced(&mut prep, &pipe, Instant::now(), 30, a.fps);
        reuse_frames = lp.frames_written;
        drop(pipe);
        reuse_exit = reap(&mut child2, Duration::from_secs(30));
        reuse_segs = probe_segments(&a.ff, &out_dir);
    } else {
        drop(pipe);
        let _ = reap(&mut child2, Duration::from_secs(10));
    }

    // phase 3: recreate pipe (fresh handle, same name) — the fallback recovery path
    let mut recreate = String::from("skipped (reuse succeeded)");
    let mut recreate_frames = 0u64;
    let mut recreate_exit: Option<i32> = None;
    let mut recreate_segs: Vec<SegInfo> = Vec::new();
    let mut recreate_ok = reuse_ok;
    if !reuse_ok {
        let pipe2 = PipeServer::create(&name_wide, a.pipe_buf).expect("recreate CreateNamedPipeW failed");
        let (mut child3, _cmd3) = spawn_ffmpeg_reader(a, &out_dir, &pipe_name, &[]);
        let (dur3, err3, _cancel_err3) = connect_with_cancel(&pipe2, Duration::from_secs(8));
        recreate = format!("connect_dur={:.3}s; connect_err={err3:?}", dur3.as_secs_f64());
        recreate_ok = err3.is_none();
        if recreate_ok {
            let mut prep = Prep::new();
            let lp = write_paced(&mut prep, &pipe2, Instant::now(), 30, a.fps);
            recreate_frames = lp.frames_written;
            drop(pipe2);
            recreate_exit = reap(&mut child3, Duration::from_secs(30));
            recreate_segs = probe_segments(&a.ff, &out_dir);
        } else {
            drop(pipe2);
            let _ = reap(&mut child3, Duration::from_secs(10));
        }
    }
    copy_stderr(&out_dir, &a.raw_dir, &a.label);

    let mut j = J::new();
    j.s("case", "connect-timeout"); j.s("label", &a.label);
    j.s("decoy_cmd", &decoy_cmd);
    j.raw("phase1_connect", format!("{{\"dur_s\":{:.3},\"err\":{}}}", block_dur.as_secs_f64(),
        block_err.map(|c| format!("{{\"code\":{c},\"name\":\"{}\"}}", err_name(c))).unwrap_or_else(|| "null".into())));
    j.opt("decoy_exit", decoy_exit);
    j.f3("phase1_wall_s", phase1_wall_s);
    j.s("phase2_reuse", &reuse_desc);
    j.num("reuse_frames_written", reuse_frames);
    j.opt("reuse_ffmpeg_exit", reuse_exit);
    if !reuse_segs.is_empty() { j.segs("reuse_segments", &reuse_segs); }
    j.s("phase3_recreate", &recreate);
    j.num("recreate_ok", recreate_ok);
    j.num("recreate_frames_written", recreate_frames);
    j.opt("recreate_ffmpeg_exit", recreate_exit);
    if !recreate_segs.is_empty() { j.segs("recreate_segments", &recreate_segs); }
    j.s("ffmpeg_cmd_reader", &cmd2);
    j
}

// writer closes pipe early -> ffmpeg sees EOF -> finalize + exit
fn case_eof(a: &Args) -> J {
    let n_frames = 90; // 3s @30fps
    let out_dir = a.out_root.join(&a.label);
    std::fs::create_dir_all(&out_dir).unwrap();
    let pipe_name = format!(r"\\.\pipe\screenrec_s1b_{}", a.label);
    let name_wide = to_wide(&pipe_name);

    let t0 = Instant::now();
    let pipe = PipeServer::create(&name_wide, a.pipe_buf).expect("CreateNamedPipeW failed");
    let t_pipe_created = t0.elapsed();
    let (mut child, cmd) = spawn_ffmpeg_reader(a, &out_dir, &pipe_name, &[]);
    pipe.connect().expect("ffmpeg did not connect to pipe");
    let connect_ms = (t0.elapsed() - t_pipe_created).as_secs_f64() * 1000.0;

    let mut prep = Prep::new();
    let t_write_start = Instant::now();
    let lp = write_paced(&mut prep, &pipe, t_write_start, n_frames, a.fps);
    let write_wall_s = t_write_start.elapsed().as_secs_f64();
    drop(pipe); // EOF
    let eof_at_s = t0.elapsed().as_secs_f64();
    let exit = reap(&mut child, Duration::from_secs(30));
    let exit_after_eof_s = t0.elapsed().as_secs_f64() - eof_at_s;
    let wall_s = t0.elapsed().as_secs_f64();
    copy_stderr(&out_dir, &a.raw_dir, &a.label);

    let segs = probe_segments(&a.ff, &out_dir);
    let packets_sum: u64 = segs.iter().map(|s| s.packets.unwrap_or(0)).sum();
    let (late_max, late_over) = late_stats(&lp.late);
    let ww = summary(&lp.write_wait, 0);

    let mut j = J::new();
    j.s("case", "eof"); j.s("label", &a.label);
    j.s("ffmpeg_cmd", &cmd);
    j.num("pipe_buf", a.pipe_buf); j.num("n_frames_target", n_frames); j.num("frames_written", lp.frames_written);
    j.f3("connect_ms", connect_ms);
    j.f3("write_wall_s", write_wall_s);
    j.f3("eof_at_s", eof_at_s);
    j.f3("exit_after_eof_s", exit_after_eof_s);
    j.f3("wall_s", wall_s);
    match lp.error {
        Some((i, code)) => j.raw("write_error", format!("{{\"frame\":{i},\"code\":{code},\"name\":\"{}\"}}", err_name(code))),
        None => j.raw("write_error", "null".into()),
    }
    j.f1("late_max_ms", late_max); j.num("late_over_budget_frames", late_over);
    j.stat("write_wait_ms", ww);
    j.num("packets_sum", packets_sum);
    j.opt("ffmpeg_exit", exit);
    j.segs("segments", &segs);
    j
}

// repeated start/stop loop -> handle/thread/memory/zombie leak check
fn case_restart(a: &Args) -> J {
    let per_iter_frames = 45; // 1.5s @30fps
    let snap_before = (handle_count_self(), thread_count_self(), private_usage_bytes(), ffmpeg_proc_count());
    let mut iters: Vec<String> = Vec::new();
    let mut killed_count = 0u32;
    let mut mid: Option<(u32, u32, u64, u32)> = None;
    for it in 0..a.restart_iters {
        let label = format!("{}i{it:02}", a.label);
        let out_dir = a.out_root.join(&label);
        std::fs::create_dir_all(&out_dir).unwrap();
        let pipe_name = format!(r"\\.\pipe\screenrec_s1b_{label}");
        let name_wide = to_wide(&pipe_name);
        let t0 = Instant::now();
        let pipe = PipeServer::create(&name_wide, a.pipe_buf).expect("CreateNamedPipeW failed");
        let (mut child, _) = spawn_ffmpeg_reader(a, &out_dir, &pipe_name, &[]);
        let c = pipe.connect();
        let connect_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let connect_ok = c.is_ok();
        let mut frames = 0u64;
        if connect_ok {
            let mut prep = Prep::new();
            let lp = write_paced(&mut prep, &pipe, t0, per_iter_frames, a.fps);
            frames = lp.frames_written;
        }
        drop(pipe);
        let exit = reap(&mut child, Duration::from_secs(30));
        if exit.is_none() { killed_count += 1; }
        let segs = probe_segments(&a.ff, &out_dir);
        let packets_sum: u64 = segs.iter().map(|s| s.packets.unwrap_or(0)).sum();
        let dur_sum: f64 = segs.iter().map(|s| s.duration.unwrap_or(0.0)).sum();
        let _ = std::fs::remove_dir_all(&out_dir);
        iters.push(format!("{{\"iter\":{it},\"connect_ok\":{connect_ok},\"connect_ms\":{connect_ms:.1},\"frames\":{frames},\"packets\":{packets_sum},\"dur\":{dur_sum:.3},\"exit\":{}}}",
            exit.map(|x| x.to_string()).unwrap_or_else(|| "null".into())));
        if it + 1 == a.restart_iters / 2 {
            mid = Some((handle_count_self(), thread_count_self(), private_usage_bytes(), ffmpeg_proc_count()));
        }
    }
    let snap_after = (handle_count_self(), thread_count_self(), private_usage_bytes(), ffmpeg_proc_count());
    let mut j = J::new();
    j.s("case", "restart"); j.s("label", &a.label);
    j.num("iters", a.restart_iters); j.num("frames_per_iter", per_iter_frames);
    j.num("handles_before", snap_before.0); j.num("handles_after", snap_after.0);
    j.num("threads_before", snap_before.1); j.num("threads_after", snap_after.1);
    j.num("private_b_before", snap_before.2); j.num("private_b_after", snap_after.2);
    j.num("ffmpeg_procs_before", snap_before.3); j.num("ffmpeg_procs_after", snap_after.3);
    if let Some(m) = mid {
        j.num("handles_mid", m.0); j.num("threads_mid", m.1);
        j.num("private_b_mid", m.2); j.num("ffmpeg_procs_mid", m.3);
    }
    j.num("forced_kills", killed_count);
    j.raw("iters", format!("[{}]", iters.join(",")));
    j
}

fn main() {
    let a = parse_args();
    println!("pipefault case={} label={} seconds={} fps={} pipe_buf={}", a.case, a.label, a.seconds, a.fps, a.pipe_buf);
    std::fs::create_dir_all(&a.raw_dir).unwrap();
    let j = match a.case.as_str() {
        "slowread" => case_paced_like(&a, if a.readrate > 0.0 { a.readrate } else { 0.25 }), // catchup via --readrate-catchup (default 0.25 in driver for strict cap)
        "bufsz" => case_paced_like(&a, 0.0),
        "suspend-resume" => case_suspend(&a, false),
        "suspend-cancel" => case_suspend(&a, true),
        "kill" => case_kill(&a, false),
        "kill-stalled" => case_kill(&a, true),
        "connect-timeout" => case_connect_timeout(&a),
        "eof" => case_eof(&a),
        "restart" => case_restart(&a),
        other => panic!("unknown case {other}"),
    };
    j.write(&a.raw_dir, &a.label);
    println!("DONE case={}", a.case);
}
