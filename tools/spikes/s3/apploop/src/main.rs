// S3 spike tooling, NOT production code.
// WASAPI per-process (process-tree) application loopback: record + FFT check.
// Background and measured results: docs/spikes/s3/s3-audio-isolation.md
//
// Usage:
//   apploop record  --pid <PID> --out <file.wav> [--seconds 8] [--exclude-tree]
//                   [--pad-silence] [--no-autoconvert] [--sta] [--pre-sta]
//                   [--channels 2] [--rate 48000] [--log <file>]
//                   (repeat --pid/--out pairs to run concurrent clients in one process)
//   apploop analyze --input <file.wav> [--freqs 440,880] [--ref 440]
//                   [--nfft 32768] [--threshold-db -40] [--csv <file.csv>] [--block-s 0.5]
//   apploop probe
//
// Design notes (measured behaviour is in the report):
// - Application loopback goes through ActivateAudioInterfaceAsync with
//   AUDIOCLIENT_ACTIVATION_PARAMS { PROCESS_LOOPBACK } wrapped in a VT_BLOB PROPVARIANT;
//   the `wasapi` crate performs this internally in `new_application_loopback_client`.
// - Process loopback has no physical endpoint, so GetMixFormat is unavailable;
//   the caller chooses the format (48 kHz f32 here) and the engine converts to it.
// - COM must be initialised on the capturing thread; MTA by default here
//   (`--sta` / `--pre-sta` document the UI-thread/apartment failure modes).
// - `AudioClient`/`AudioCaptureClient` are !Send: client lifetime stays inside one thread.

use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use wasapi::{
    initialize_mta, initialize_sta, AudioClient, BufferInfo, DeviceEnumerator, Direction,
    SampleType, StreamMode, WaveFormat,
};

const USAGE: &str = "\
apploop — S3 spike: WASAPI application (process-tree) loopback record + FFT check

record  --pid <PID> --out <file.wav> [--seconds 8] [--exclude-tree] [--pad-silence]
        [--no-autoconvert] [--sta] [--pre-sta] [--device-loopback] [--channels 2]
        [--rate 48000] [--log <file>]
analyze --input <file.wav> [--freqs 440,880] [--ref 440] [--nfft 32768]
        [--threshold-db -40] [--csv <file.csv>] [--block-s 0.5]
probe";

fn main() {
    let args: Vec<String> = env::args().collect();
    if let Err(e) = run(&args) {
        eprintln!("ERROR: {e:#}");
        process::exit(1);
    }
}

fn run(args: &[String]) -> Result<()> {
    match args.get(1).map(String::as_str) {
        Some("record") => cmd_record(&args[2..]),
        Some("analyze") => cmd_analyze(&args[2..]),
        Some("probe") => cmd_probe(&args[2..]),
        Some("help") | Some("-h") | Some("--help") | None => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => bail!("unknown subcommand '{other}'\n{USAGE}"),
    }
}

// ---------------------------------------------------------------- arg helpers

fn take_value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str> {
    *i += 1;
    args.get(*i)
        .map(String::as_str)
        .with_context(|| format!("missing value for {flag}"))
}

fn take_f64(args: &[String], i: &mut usize, flag: &str) -> Result<f64> {
    let v = take_value(args, i, flag)?;
    v.parse::<f64>()
        .with_context(|| format!("bad number for {flag}: '{v}'"))
}

fn take_usize(args: &[String], i: &mut usize, flag: &str) -> Result<usize> {
    let v = take_value(args, i, flag)?;
    v.parse::<usize>()
        .with_context(|| format!("bad number for {flag}: '{v}'"))
}

// ---------------------------------------------------------------------- log

struct RunLog {
    file: Option<File>,
    start: Instant,
}

