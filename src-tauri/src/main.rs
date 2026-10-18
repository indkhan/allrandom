#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
struct Job {
    child: Mutex<Option<Child>>,
    cancelled: AtomicBool,
}

#[derive(Serialize)]
struct Info {
    duration: f64,
    width: u32,
    height: u32,
    size: u64,
    codec: String,
}

#[derive(Deserialize)]
struct Opts {
    input: String,
    output: String,
    mode: String, // "crf" | "target" | "lossless"
    codec: String, // "h264" | "h265"
    crf: u32,
    target_mb: f64,
    height: u32, // 0 = keep
}

// Bundled sidecar sits next to our exe; fall back to ffmpeg on PATH.
fn ffmpeg() -> Command {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let side = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name)));
    let mut cmd = Command::new(side.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("ffmpeg")));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn parse_info(stderr: &str, size: u64) -> Option<Info> {
    let d = stderr.split("Duration: ").nth(1)?.split(',').next()?;
    let mut duration = 0.0;
    for part in d.trim().split(':') {
        duration = duration * 60.0 + part.parse::<f64>().ok()?;
    }
    let video = stderr.lines().find(|l| l.contains("Video: "))?;
    let after = video.split("Video: ").nth(1)?;
    let codec = after.split(|c: char| c == ' ' || c == ',').next()?.to_string();
    let (width, height) = after
        .split(|c: char| c == ' ' || c == ',')
        .find_map(|t| {
            let (w, h) = t.split_once('x')?;
            let (w, h): (u32, u32) = (w.parse().ok()?, h.parse().ok()?);
            (w > 0 && h > 0).then_some((w, h)) // skips hex tags like 0x31637661
        })?;
    Some(Info { duration, width, height, size, codec })
}

#[tauri::command]
fn probe(path: String) -> Result<Info, String> {
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    let out = ffmpeg().args(["-hide_banner", "-i", &path]).output().map_err(|e| format!("ffmpeg not found: {e}"))?;
    parse_info(&String::from_utf8_lossy(&out.stderr), size).ok_or_else(|| "Not a readable video file".into())
}

fn build_passes(o: &Opts, duration: f64) -> Vec<Vec<String>> {
    let enc = if o.codec == "h265" { "libx265" } else { "libx264" };
    let mut base: Vec<String> = ["-y", "-hide_banner", "-nostats", "-progress", "pipe:1", "-i", &o.input]
        .map(String::from)
        .to_vec();
    if o.height > 0 {
        base.extend(["-vf".into(), format!("scale=-2:min({}\\,ih)", o.height)]);
    }
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let mut tail = if enc == "libx265" { s(&["-tag:v", "hvc1"]) } else { vec![] };
    tail.extend(s(&["-pix_fmt", "yuv420p", "-movflags", "+faststart"]));

    match o.mode.as_str() {
        "lossless" => vec![[base, s(&["-c:v", "libx264", "-qp", "0", "-preset", "veryfast", "-c:a", "copy", &o.output])].concat()],
        "target" => {
            // total kbit budget over the clip, minus audio; 8388.608 = 1 MiB in kbit
            let total = o.target_mb * 8388.608 / duration.max(0.1);
            let audio = if total < 400.0 { 64.0 } else { 128.0 };
            let v = format!("{}k", (total - audio).max(50.0) as u64);
            let pass = |n: u32| {
                if enc == "libx265" {
                    s(&["-c:v", enc, "-b:v", &v, "-x265-params", &format!("pass={n}:stats=squeeze_pass.log")])
                } else {
                    s(&["-c:v", enc, "-b:v", &v, "-pass", &n.to_string(), "-passlogfile", "squeeze_pass"])
                }
            };
            let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
            vec![
                [base.clone(), pass(1), s(&["-an", "-f", "null", null])].concat(),
                [base, pass(2), tail, s(&["-c:a", "aac", "-b:a", &format!("{}k", audio as u32), &o.output])].concat(),
            ]
        }
        _ => vec![[
            base,
            s(&["-c:v", enc, "-crf", &o.crf.to_string(), "-preset", "medium"]),
            tail,
            s(&["-c:a", "aac", "-b:a", "128k", &o.output]),
        ]
        .concat()],
    }
}

fn run(app: &AppHandle, job: &Job, o: &Opts) -> Result<String, String> {
    let dur = probe(o.input.clone())?.duration.max(0.1);
    let passes = build_passes(o, dur);
    let n = passes.len() as f64;
    let workdir = std::env::temp_dir(); // two-pass stats files land here
    for (i, args) in passes.iter().enumerate() {
        if job.cancelled.load(Ordering::SeqCst) {
            return Err("Cancelled".into());
        }
        let mut child = ffmpeg()
            .args(args)
            .current_dir(&workdir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        // drain stderr so ffmpeg never blocks on a full pipe; keep the tail for errors
        let err_thread = std::thread::spawn(move || {
            let mut tail: Vec<String> = BufReader::new(stderr).lines().map_while(Result::ok).collect();
            tail.split_off(tail.len().saturating_sub(4)).join("\n")
        });
        *job.child.lock().unwrap() = Some(child);
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.parse::<f64>().ok()) {
                let p = (i as f64 + (us / 1e6 / dur).clamp(0.0, 1.0)) / n;
                let _ = app.emit("progress", p);
            }
        }
        let status = job.child.lock().unwrap().take().map(|mut c| c.wait());
        let err = err_thread.join().unwrap_or_default();
        if job.cancelled.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&o.output);
            return Err("Cancelled".into());
        }
        match status {
            Some(Ok(s)) if s.success() => {}
            _ => return Err(if err.is_empty() { "ffmpeg failed".into() } else { err }),
        }
    }
    let _ = app.emit("progress", 1.0);
    Ok(o.output.clone())
}

#[tauri::command]
async fn compress(app: AppHandle, opts: Opts) -> Result<String, String> {
    if opts.input == opts.output {
        return Err("Output must differ from input".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let job = app.state::<Job>();
        job.cancelled.store(false, Ordering::SeqCst);
        run(&app, &job, &opts)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn cancel(job: State<Job>) {
    job.cancelled.store(true, Ordering::SeqCst);
    if let Some(c) = job.child.lock().unwrap().as_mut() {
        let _ = c.kill();
    }
}

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Job::default())
        .invoke_handler(tauri::generate_handler![probe, compress, cancel])
        .run(tauri::generate_context!())
        .expect("failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ffmpeg_banner() {
        let s = "  Duration: 00:01:02.50, start: 0.000000, bitrate: 1205 kb/s\n    Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 1070 kb/s";
        let i = parse_info(s, 10).unwrap();
        assert_eq!((i.width, i.height, i.codec.as_str()), (1920, 1080, "h264"));
        assert!((i.duration - 62.5).abs() < 1e-9);
    }
}
