// pipebench — S1 spike harness (spike tooling, NOT production code).
//
// Simulates the v1 capture pipeline per route:
//   synthetic BGRA frame -> scale to fixed canvas (letterbox) -> NV12 -> named pipe -> ffmpeg (rawvideo in, h264_qsv out, 5s MKV segments)
// and measures per-stage frame costs, pacing/backlog, pipe byte rate, startup latency, CPU usage.
//
// All encode outputs go to a temp dir; stats JSON + ffmpeg stderr logs land in docs/spikes/s1/raw/.

use std::f64::consts::TAU;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, FILETIME, HANDLE};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, WriteFile, PIPE_ACCESS_OUTBOUND};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, NAMED_PIPE_MODE, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetSystemTimes, GetThreadTimes};

const CANVAS_W: usize = 1920;
const CANVAS_H: usize = 1080;
const NV12_FRAME_BYTES: usize = CANVAS_W * CANVAS_H * 3 / 2;
const FF_DIR: &str = r"C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\tools\ffmpeg\ffmpeg-master-latest-win64-lgpl\bin";
const RAW_DIR: &str = r"C:\Users\elvis\Documents\From MSI\git_repository\Screen_rec\docs\spikes\s1\raw";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Variant {
    Scale,     // 1600x900 -> exact fit 1920x1080 (both 16:9, no bars)
    NoScale,   // generate 1920x1080 directly; convert only
    Letterbox, // 1600x1000 (16:10) -> fitted 1728x1080 centered in 1920x1080 canvas (side bars)
}

#[derive(Clone)]
struct Args {
    mode_paced: bool,
    routes: usize,
    seconds: u64,
    fps: f64,
    variant: Variant,
    out_root: PathBuf,
    label: String,
    ff: PathBuf,
    pipe_buf: u32,
}

fn parse_args() -> Args {
    let mut a = Args {
        mode_paced: true,
        routes: 1,
        seconds: 10,
        fps: 30.0,
        variant: Variant::Scale,
        out_root: std::env::temp_dir().join("opencode").join("s1out"),
        label: "run".to_string(),
        ff: PathBuf::from(FF_DIR).join("ffmpeg.exe"),
        pipe_buf: 4 * 1024 * 1024,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--mode" => a.mode_paced = it.next().map(|s| s != "freerun").unwrap_or(true),
            "--routes" => a.routes = it.next().and_then(|s| s.parse().ok()).unwrap_or(1),
            "--seconds" => a.seconds = it.next().and_then(|s| s.parse().ok()).unwrap_or(10),
            "--fps" => a.fps = it.next().and_then(|s| s.parse().ok()).unwrap_or(30.0),
            "--variant" => {
                a.variant = match it.next().as_deref() {
                    Some("noscale") => Variant::NoScale,
                    Some("letterbox") => Variant::Letterbox,
                    _ => Variant::Scale,
                }
            }
            "--out-root" => a.out_root = it.next().map(PathBuf::from).unwrap_or(a.out_root),
            "--label" => a.label = it.next().unwrap_or_else(|| "run".into()),
            "--ff" => a.ff = it.next().map(PathBuf::from).unwrap_or(a.ff),
            "--pipe-buf" => a.pipe_buf = it.next().and_then(|s| s.parse().ok()).unwrap_or(4 * 1024 * 1024),
            other => panic!("unknown arg {other}"),
        }
    }
    a
}

// ---------- synthetic frame generation (test-only cost) ----------
fn gen_frame_bgra(buf: &mut [u8], w: usize, h: usize, t: u64) {
    // Cheap animated gradient (row-wise, auto-vectorizable).
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

// ---------- BGRA -> NV12 (BT.601 limited range) ----------
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

// ---------- stage timers ----------
type Stat = (f64, f64, f64, f64); // avg, p50, p95, max (ms)
struct StageStats(Vec<f64>);
impl StageStats {
    fn new() -> Self { StageStats(Vec::new()) }
    fn push(&mut self, d: Duration) { self.0.push(d.as_secs_f64() * 1000.0); }
    fn summary(&self) -> Stat { self.summary_from(0) }
    fn summary_from(&self, skip: usize) -> Stat {
        let mut s = if skip > 0 && self.0.len() > skip { self.0[skip..].to_vec() } else { self.0.clone() };
        if s.is_empty() { return (0.0, 0.0, 0.0, 0.0); }
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = s.len();
        (s.iter().sum::<f64>() / n as f64, s[n / 2], s[n * 95 / 100], s[n - 1])
    }
}

fn filetime_to_u64(ft: &FILETIME) -> u64 { ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64 }

fn process_cpu(handle: HANDLE) -> (u64, u64) {
    unsafe {
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetProcessTimes(handle, &mut c, &mut e, &mut k, &mut u);
        (filetime_to_u64(&k), filetime_to_u64(&u))
    }
}

fn system_times() -> (u64, u64, u64) {
    unsafe {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user));
        (filetime_to_u64(&idle), filetime_to_u64(&kernel), filetime_to_u64(&user))
    }
}