impl RunLog {
    fn new(path: Option<&Path>) -> Result<Self> {
        let file = match path {
            Some(p) => {
                if let Some(parent) = p.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).ok();
                    }
                }
                Some(File::create(p).with_context(|| format!("create log {}", p.display()))?)
            }
            None => None,
        };
        Ok(Self {
            file,
            start: Instant::now(),
        })
    }

    fn line(&mut self, msg: &str) {
        let line = format!("[{:9.3}s] {}", self.start.elapsed().as_secs_f64(), msg);
        println!("{line}");
        if let Some(f) = self.file.as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

// --------------------------------------------------------------------- record

#[derive(Clone)]
struct RecordTarget {
    pid: u32,
    out: PathBuf,
}

#[derive(Clone)]
struct RecordOpts {
    targets: Vec<RecordTarget>,
    seconds: f64,
    include_tree: bool,
    pad_silence: bool,
    autoconvert: bool,
    sta: bool,
    pre_sta: bool,
    device_loopback: bool,
    channels: usize,
    rate: usize,
    log: Option<PathBuf>,
}

fn parse_record(args: &[String]) -> Result<RecordOpts> {
    let mut o = RecordOpts {
        targets: Vec::new(),
        seconds: 8.0,
        include_tree: true,
        pad_silence: false,
        autoconvert: true,
        sta: false,
        pre_sta: false,
        device_loopback: false,
        channels: 2,
        rate: 48000,
        log: None,
    };
    let mut pending_pid: Option<u32> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--pid" => {
                let v = take_value(args, &mut i, "--pid")?;
                pending_pid = Some(v.parse::<u32>().with_context(|| format!("bad pid '{v}'"))?);
            }
            "--out" => {
                let v = take_value(args, &mut i, "--out")?;
                let pid = pending_pid.take().context("--out without preceding --pid")?;
                o.targets.push(RecordTarget {
                    pid,
                    out: PathBuf::from(v),
                });
            }
            "--seconds" => o.seconds = take_f64(args, &mut i, "--seconds")?,
            "--channels" => o.channels = take_usize(args, &mut i, "--channels")?,
            "--rate" => o.rate = take_usize(args, &mut i, "--rate")?,
            "--include-tree" => o.include_tree = true,
            "--exclude-tree" => o.include_tree = false,
            "--pad-silence" => o.pad_silence = true,
            "--no-autoconvert" => o.autoconvert = false,
            "--sta" => o.sta = true,
            "--pre-sta" => o.pre_sta = true,
            "--device-loopback" => o.device_loopback = true,
            "--log" => o.log = Some(PathBuf::from(take_value(args, &mut i, "--log")?)),
            other => bail!("unknown record option '{other}'\n{USAGE}"),
        }
        i += 1;
    }
    if pending_pid.is_some() {
        bail!("--pid given without --out");
    }
    if o.targets.is_empty() {
        bail!("record requires --pid <PID> --out <file.wav>");
    }
    if o.seconds <= 0.0 {
        bail!("--seconds must be > 0");
    }
    if o.channels == 0 || o.rate < 8000 {
        bail!("bad --channels/--rate");
    }
    Ok(o)
}

fn cmd_record(args: &[String]) -> Result<()> {
    let opts = parse_record(args)?;
    let cmdline = env::args().collect::<Vec<_>>().join(" ");

    if opts.targets.len() == 1 {
        let target = &opts.targets[0];
        let log_path = opts
            .log
            .clone()
            .unwrap_or_else(|| target.out.with_extension("run.log"));
        let mut log = RunLog::new(Some(&log_path))?;
        log.line(&format!("CMD: {cmdline}"));
        let stats = run_capture(&opts, target, &mut log)?;
        stats.log_summary(&opts, target, &mut log);
        log.line("done");
        return Ok(());
    }

    // Concurrent clients in one process: one capture thread (and one COM apartment)
    // per target. This mirrors the M4 per-route worker-thread shape.
    let mut handles = Vec::new();
    for target in opts.targets.clone() {
        let opts = opts.clone();
        let cmdline = cmdline.clone();
        let log_path = target.out.with_extension("run.log");
        let pid = target.pid;
        let h = thread::Builder::new()
            .name(format!("capture-{pid}"))
            .spawn(move || -> Result<()> {
                let mut log = RunLog::new(Some(&log_path))?;
                log.line(&format!("CMD: {cmdline}"));
                let stats = run_capture(&opts, &target, &mut log)?;
                stats.log_summary(&opts, &target, &mut log);
                log.line("done");
                Ok(())
            })?;
        handles.push((pid, h));
    }
    let mut failures = 0usize;
    for (pid, h) in handles {
        match h.join() {
            Ok(Ok(())) => println!("THREAD pid={pid} result=ok"),
            Ok(Err(e)) => {
                println!("THREAD pid={pid} result=error: {e:#}");
                failures += 1;
            }
            Err(_) => {
                println!("THREAD pid={pid} result=panic");
                failures += 1;
            }
        }
    }
    if failures > 0 {
        bail!("{failures} capture thread(s) failed");
    }
    Ok(())
}

#[derive(Default)]
struct CaptureStats {
    packets: u64,
    frames_written: u64,
    padded_frames: u64,
    empty_reads: u64,
    silent_packets: u64,
    disc_packets: u64,
    ts_error_packets: u64,
    first_packet_s: Option<f64>,
    last_packet_s: Option<f64>,
    first_ts: Option<u64>,
    last_ts: Option<u64>,
    last_index_end: Option<u64>,
    gap_events: u64,
    gap_frames: u64,
    rewind_events: u64,
    first_gap: Option<String>,
    packet_frames_min: Option<u32>,
    packet_frames_max: Option<u32>,
    rate_hz: f64,
    expected_next_ts: Option<f64>,
    ts_step_max_abs_ms: f64,
    ts_step_events: u64,
    ts_step_logged: u32,
    silent_logged: u32,
    disc_logged: u32,
    wall_s: f64,
}

