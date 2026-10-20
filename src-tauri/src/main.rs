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

fn add_to_zip<W: std::io::Write + std::io::Seek>(zw: &mut zip::ZipWriter<W>, path: &std::path::Path, name: &str) -> std::io::Result<()> {
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if path.is_dir() {
        zw.add_directory(format!("{name}/"), opts)?;
        for e in std::fs::read_dir(path)? {
            let e = e?;
            add_to_zip(zw, &e.path(), &format!("{name}/{}", e.file_name().to_string_lossy()))?;
        }
    } else {
        let big = path.metadata()?.len() >= 0xFFFF_FFFF;
        zw.start_file(name, opts.large_file(big))?;
        std::io::copy(&mut std::fs::File::open(path)?, zw)?;
    }
    Ok(())
}

#[tauri::command]
async fn zip_files(paths: Vec<String>, output: String) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let r = (|| {
            let mut zw = zip::ZipWriter::new(std::fs::File::create(&output)?);
            for p in &paths {
                let p = std::path::Path::new(p);
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                add_to_zip(&mut zw, p, &name)?;
            }
            zw.finish()?;
            std::fs::metadata(&output).map(|m| m.len())
        })();
        if r.is_err() {
            let _ = std::fs::remove_file(&output);
        }
        r.map_err(|e: std::io::Error| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// Raw bytes in/out for the Merge tool (pdf-lib runs in the webview).
#[tauri::command]
fn read_file(path: String) -> Result<tauri::ipc::Response, String> {
    std::fs::read(path).map(tauri::ipc::Response::new).map_err(|e| e.to_string())
}

#[tauri::command]
fn write_file(req: tauri::ipc::Request) -> Result<(), String> {
    let path = req.headers().get("path").and_then(|v| v.to_str().ok()).ok_or("missing path")?;
    let path = percent_decode(path);
    match req.body() {
        tauri::ipc::InvokeBody::Raw(b) => std::fs::write(path, b).map_err(|e| e.to_string()),
        _ => Err("expected raw body".into()),
    }
}

// headers are ASCII-only, so the frontend sends encodeURIComponent(path)
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// Audio tool: input -> sibling file with the new extension, never overwriting the input.
fn audio_args(input: &str, format: &str, kbps: u32) -> (String, Vec<String>) {
    let base = input.rsplit_once('.').filter(|(b, _)| !b.ends_with(['/', '\\'])).map_or(input, |(b, _)| b);
    let mut output = format!("{base}.{format}");
    if output == input {
        output = format!("{base}_audio.{format}");
    }
    let b = format!("{kbps}k");
    let codec: &[&str] = match format {
        "wav" => &["-c:a", "pcm_s16le"],
        "m4a" => &["-c:a", "aac", "-b:a", &b],
        _ => &["-c:a", "libmp3lame", "-b:a", &b],
    };
    let args = [&["-y", "-hide_banner", "-i", input, "-vn"][..], codec, &[&output]].concat().iter().map(|s| s.to_string()).collect();
    (output, args)
}

#[tauri::command]
async fn convert_audio(input: String, format: String, kbps: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (output, args) = audio_args(&input, &format, kbps);
        let out = ffmpeg().args(&args).output().map_err(|e| format!("ffmpeg not found: {e}"))?;
        if out.status.success() {
            return Ok(output);
        }
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().last().unwrap_or("ffmpeg failed").to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(Job::default())
        .invoke_handler(tauri::generate_handler![probe, compress, cancel, zip_files, read_file, write_file, convert_audio])
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

    // Runs every mode through real ffmpeg (needs ffmpeg on PATH): cargo test -- --ignored
    #[test]
    #[ignore]
    fn encodes_all_modes() {
        let dir = std::env::temp_dir().join("squeeze_test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.mp4").to_string_lossy().to_string();
        let ok = ffmpeg()
            .args(["-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=6", "-f", "lavfi", "-i", "sine=duration=6", "-c:v", "libx264", "-crf", "10", "-shortest", &src])
            .status().unwrap().success();
        assert!(ok);
        let src_size = probe(src.clone()).unwrap().size;
        for (mode, codec, ext) in [("crf", "h264", "mp4"), ("crf", "h265", "mp4"), ("target", "h264", "mp4"), ("target", "h265", "mp4"), ("lossless", "h264", "mkv")] {
            let out = dir.join(format!("{mode}_{codec}.{ext}")).to_string_lossy().to_string();
            let o = Opts { input: src.clone(), output: out.clone(), mode: mode.into(), codec: codec.into(), crf: 28, target_mb: 0.5, height: 480 };
            for args in build_passes(&o, 6.0) {
                let st = ffmpeg().args(&args).current_dir(&dir).output().unwrap();
                assert!(st.status.success(), "{mode} {codec}: {}", String::from_utf8_lossy(&st.stderr));
            }
            let i = probe(out).unwrap();
            assert_eq!(i.height, 480, "{mode} {codec}");
            if mode == "target" {
                let mb = i.size as f64 / 1048576.0;
                assert!((0.35..=0.6).contains(&mb), "{codec} target 0.5MB got {mb}");
            }
            if mode == "crf" { assert!(i.size < src_size); }
        }
    }
}

#[cfg(test)]
mod zip_tests {
    #[test]
    fn zips_files_and_folders() {
        let dir = std::env::temp_dir().join("all_zip_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder/sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "aaaa".repeat(1000)).unwrap();
        std::fs::write(dir.join("folder/sub/b.txt"), "b").unwrap();
        let out = dir.join("out.zip");
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
        super::add_to_zip(&mut zw, &dir.join("a.txt"), "a.txt").unwrap();
        super::add_to_zip(&mut zw, &dir.join("folder"), "folder").unwrap();
        zw.finish().unwrap();
        let z = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut names: Vec<_> = z.file_names().map(|n| n.unwrap().to_string()).collect();
        names.sort();
        assert_eq!(names, ["a.txt", "folder/", "folder/sub/", "folder/sub/b.txt"]);
        assert!(std::fs::metadata(&out).unwrap().len() < 4000);
    }
}

#[test]
fn decodes_percent_paths() {
    assert_eq!(percent_decode("C%3A%5Cm%C3%BCde%20x.pdf"), r"C:\müde x.pdf");
    assert_eq!(percent_decode("a%2"), "a%2");
}

#[test]
fn converts_audio() {
    let dir = std::env::temp_dir().join("all_audio_test");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.mp3").to_string_lossy().to_string();
    assert!(ffmpeg().args(["-y", "-f", "lavfi", "-i", "sine=duration=1", &src]).status().unwrap().success());
    for f in ["mp3", "wav", "m4a"] {
        let (out, args) = audio_args(&src, f, 128);
        assert_eq!(out, dir.join(if f == "mp3" { "a_audio.mp3".to_string() } else { format!("a.{f}") }).to_string_lossy());
        assert!(ffmpeg().args(&args).status().unwrap().success(), "{f}");
    }
}