fn logical_cores() -> usize {
    unsafe {
        let mut si = SYSTEM_INFO::default();
        GetSystemInfo(&mut si);
        si.dwNumberOfProcessors as usize
    }
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
    fn connect(&self) -> windows::core::Result<()> {
        unsafe {
            match ConnectNamedPipe(self.handle, None) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(()),
                Err(e) => Err(e),
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
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        unsafe {
            let _ = FlushFileBuffers(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

fn to_wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

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

#[derive(Clone)]
struct RouteResult {
    route: usize,
    variant: String,
    paced: bool,
    frames_written: u64,
    elapsed_s: f64,
    free_run_fps: f64,
    late_max_ms: f64,
    late_over_budget_frames: u64,
    late_steady_max_ms: f64,
    late_over_budget_steady: u64,
    write_wait_steady: Stat,
    connect_ms: f64,
    first_write_ms: f64,
    gen: Stat,
    resize: Stat,
    convert: Stat,
    write_wait: Stat,
    pipe_bytes: u64,
    pipe_MBps: f64,
    worker_thread_pct: f64,
    ffmpeg_cores: f64,
    ffmpeg_exit: Option<i32>,
    segments: Vec<SegInfo>,
}

fn worker(route: usize, args: &Args) -> RouteResult {
    let (src_w, src_h) = match args.variant {
        Variant::Scale => (1600usize, 900usize),
        Variant::NoScale => (CANVAS_W, CANVAS_H),
        Variant::Letterbox => (1600usize, 1000usize),
    };
    // aspect-fit into canvas (even dims)
    let s = ((CANVAS_W as f64 / src_w as f64).min(CANVAS_H as f64 / src_h as f64) * 2.0).floor() / 2.0;
    let fit_w = ((src_w as f64 * s) as usize) & !1;
    let fit_h = ((src_h as f64 * s) as usize) & !1;
    let (off_x, off_y) = ((CANVAS_W - fit_w) / 2, (CANVAS_H - fit_h) / 2);

    let n_frames = (args.seconds as f64 * args.fps).round() as u64;
    let out_dir = args.out_root.join(&args.label).join(format!("r{route}"));
    std::fs::create_dir_all(&out_dir).unwrap();
    let pipe_name = format!(r"\\.\pipe\screenrec_s1_{}_r{route}", args.label);
    let name_wide = to_wide(&pipe_name);

    let mut src_buf = vec![0u8; src_w * src_h * 4];
    let mut canvas = vec![0u8; CANVAS_W * CANVAS_H * 4];
    for px in canvas.chunks_exact_mut(4) { px.copy_from_slice(&[16, 16, 16, 255]); } // studio black bars
    let mut nv12 = vec![0u8; NV12_FRAME_BYTES];
    nv12[..CANVAS_W * CANVAS_H].fill(16);
    nv12[CANVAS_W * CANVAS_H..].fill(128);

    let mut resizer = fast_image_resize::Resizer::new();
    let mut fitted_img = fast_image_resize::images::Image::new(fit_w as u32, fit_h as u32, fast_image_resize::PixelType::U8x4);

    let t0 = Instant::now();
    let pipe = PipeServer::create(&name_wide, args.pipe_buf).expect("CreateNamedPipeW failed");
    let t_pipe_created = t0.elapsed();

    let seg_pattern = out_dir.join("out_%05d.mkv");
    let stderr_log = std::fs::File::create(out_dir.join("ffmpeg_stderr.log")).unwrap();
    let mut child = Command::new(&args.ff)
        .args(["-hide_banner", "-nostdin",
               "-f", "rawvideo", "-pix_fmt", "nv12", "-s", &format!("{}x{}", CANVAS_W, CANVAS_H), "-framerate", &args.fps.to_string(),
               "-i", &pipe_name,
               "-c:v", "h264_qsv", "-g", "60",
               "-f", "segment", "-segment_time", "5", "-reset_timestamps", "1", "-segment_format", "matroska"])
        .arg(&seg_pattern)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_log))
        .spawn()
        .expect("spawn ffmpeg failed");
    let child_handle = HANDLE(child.as_raw_handle());

    pipe.connect().expect("ffmpeg did not connect to pipe");
    let t_connected = t0.elapsed();

    let mut gen = StageStats::new();
    let mut resize = StageStats::new();
    let mut convert = StageStats::new();
    let mut write_wait = StageStats::new();
    let mut late_ms: Vec<f64> = Vec::with_capacity(n_frames as usize);
    let mut pipe_bytes: u64 = 0;
    let mut t_first_write: Option<Instant> = None;
    let mut t_first_frame_done: Option<Instant> = None;
    let mut t_start = t0;

    for i in 0..n_frames {
        if args.mode_paced {
            let target = t0 + Duration::from_secs_f64(i as f64 / args.fps);
            let now = Instant::now();
            if now < target { std::thread::sleep(target - now); }
        } else if i == 0 {
            t_start = Instant::now();
        }
        let s = Instant::now();
        gen_frame_bgra(&mut src_buf, src_w, src_h, i);
        gen.push(s.elapsed());

        let s = Instant::now();
        if args.variant != Variant::NoScale {
            let vec = std::mem::take(&mut src_buf);
            let src_img = fast_image_resize::images::Image::from_vec_u8(src_w as u32, src_h as u32, vec, fast_image_resize::PixelType::U8x4).expect("src image");
            let opts = fast_image_resize::ResizeOptions::new()
                .resize_alg(fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Bilinear));
            resizer.resize(&src_img, &mut fitted_img, &opts).expect("resize");
            src_buf = src_img.into_vec();
            if fit_w != CANVAS_W || fit_h != CANVAS_H {
                let fit_row = fit_w * 4;
                let canvas_row = CANVAS_W * 4;
                let fb = fitted_img.buffer();
                for row in 0..fit_h {
                    let d = (off_y + row) * canvas_row + off_x * 4;
                    let sr = row * fit_row;
                    canvas[d..d + fit_row].copy_from_slice(&fb[sr..sr + fit_row]);
                }
            }
        }
        resize.push(s.elapsed());

        let s = Instant::now();
        bgra_to_nv12(&canvas, CANVAS_W, CANVAS_H, &mut nv12);
        convert.push(s.elapsed());

        let s = Instant::now();
        pipe.write_all(&nv12).expect("pipe write failed");
        if t_first_write.is_none() { t_first_write = Some(Instant::now()); }
        write_wait.push(s.elapsed());
        pipe_bytes += NV12_FRAME_BYTES as u64;
        if t_first_frame_done.is_none() { t_first_frame_done = Some(Instant::now()); }

        if args.mode_paced {
            let deadline = t0 + Duration::from_secs_f64((i + 1) as f64 / args.fps);
            late_ms.push((Instant::now() - deadline).as_secs_f64() * 1000.0);
        }
    }
    let t_write_done = Instant::now();
    drop(pipe); // EOF for ffmpeg

    let mut ffmpeg_exit = None;
    for _ in 0..600 {
        match child.try_wait() {
            Ok(Some(code)) => { ffmpeg_exit = Some(code.code().unwrap_or(-1)); break; }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    if ffmpeg_exit.is_none() { let _ = child.kill(); let _ = child.wait(); }
    let t_child_done = Instant::now();

    let ffmpeg_cores = {
        let (k, u) = process_cpu(child_handle);
        ((k + u) as f64 / 1e7) / (t_child_done - t0).as_secs_f64().max(0.001)
    };
    drop(child);

    let worker_thread_pct = unsafe {
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u);
        let busy = (filetime_to_u64(&k) + filetime_to_u64(&u)) as f64 / 1e7;
        busy / (t_child_done - t_start).as_secs_f64().max(0.001) * 100.0
    };

    let ffprobe_exe = args.ff.parent().unwrap().join("ffprobe.exe");
    let mut segments = Vec::new();
    for e in std::fs::read_dir(&out_dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().map(|x| x == "mkv").unwrap_or(false) {
            let (duration, packets, codec) = ffprobe(&ffprobe_exe, &p);
            segments.push(SegInfo { path: p.to_string_lossy().to_string(), duration, packets, codec });
        }
    }
    segments.sort_by(|a, b| a.path.cmp(&b.path));

    let elapsed = if args.mode_paced { t_child_done - t0 } else { t_child_done - t_start };
    RouteResult {
        route,
        variant: format!("{:?}", args.variant).to_lowercase(),
        paced: args.mode_paced,
        frames_written: n_frames,
        elapsed_s: elapsed.as_secs_f64(),
        free_run_fps: if args.mode_paced { 0.0 } else {
            // steady-state free-run throughput: exclude frame 0 (contains encoder init absorb time)
            match t_first_frame_done {
                Some(t_ffd) if n_frames > 1 => (n_frames - 1) as f64 / (t_write_done - t_ffd).as_secs_f64(),
                _ => n_frames as f64 / (t_write_done - t_start).as_secs_f64(),
            }
        },
        late_max_ms: late_ms.iter().cloned().fold(f64::MIN, f64::max).max(0.0),
        late_over_budget_frames: late_ms.iter().filter(|x| **x > 0.0).count() as u64,
        late_steady_max_ms: if late_ms.len() > 30 { late_ms[30..].iter().cloned().fold(f64::MIN, f64::max).max(0.0) } else { 0.0 },
        late_over_budget_steady: if late_ms.len() > 30 { late_ms[30..].iter().filter(|x| **x > 0.0).count() as u64 } else { 0 },
        write_wait_steady: write_wait.summary_from(30),
        connect_ms: (t_connected - t_pipe_created).as_secs_f64() * 1000.0,
        first_write_ms: (t_first_write.map(|t| t - t0).unwrap_or(t_connected) - t_pipe_created).as_secs_f64() * 1000.0,
        gen: gen.summary(),
        resize: resize.summary(),
        convert: convert.summary(),
        write_wait: write_wait.summary(),
        pipe_bytes,
        pipe_MBps: pipe_bytes as f64 / elapsed.as_secs_f64() / 1e6,
        worker_thread_pct,
        ffmpeg_cores,
        ffmpeg_exit,
        segments,
    }
}

fn json_str(s: &str) -> String { format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")) }
fn stat_json(t: &Stat) -> String {
    format!("{{\"avg\":{:.3},\"p50\":{:.3},\"p95\":{:.3},\"max\":{:.3}}}", t.0, t.1, t.2, t.3)
}

fn main() {
    let args = Arc::new(parse_args());
    let cores = logical_cores();
    let variant_s = format!("{:?}", args.variant).to_lowercase();
    println!("pipebench label={} mode={} variant={} routes={} seconds={} fps={} pipe_buf={}",
        args.label, if args.mode_paced { "paced" } else { "freerun" }, variant_s, args.routes, args.seconds, args.fps, args.pipe_buf);

    let (sys0_idle, sys0_k, sys0_u) = system_times();
    let run_start = Instant::now();

    let mut handles = Vec::new();
    for r in 0..args.routes {
        let args2 = args.clone();
        handles.push(std::thread::spawn(move || worker(r, &args2)));
    }
    let mut results: Vec<RouteResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    results.sort_by_key(|r| r.route);
    let wall = run_start.elapsed();

    let (idle1, k1, u1) = system_times();
    let sys_busy_pct = {
        let idle_d = (idle1 - sys0_idle) as f64;
        let total_d = ((k1 - sys0_k) + (u1 - sys0_u)) as f64;
        if total_d > 0.0 { (1.0 - idle_d / total_d) * 100.0 } else { 0.0 }
    };
    let (sk, su) = process_cpu(unsafe { GetCurrentProcess() });
    let self_cores = ((sk + su) as f64 / 1e7) / wall.as_secs_f64().max(0.001);

    // JSON
    let mut route_objs: Vec<String> = Vec::new();
    for r in &results {
        let segs: Vec<String> = r.segments.iter().map(|s| format!("    {{\"path\":{},\"duration\":{},\"packets\":{},\"codec\":{}}}",
            json_str(&s.path),
            s.duration.map(|x| format!("{x:.3}")).unwrap_or_else(|| "null".into()),
            s.packets.map(|x| x.to_string()).unwrap_or_else(|| "null".into()),
            s.codec.as_deref().map(json_str).unwrap_or_else(|| "null".into()))).collect();
        route_objs.push(format!(r#"  {{
    "route": {}, "variant": {}, "paced": {}, "frames_written": {}, "elapsed_s": {:.3}, "free_run_fps": {:.2},
    "late_max_ms": {:.2}, "late_over_budget_frames": {}, "late_steady_max_ms": {:.2}, "late_over_budget_steady": {}, "write_wait_steady_ms": {},
    "connect_ms": {:.1}, "first_write_ms": {:.1},
    "gen_ms": {}, "resize_ms": {}, "convert_ms": {}, "write_wait_ms": {},
    "pipe_bytes": {}, "pipe_MBps": {:.1},
    "worker_thread_pct": {:.1}, "ffmpeg_cores": {:.2}, "ffmpeg_exit": {},
    "segments": [
{}
    ]
  }}"#,
            r.route, json_str(&r.variant), r.paced, r.frames_written, r.elapsed_s, r.free_run_fps,
            r.late_max_ms, r.late_over_budget_frames, r.late_steady_max_ms, r.late_over_budget_steady, stat_json(&r.write_wait_steady), r.connect_ms, r.first_write_ms,
            stat_json(&r.gen), stat_json(&r.resize), stat_json(&r.convert), stat_json(&r.write_wait),
            r.pipe_bytes, r.pipe_MBps, r.worker_thread_pct, r.ffmpeg_cores,
            r.ffmpeg_exit.map(|x| x.to_string()).unwrap_or_else(|| "null".into()),
            segs.join(",\n")));
    }
    let json = format!(r#"{{
  "label": {}, "mode": {}, "variant": {}, "routes": {}, "seconds": {}, "fps": {}, "pipe_buf": {},
  "logical_cores": {}, "wall_s": {:.3}, "system_busy_pct": {:.1}, "self_cores": {:.2},
  "routes": [
{}
  ]
}}
"#,
        json_str(&args.label), json_str(if args.mode_paced { "paced" } else { "freerun" }), json_str(&variant_s),
        args.routes, args.seconds, args.fps, args.pipe_buf, cores, wall.as_secs_f64(), sys_busy_pct, self_cores,
        route_objs.join(",\n"));

    let raw_dir = PathBuf::from(RAW_DIR);
    std::fs::create_dir_all(&raw_dir).unwrap();
    let json_path = raw_dir.join(format!("{}.json", args.label));
    std::fs::write(&json_path, &json).unwrap();
    println!("system_busy_pct={:.1} self_cores={:.2}", sys_busy_pct, self_cores);
    println!("JSON: {}", json_path.display());
    for r in &results {
        let packets_sum: u64 = r.segments.iter().map(|s| s.packets.unwrap_or(0)).sum();
        println!("route{}: frames={} elapsed={:.2}s free_run_fps={:.1} late_max={:.2}ms over_budget={} steady(late_max={:.2}ms over_budget={} write_wait avg/p95/max={:.2}/{:.2}/{:.2}ms) | gen={:.2}ms resize={:.2}ms convert={:.2}ms | pipe={:.0}MB/s ffmpeg_cores={:.2} exit={:?} segs={} packets={}",
            r.route, r.frames_written, r.elapsed_s, r.free_run_fps, r.late_max_ms, r.late_over_budget_frames,
            r.late_steady_max_ms, r.late_over_budget_steady, r.write_wait_steady.0, r.write_wait_steady.2, r.write_wait_steady.3,
            r.gen.0, r.resize.0, r.convert.0,
            r.pipe_MBps, r.ffmpeg_cores, r.ffmpeg_exit, r.segments.len(), packets_sum);
    }
}