impl CaptureStats {
    fn observe(&mut self, frames: u32, info: &BufferInfo, elapsed: f64, log: &mut RunLog) {
        self.packets += 1;
        if self.packets <= 5 {
            log.line(&format!(
                "PKT#{} frames={} index={} ts={} flags=silent:{} disc:{} tserr:{} t={:.3}s",
                self.packets,
                frames,
                info.index,
                info.timestamp,
                info.flags.silent,
                info.flags.data_discontinuity,
                info.flags.timestamp_error,
                elapsed
            ));
        }
        if frames > 0 {
            self.first_packet_s.get_or_insert(elapsed);
            self.last_packet_s = Some(elapsed);
            self.packet_frames_min = Some(
                self.packet_frames_min
                    .map_or(frames, |m| m.min(frames)),
            );
            self.packet_frames_max = Some(
                self.packet_frames_max
                    .map_or(frames, |m| m.max(frames)),
            );
        }
        if self.first_ts.is_none() {
            self.first_ts = Some(info.timestamp);
        }
        self.last_ts = Some(info.timestamp);
        if info.flags.silent {
            self.silent_packets += 1;
            if self.silent_logged < 12 {
                log.line(&format!(
                    "FLAG SILENT: packet#{} frames={} index={} t={:.3}s",
                    self.packets, frames, info.index, elapsed
                ));
                self.silent_logged += 1;
            }
        }
        if info.flags.data_discontinuity {
            self.disc_packets += 1;
            if self.disc_logged < 12 {
                log.line(&format!(
                    "FLAG DATA_DISCONTINUITY: packet#{} frames={} index={} t={:.3}s",
                    self.packets, frames, info.index, elapsed
                ));
                self.disc_logged += 1;
            }
        }
        if info.flags.timestamp_error {
            self.ts_error_packets += 1;
        }
        // Timestamp continuity (QPC, 100 ns units). This is the reliable time base for
        // process loopback: the device position stays at 0 for every packet.
        if let Some(expected) = self.expected_next_ts {
            let delta_ms = (info.timestamp as f64 - expected) / 1e4;
            self.ts_step_max_abs_ms = self.ts_step_max_abs_ms.max(delta_ms.abs());
            if delta_ms.abs() > 5.0 {
                self.ts_step_events += 1;
                if self.ts_step_logged < 8 {
                    log.line(&format!(
                        "TS STEP: packet#{} expected={:.3}ms got={:.3}ms delta={:+.3}ms index={} t={:.3}s",
                        self.packets,
                        expected / 1e4,
                        info.timestamp as f64 / 1e4,
                        delta_ms,
                        info.index,
                        elapsed
                    ));
                    self.ts_step_logged += 1;
                }
            }
        }
        self.expected_next_ts =
            Some(info.timestamp as f64 + frames as f64 * 1e7 / self.rate_hz);
        if let Some(prev_end) = self.last_index_end {
            if info.index > prev_end {
                self.gap_events += 1;
                self.gap_frames += info.index - prev_end;
                if self.first_gap.is_none() {
                    self.first_gap = Some(format!(
                        "expected_index={prev_end} got={} gap_frames={} t={elapsed:.3}s",
                        info.index,
                        info.index - prev_end
                    ));
                }
            } else if info.index < prev_end {
                self.rewind_events += 1;
            }
        }
        self.last_index_end = Some(info.index + frames as u64);
    }

    fn log_summary(&self, opts: &RecordOpts, target: &RecordTarget, log: &mut RunLog) {
        let rate = opts.rate as f64;
        let written_s = self.frames_written as f64 / rate;
        let padded_s = self.padded_frames as f64 / rate;
        let ts_span_s = match (self.first_ts, self.last_ts) {
            (Some(a), Some(b)) if b >= a => (b - a) as f64 / 1e7,
            _ => 0.0,
        };
        let ts_vs_frames = ts_span_s / written_s.max(1e-9);
        log.line(&format!(
            "RESULT pid={} out={}",
            target.pid,
            target.out.display()
        ));
        log.line(&format!(
            "RESULT seconds={} mode={} autoconvert={} pad_silence={} format=f32/{}ch/{}Hz",
            opts.seconds,
            if opts.device_loopback {
                "device_loopback"
            } else if opts.include_tree {
                "include_tree"
            } else {
                "exclude_tree"
            },
            opts.autoconvert,
            opts.pad_silence,
            opts.channels,
            opts.rate
        ));
        log.line(&format!(
            "RESULT packets={} frames={} written_s={:.6} padded_frames={} padded_s={:.6} wall_s={:.3}",
            self.packets, self.frames_written, written_s, self.padded_frames, padded_s, self.wall_s
        ));
        log.line(&format!(
            "RESULT flags silent_packets={} data_discontinuity_packets={} timestamp_error_packets={} empty_reads={}",
            self.silent_packets, self.disc_packets, self.ts_error_packets, self.empty_reads
        ));
        log.line(&format!(
            "RESULT first_packet_s={:?} last_packet_s={:?} ts_span_s={:.6} ts_vs_frames_ratio={:.6}",
            self.first_packet_s, self.last_packet_s, ts_span_s, ts_vs_frames
        ));
        log.line(&format!(
            "RESULT device_position_gap_events={} device_position_gap_frames={} device_position_rewind_events={} first_gap={:?} (note: process loopback keeps device position at 0 per packet; use timestamps)",
            self.gap_events, self.gap_frames, self.rewind_events, self.first_gap
        ));
        log.line(&format!(
            "RESULT ts_step_max_abs_ms={:.3} ts_step_events_gt5ms={}",
            self.ts_step_max_abs_ms, self.ts_step_events
        ));
        log.line(&format!(
            "RESULT packet_frames_min={:?} max={:?}",
            self.packet_frames_min, self.packet_frames_max
        ));
    }
}

fn fmt_waveformat(f: &WaveFormat) -> String {
    let sub = match f.get_subformat() {
        Ok(SampleType::Float) => "float",
        Ok(SampleType::Int) => "int",
        Err(_) => "unknown",
    };
    format!(
        "{}Hz {}ch {}{}bits/{}bits blockalign={}",
        f.get_samplespersec(),
        f.get_nchannels(),
        sub,
        f.get_bitspersample(),
        f.get_validbitspersample(),
        f.get_blockalign()
    )
}

fn default_endpoint_formats() -> Result<String> {
    let enumerator = DeviceEnumerator::new().context("DeviceEnumerator::new")?;
    let mut parts = Vec::new();
    for (dir, name) in [(&Direction::Render, "render"), (&Direction::Capture, "capture")] {
        match enumerator.get_default_device(dir).and_then(|d| {
            let desc = d
                .get_description()
                .unwrap_or_else(|_| "<no description>".to_string());
            let fmt = d.get_device_format()?;
            Ok((desc, fmt))
        }) {
            Ok((desc, fmt)) => parts.push(format!("{name}: '{desc}' {}", fmt_waveformat(&fmt))),
            Err(e) => parts.push(format!("{name}: error {e:?}")),
        }
    }
    Ok(parts.join(" | "))
}

fn run_capture(opts: &RecordOpts, target: &RecordTarget, log: &mut RunLog) -> Result<CaptureStats> {
    // COM apartment for this capture thread.
    if opts.pre_sta {
        let hr = initialize_sta();
        log.line(&format!(
            "COM: initialize_sta() first (simulates a UI thread that already owns an STA) hr={:#010x} ok={}",
            hr.0,
            hr.is_ok()
        ));
    }
    let hr = if opts.sta {
        initialize_sta()
    } else {
        initialize_mta()
    };
    log.line(&format!(
        "COM: initialize_{}() hr={:#010x} ok={} (RPC_E_CHANGED_MODE=0x80010106 means the thread already had another apartment)",
        if opts.sta { "sta" } else { "mta" },
        hr.0,
        hr.is_ok()
    ));

    // Context only: the default endpoint formats are irrelevant for process loopback itself.
    match default_endpoint_formats() {
        Ok(s) => log.line(&format!("ENDPOINT: {s}")),
        Err(e) => log.line(&format!("ENDPOINT: probe failed: {e:#}")),
    }

    // 1) Activate the process loopback client (ActivateAudioInterfaceAsync + VT_BLOB PROPVARIANT),
    //    or set up a classic device (system-mix) loopback as the control case.
    let t_act = Instant::now();
    let mut client = if opts.device_loopback {
        let enumerator = DeviceEnumerator::new().context("DeviceEnumerator::new")?;
        let device = enumerator
            .get_default_device(&Direction::Render)
            .context("get_default_device(Render)")?;
        let desc = device
            .get_description()
            .unwrap_or_else(|_| "<no description>".to_string());
        let c = device.get_iaudioclient().context("get_iaudioclient")?;
        log.line(&format!(
            "ACTIVATE: device loopback (system mix) on default render endpoint '{desc}' in {:.1} ms (pid={} ignored)",
            t_act.elapsed().as_secs_f64() * 1e3,
            target.pid
        ));
        c
    } else {
        match AudioClient::new_application_loopback_client(target.pid, opts.include_tree) {
            Ok(c) => {
                log.line(&format!(
                    "ACTIVATE: ok pid={} mode={} in {:.1} ms",
                    target.pid,
                    if opts.include_tree {
                        "INCLUDE_TARGET_PROCESS_TREE"
                    } else {
                        "EXCLUDE_TARGET_PROCESS_TREE"
                    },
                    t_act.elapsed().as_secs_f64() * 1e3
                ));
                c
            }
            Err(e) => {
                log.line(&format!(
                    "ACTIVATE: FAILED pid={} include_tree={} err={e:?}",
                    target.pid, opts.include_tree
                ));
                return Err(anyhow::anyhow!("activation failed: {e:?}"));
            }
        }
    };

    // 2) GetMixFormat is expected to fail for process loopback (no physical endpoint).
    match client.get_mixformat() {
        Ok(f) => log.line(&format!("GETMIXFORMAT: ok: {}", fmt_waveformat(&f))),
        Err(e) => log.line(&format!(
            "GETMIXFORMAT: error (expected for process loopback): {e:?}"
        )),
    }

    // 3) Initialise with the caller-chosen format.
    let format = WaveFormat::new(32, 32, &SampleType::Float, opts.rate, opts.channels, None);
    let mode = StreamMode::EventsShared {
        autoconvert: opts.autoconvert,
        buffer_duration_hns: 0,
    };
    if let Err(e) = client.initialize_client(&format, &Direction::Capture, &mode) {
        log.line(&format!(
            "INITIALIZE: FAILED fmt={} autoconvert={} err={e:?}",
            fmt_waveformat(&format),
            opts.autoconvert
        ));
        return Err(anyhow::anyhow!("initialize_client failed: {e:?}"));
    }
    log.line(&format!(
        "INITIALIZE: ok fmt={} mode=EventsShared autoconvert={} buffer_duration_hns=0",
        fmt_waveformat(&format),
        opts.autoconvert
    ));

    let h_event = client
        .set_get_eventhandle()
        .map_err(|e| anyhow::anyhow!("set_get_eventhandle: {e:?}"))?;
    let capture = client
        .get_audiocaptureclient()
        .map_err(|e| anyhow::anyhow!("get_audiocaptureclient: {e:?}"))?;
    let blockalign = format.get_blockalign() as usize;

    // 4) WAV output.
    let wav_spec = hound::WavSpec {
        channels: opts.channels as u16,
        sample_rate: opts.rate as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&target.out, wav_spec)
        .with_context(|| format!("create {}", target.out.display()))?;

    // 5) Capture loop.
    if let Err(e) = client.start_stream() {
        log.line(&format!("START: FAILED err={e:?}"));
        return Err(anyhow::anyhow!("start_stream failed: {e:?}"));
    }
    log.line(&format!(
        "START: recording {:.3}s -> {}",
        opts.seconds,
        target.out.display()
    ));

    let wall_start = Instant::now();
    let deadline = wall_start + Duration::from_secs_f64(opts.seconds);
    let mut stats = CaptureStats::default();
    stats.rate_hz = opts.rate as f64;
    let mut buf: Vec<u8> = Vec::new();
    let mut next_hb = 1.0f64;
    loop {
        if Instant::now() >= deadline {
            break;
        }
        let now = wall_start.elapsed().as_secs_f64();
        if now >= next_hb {
            log.line(&format!(
                "HEARTBEAT t={:.1}s packets={} frames={} silent={} gaps={} empty_reads={}",
                now,
                stats.packets,
                stats.frames_written,
                stats.silent_packets,
                stats.gap_events,
                stats.empty_reads
            ));
            next_hb += 1.0;
        }
        // EventTimeout only means "no new data yet"; the deadline check above still holds.
        let _ = h_event.wait_for_event(100);
        loop {
            let n = match capture.get_next_packet_size() {
                Ok(Some(n)) => n,
                Ok(None) => break,
                Err(e) => {
                    log.line(&format!("CAPTURE: get_next_packet_size error {e:?}"));
                    break;
                }
            };
            if n == 0 {
                stats.empty_reads += 1;
                break;
            }
            let need = n as usize * blockalign;
            if buf.len() < need {
                buf.resize(need, 0);
            }
            let elapsed = wall_start.elapsed().as_secs_f64();
            let (frames, info) = match capture.read_from_device(&mut buf) {
                Ok(v) => v,
                Err(e) => {
                    log.line(&format!("CAPTURE: read_from_device error {e:?}"));
                    break;
                }
            };
            if frames == 0 {
                stats.empty_reads += 1;
                continue;
            }
            stats.observe(frames, &info, elapsed, log);
            if info.flags.silent {
                // Buffer contents are undefined when SILENT is set; write digital silence.
                for _ in 0..(frames as usize * opts.channels) {
                    writer.write_sample(0.0f32)?;
                }
            } else {
                for bytes in buf[..frames as usize * blockalign].chunks_exact(4) {
                    writer.write_sample(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))?;
                }
            }
            stats.frames_written += frames as u64;
        }
    }
    let wall_s = wall_start.elapsed().as_secs_f64();
    if let Err(e) = client.stop_stream() {
        log.line(&format!("STOP: stop_stream error {e:?}"));
    }

    // 6) Optional silence padding so the file keeps the requested duration.
    let target_frames = (opts.seconds * opts.rate as f64).round() as u64;
    if opts.pad_silence && stats.frames_written < target_frames {
        let pad = target_frames - stats.frames_written;
        for _ in 0..(pad as usize * opts.channels) {
            writer.write_sample(0.0f32)?;
        }
        stats.padded_frames = pad;
        log.line(&format!(
            "PAD: added {} frames ({:.3}s) of silence to keep the requested duration",
            pad,
            pad as f64 / opts.rate as f64
        ));
    }
    writer.finalize().context("finalize wav")?;
    if let Ok(m) = std::fs::metadata(&target.out) {
        log.line(&format!("WAV: {} bytes", m.len()));
    }
    stats.wall_s = wall_s;
    Ok(stats)
}

// -------------------------------------------------------------------- analyze

#[derive(Clone)]
struct AnalyzeOpts {
    input: PathBuf,
    freqs: Vec<f64>,
    ref_freq: Option<f64>,
    nfft: usize,
    threshold_db: f64,
    csv: Option<PathBuf>,
    block_s: f64,
}

fn parse_analyze(args: &[String]) -> Result<AnalyzeOpts> {
    let mut o = AnalyzeOpts {
        input: PathBuf::new(),
        freqs: vec![440.0, 880.0],
        ref_freq: None,
        nfft: 32768,
        threshold_db: -40.0,
        csv: None,
        block_s: 0.5,
    };
    let mut have_input = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" => {
                o.input = PathBuf::from(take_value(args, &mut i, "--input")?);
                have_input = true;
            }
            "--freqs" => {
                o.freqs = take_value(args, &mut i, "--freqs")?
                    .split(',')
                    .map(|s| s.trim().parse::<f64>())
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .context("bad --freqs")?;
            }
            "--ref" => o.ref_freq = Some(take_f64(args, &mut i, "--ref")?),
            "--nfft" => o.nfft = take_usize(args, &mut i, "--nfft")?,
            "--threshold-db" => o.threshold_db = take_f64(args, &mut i, "--threshold-db")?,
            "--csv" => o.csv = Some(PathBuf::from(take_value(args, &mut i, "--csv")?)),
            "--block-s" => o.block_s = take_f64(args, &mut i, "--block-s")?,
            other => bail!("unknown analyze option '{other}'\n{USAGE}"),
        }
        i += 1;
    }
    if !have_input {
        bail!("analyze requires --input <file.wav>");
    }
    if o.freqs.is_empty() {
        bail!("--freqs must not be empty");
    }
    Ok(o)
}

struct Spectrum {
    nfft: usize,
    bin_hz: f64,
    bins: Vec<f64>,
    frames: usize,
}

fn db_amp(a: f64) -> f64 {
    20.0 * a.max(1e-10).log10()
}

fn power_db(p: f64) -> f64 {
    10.0 * p.max(1e-30).log10()
}

fn floor_pow2(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    1usize << (usize::BITS - 1 - n.leading_zeros()) as usize
}

fn hann(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let x = std::f64::consts::TAU * i as f64 / n as f64;
            0.5 * (1.0 - x.cos())
        })
        .collect()
}

fn welch_power(
    mono: &[f64],
    rate: f64,
    nfft_req: usize,
    skip_start_s: f64,
    skip_end_s: f64,
) -> Result<Spectrum> {
    let s0 = ((skip_start_s * rate) as usize).min(mono.len());
    let s1 = mono
        .len()
        .saturating_sub((skip_end_s * rate) as usize)
        .max(s0);
    let data = &mono[s0..s1];
    let nfft = floor_pow2(nfft_req.min(data.len()));
    if nfft < 256 {
        bail!("not enough samples for FFT (nfft={nfft})");
    }
    let win = hann(nfft);
    let wsum: f64 = win.iter().sum();
    let mut acc = vec![0.0f64; nfft / 2 + 1];
    let mut frames = 0usize;
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(nfft);
    let mut buf: Vec<Complex<f64>> = vec![Complex::default(); nfft];
    let hop = nfft / 2;
    let mut pos = 0usize;
    while pos + nfft <= data.len() {
        for i in 0..nfft {
            buf[i] = Complex::new(data[pos + i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        for k in 0..=nfft / 2 {
            acc[k] += buf[k].norm_sqr() * 4.0 / (wsum * wsum);
        }
        frames += 1;
        pos += hop;
    }
    if frames == 0 {
        bail!("no complete FFT frame in analysis range");
    }
    for v in acc.iter_mut() {
        *v /= frames as f64;
    }
    Ok(Spectrum {
        nfft,
        bin_hz: rate / nfft as f64,
        bins: acc,
        frames,
    })
}

/// Peak level (dBFS) inside [f-half_bw, f+half_bw] and the frequency of that bin.
fn band_peak(s: &Spectrum, f: f64, half_bw_hz: f64) -> (f64, f64) {
    let k0 = (((f - half_bw_hz).max(0.0)) / s.bin_hz).floor() as usize;
    let k1 = ((((f + half_bw_hz) / s.bin_hz).ceil() as usize).min(s.bins.len() - 1)).max(k0);
    let mut best_k = k0;
    let mut best = f64::NEG_INFINITY;
    for k in k0..=k1 {
        let db = power_db(s.bins[k]);
        if db > best {
            best = db;
            best_k = k;
        }
    }
    (best, best_k as f64 * s.bin_hz)
}

fn noise_floor(s: &Spectrum, targets: &[f64], excl_hz: f64, lo_hz: f64, hi_hz: f64) -> f64 {
    let mut vals = Vec::new();
    for k in 1..s.bins.len() {
        let f = k as f64 * s.bin_hz;
        if f < lo_hz || f > hi_hz {
            continue;
        }
        if targets.iter().any(|t| (f - t).abs() <= excl_hz) {
            continue;
        }
        vals.push(s.bins[k]);
    }
    if vals.is_empty() {
        return f64::NEG_INFINITY;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    power_db(vals[vals.len() / 2])
}

fn global_peak(s: &Spectrum, lo_hz: f64, hi_hz: f64) -> (f64, f64) {
    let k0 = ((lo_hz / s.bin_hz).ceil() as usize).min(s.bins.len() - 1);
    let k1 = ((hi_hz / s.bin_hz).floor() as usize).min(s.bins.len() - 1);
    let mut best_k = k0;
    let mut best = f64::NEG_INFINITY;
    for k in k0..=k1 {
        let db = power_db(s.bins[k]);
        if db > best {
            best = db;
            best_k = k;
        }
    }
    (best_k as f64 * s.bin_hz, best)
}

fn cmd_analyze(args: &[String]) -> Result<()> {
    let o = parse_analyze(args)?;
    let mut reader =
        hound::WavReader::open(&o.input).with_context(|| format!("open {}", o.input.display()))?;
    let spec = reader.spec();
    println!(
        "BASIC file={} rate={} channels={} bits={} format={:?}",
        o.input.display(),
        spec.sample_rate,
        spec.channels,
        spec.bits_per_sample,
        spec.sample_format
    );
    if spec.bits_per_sample != 32 || spec.sample_format != hound::SampleFormat::Float {
        bail!(
            "expected 32-bit float WAV (spike recordings), got {:?} {} bits",
            spec.sample_format,
            spec.bits_per_sample
        );
    }
    let ch = spec.channels as usize;
    let raw: Vec<f32> = reader
        .samples::<f32>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("read samples")?;
    let frames = raw.len() / ch;
    let rate = spec.sample_rate as f64;
    println!(
        "BASIC frames={frames} duration_s={:.6}",
        frames as f64 / rate
    );
    let mono: Vec<f64> = if ch == 1 {
        raw.iter().map(|&s| s as f64).collect()
    } else {
        raw.chunks_exact(ch)
            .map(|c| c.iter().map(|&s| s as f64).sum::<f64>() / ch as f64)
            .collect()
    };
    let peak = mono.iter().fold(0.0f64, |m, &s| m.max(s.abs()));
    let rms = (mono.iter().map(|&s| s * s).sum::<f64>() / mono.len().max(1) as f64).sqrt();
    println!(
        "BASIC peak_dbfs={:.2} rms_dbfs={:.2}",
        db_amp(peak),
        db_amp(rms)
    );

    let sp = welch_power(&mono, rate, o.nfft, 0.25, 0.25)?;
    println!(
        "FFT nfft={} bin_hz={:.6} frames={} window=hann skip_start_s=0.25 skip_end_s=0.25",
        sp.nfft, sp.bin_hz, sp.frames
    );

    let mut levels: Vec<(f64, f64, f64)> = Vec::new(); // (freq, db, peak bin freq)
    for &f in &o.freqs {
        let (db, pf) = band_peak(&sp, f, 5.0);
        levels.push((f, db, pf));
    }
    let ref_idx = if let Some(rf) = o.ref_freq {
        levels
            .iter()
            .position(|l| (l.0 - rf).abs() < 1e-6)
            .with_context(|| format!("--ref {rf} not among --freqs"))?
    } else {
        levels
            .iter()
            .enumerate()
            .max_by(|a, b| a.1 .1.partial_cmp(&b.1 .1).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0)
    };
    let ref_f = levels[ref_idx].0;
    let ref_db = levels[ref_idx].1;
    for (f, db, pf) in &levels {
        println!(
            "TONE freq={:.1} peak_at={:.3} db_fs={:.2} db_rel_ref={:.2}",
            f,
            pf,
            db,
            db - ref_db
        );
    }
    let floor = noise_floor(&sp, &o.freqs, 10.0, 100.0, 6000.0);
    println!(
        "FLOOR band=100-6000Hz excl_targets_pm10Hz median_db_fs={:.2} median_db_rel_ref={:.2}",
        floor,
        floor - ref_db
    );
    let (gp_f, gp_db) = global_peak(&sp, 50.0, 20000.0);
    println!("GLOBAL_PEAK freq={:.3} db_fs={:.2}", gp_f, gp_db);

    let mut verdict = "PASS";
    for (f, db, _) in &levels {
        if (*f - ref_f).abs() > 1e-6 {
            let rel = db - ref_db;
            let ok = rel <= o.threshold_db;
            println!(
                "LEAK freq={:.1} rel_db={:.2} limit_db={:.2} {}",
                f,
                rel,
                o.threshold_db,
                if ok { "ok" } else { "VIOLATION" }
            );
            if !ok {
                verdict = "FAIL";
            }
        }
    }
    println!(
        "VERDICT ref={:.1} threshold_db={:.2} {verdict}",
        ref_f, o.threshold_db
    );

    if let Some(csv) = &o.csv {
        write_block_csv(csv, &mono, rate, &o.freqs, o.block_s)?;
        println!("CSV path={}", csv.display());
    }
    Ok(())
}

fn write_block_csv(
    path: &Path,
    mono: &[f64],
    rate: f64,
    freqs: &[f64],
    block_s: f64,
) -> Result<()> {
    let block = (block_s * rate).round() as usize;
    if block < 256 {
        bail!("--block-s too small");
    }
    let nfft = floor_pow2(block).max(256);
    let win = hann(nfft);
    let wsum: f64 = win.iter().sum();
    let bin_hz = rate / nfft as f64;
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(nfft);
    let mut buf: Vec<Complex<f64>> = vec![Complex::default(); nfft];
    let mut w = BufWriter::new(File::create(path).with_context(|| format!("create {}", path.display()))?);
    let cols: Vec<String> = freqs.iter().map(|f| format!("db_{f}")).collect();
    writeln!(w, "block,t_mid_s,rms_dbfs,{}", cols.join(","))?;
    for (bi, chunk) in mono.chunks(block).enumerate() {
        if chunk.len() < nfft {
            break;
        }
        for i in 0..nfft {
            buf[i] = Complex::new(chunk[i] * win[i], 0.0);
        }
        fft.process(&mut buf);
        let mut acc = vec![0.0f64; nfft / 2 + 1];
        for k in 0..=nfft / 2 {
            acc[k] = buf[k].norm_sqr() * 4.0 / (wsum * wsum);
        }
        let s = Spectrum {
            nfft,
            bin_hz,
            bins: acc,
            frames: 1,
        };
        let rms = (chunk.iter().map(|&x| x * x).sum::<f64>() / chunk.len() as f64).sqrt();
        let t_mid = (bi as f64 + 0.5) * block_s;
        let mut row = format!("{},{:.3},{:.2}", bi, t_mid, db_amp(rms));
        for f in freqs {
            let (db, _) = band_peak(&s, *f, 5.0);
            row.push_str(&format!(",{db:.2}"));
        }
        writeln!(w, "{row}")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------- probe

fn cmd_probe(_args: &[String]) -> Result<()> {
    let hr = initialize_mta();
    println!("COM: initialize_mta() hr={:#010x} ok={}", hr.0, hr.is_ok());
    let s = default_endpoint_formats()?;
    println!("ENDPOINT: {s}");
    Ok(())
}
