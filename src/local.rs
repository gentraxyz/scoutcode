//! Local models on this PC: llama.cpp's llama-server, which Scout downloads, starts and stops itself. It speaks OpenAI's
//! API, so turns go through `api::stream_chat` like any other provider. See "Local models" in docs/providers.md.

use crate::api::{ChatRequest, Content, Message, ModelInfo};
use crate::config::{Local, LocalModel, home};
use crate::waker::Waker;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What `Config::provider` says when a model runs here.
pub const PROVIDER: &str = "local";
/// The llama.cpp build Scout runs. It only changes with a Scout release, and the hashes below are GitHub's for it.
pub const BUILD: &str = "b12110";
const RELEASE_URL: &str = "https://github.com/ggml-org/llama.cpp/releases/download";

pub struct Archive {
    pub file: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

/// One way to run llama.cpp on this platform: the GPU API it's built for, and the archives it needs.
pub struct Backend {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    os: &'static str,
    arch: &'static str,
    pub archives: &'static [Archive],
}

const fn a(file: &'static str, sha256: &'static str, size: u64) -> Archive {
    Archive { file, sha256, size }
}

/// Every build Scout knows of. `backends` picks this platform's, best first.
static ALL_BACKENDS: &[Backend] = &[
    Backend { id: "vulkan", name: "Vulkan", about: "Works on most NVIDIA, AMD and Intel GPUs.", os: "windows", arch: "x86_64", archives: &[a("llama-b12110-bin-win-vulkan-x64.zip", "9732a5e48887ca54d0bfe4af4324dc85fd56cd8cda19b52df3542fa6ebe6874f", 37661146)] },
    Backend { id: "cuda", name: "CUDA", about: "Fastest on NVIDIA GPUs. Needs a recent NVIDIA driver, and is a bigger download.", os: "windows", arch: "x86_64", archives: &[a("llama-b12110-bin-win-cuda-12.4-x64.zip", "1ce755bc3a061889234048f88e33328e17971eaad73c4666cfeb26026b317fee", 287014657), a("cudart-llama-bin-win-cuda-12.4-x64.zip", "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6", 391443627)] },
    Backend { id: "cpu", name: "CPU only", about: "No GPU. Slower, but works everywhere.", os: "windows", arch: "x86_64", archives: &[a("llama-b12110-bin-win-cpu-x64.zip", "98901e327c252875accaf2d1bf364e10e868843b0b63468baad1aa945cdc5da2", 24708445)] },
    Backend { id: "opencl", name: "Adreno (OpenCL)", about: "Built for the Adreno GPU in Snapdragon PCs.", os: "windows", arch: "aarch64", archives: &[a("llama-b12110-bin-win-opencl-adreno-arm64.zip", "fac3826972eeb5f0efa30dbda0bad62c559f91e3c69916d2c1bf599621bd5659", 14200332)] },
    Backend { id: "vulkan", name: "Vulkan", about: "Uses the GPU through Vulkan.", os: "windows", arch: "aarch64", archives: &[a("llama-b12110-bin-win-vulkan-arm64.zip", "2be672cf538aabfc9fcb22c21648422b01d623673f15c2742bd43a9bd1a11ce2", 25728494)] },
    Backend { id: "cpu", name: "CPU only", about: "No GPU. Slower, but works everywhere.", os: "windows", arch: "aarch64", archives: &[a("llama-b12110-bin-win-cpu-arm64.zip", "f8d20fb07d0b499988ced8963afcd5ded2d2cdd9983b757fbc7e51c4abd722e8", 13197637)] },
    Backend { id: "metal", name: "Metal", about: "Uses the Mac's GPU.", os: "macos", arch: "aarch64", archives: &[a("llama-b12110-bin-macos-arm64.tar.gz", "4f5b8de12e0b9516658ca6a5764c6ffb3d7c659c3dd538556b1413c7b4050662", 16650691)] },
    Backend { id: "metal", name: "Metal", about: "Uses the Mac's GPU.", os: "macos", arch: "x86_64", archives: &[a("llama-b12110-bin-macos-x64.tar.gz", "c8094268b6bf62f4998f0e45656fd9f5e2d4afb6b1cd0044d761db64be1dd47d", 12388601)] },
    Backend { id: "vulkan", name: "Vulkan", about: "Works on most NVIDIA, AMD and Intel GPUs.", os: "linux", arch: "x86_64", archives: &[a("llama-b12110-bin-ubuntu-vulkan-x64.tar.gz", "a5d741940956ec62e8783aee7e5a4c7c761af867d6f027c3277a51e1a3ac59b0", 35858360)] },
    Backend { id: "cuda", name: "CUDA", about: "Fastest on NVIDIA GPUs. Needs a recent NVIDIA driver, and is a bigger download.", os: "linux", arch: "x86_64", archives: &[a("llama-b12110-bin-ubuntu-cuda-12.8-x64.tar.gz", "4af1e1da23661db4a5b84b29aae16ebc053df689c5b8b1427c4670f2c6406d8c", 187314621), a("cudart-llama-b12110-bin-ubuntu-cuda-12.8-x64.tar.gz", "6f1d562086d84ae85df880d3711079ec225602b51d530608e5001600625067dd", 594371477)] },
    Backend { id: "cpu", name: "CPU only", about: "No GPU. Slower, but works everywhere.", os: "linux", arch: "x86_64", archives: &[a("llama-b12110-bin-ubuntu-x64.tar.gz", "368227edcc229be5f9bed16a2223d79b5e41d0ac2e19bcf5825e82291c060e55", 23051958)] },
    Backend { id: "vulkan", name: "Vulkan", about: "Uses the GPU through Vulkan.", os: "linux", arch: "aarch64", archives: &[a("llama-b12110-bin-ubuntu-vulkan-arm64.tar.gz", "1c68bf23d35230cc607e3d30f8d447d21fef500d9fa8d65e15520dfa3619ad73", 29168359)] },
    Backend { id: "cpu", name: "CPU only", about: "No GPU. Slower, but works everywhere.", os: "linux", arch: "aarch64", archives: &[a("llama-b12110-bin-ubuntu-arm64.tar.gz", "4758b51335e37b4346251f2687d64841ff1c07ecb27fc46323945bb2ec9c44d2", 18723578)] },
];

/// This platform's builds, best first. Empty where llama.cpp publishes none.
pub fn backends() -> Vec<&'static Backend> {
    ordered(std::env::consts::OS, std::env::consts::ARCH, has_nvidia())
}

/// A platform's builds, best first: the table's order, with CUDA ahead of the rest on a PC with an NVIDIA driver.
fn ordered(os: &str, arch: &str, nvidia: bool) -> Vec<&'static Backend> {
    let mut list: Vec<_> = ALL_BACKENDS.iter().filter(|b| b.os == os && b.arch == arch).collect();
    if nvidia {
        list.sort_by_key(|b| b.id != "cuda");
    }
    list
}

/// Whether this PC has NVIDIA's driver, which the CUDA build needs: its CUDA library on Windows, its entry in /proc on
/// Linux.
fn has_nvidia() -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| {
        if cfg!(windows) {
            std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("System32").join("nvcuda.dll").is_file()
        } else {
            Path::new("/proc/driver/nvidia/version").exists()
        }
    })
}

fn backend(id: &str) -> Option<&'static Backend> {
    backends().into_iter().find(|b| b.id == id)
}

/// A model Scout offers to download: tool-capable and Apache-2.0, as GGUF builds on Hugging Face.
pub struct Curated {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    repo: &'static str,
    file: &'static str,
    sha256: &'static str,
    pub size: u64,
}

pub static CURATED: [Curated; 6] = [
    Curated { id: "qwen3.5-4b", name: "Qwen3.5 4B", about: "Small and quick. Runs on most PCs, even without a GPU. About 4 GB of memory.", repo: "unsloth/Qwen3.5-4B-GGUF", file: "Qwen3.5-4B-Q4_K_M.gguf", sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4", size: 2740937888 },
    Curated { id: "qwen3.5-9b", name: "Qwen3.5 9B", about: "A good balance for 8 GB GPUs. About 7 GB of memory.", repo: "unsloth/Qwen3.5-9B-GGUF", file: "Qwen3.5-9B-Q4_K_M.gguf", sha256: "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8", size: 5680522464 },
    Curated { id: "omnicoder-9b", name: "OmniCoder 9B", about: "Tesslate's coding model: Qwen3.5 9B trained further on agentic coding. About 7 GB of memory.", repo: "Tesslate/OmniCoder-9B-GGUF", file: "omnicoder-9b-q4_k_m.gguf", sha256: "550e8f7253c8e07997fbce2570d37259b69b0d21faf77e5ed518d4ee4c73d8b3", size: 5737242944 },
    Curated { id: "gemma-4-12b", name: "Gemma 4 12B", about: "Google's mid-size model. About 9 GB of memory.", repo: "unsloth/gemma-4-12b-it-GGUF", file: "gemma-4-12b-it-Q4_K_M.gguf", sha256: "0a270ec9fe6b34f4a0d33992b6135117b484ebc4766ab76b51d4ae8c457e4c42", size: 7121861440 },
    Curated { id: "qwen3.8-27b", name: "Qwen3.8 27B", about: "The strongest coder here. Best on a 24 GB GPU. About 19 GB of memory.", repo: "unsloth/Qwen3.8-27B-GGUF", file: "Qwen3.8-27B-UD-Q4_K_M.gguf", sha256: "322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482", size: 16464440224 },
    Curated { id: "qwen3.6-35b-a3b", name: "Qwen3.6 35B A3B", about: "Mixture of experts: only 3B of it works at a time, so it stays quick with part of it on the CPU. About 24 GB of memory.", repo: "unsloth/Qwen3.6-35B-A3B-GGUF", file: "Qwen3.6-35B-A3B-UD-Q4_K_M.gguf", sha256: "ac0e2c1189e055faa36eff361580e79c5bd6f8e76bffb4ce547f167d53e31a61", size: 22134528992 },
];

pub fn curated(id: &str) -> Option<&'static Curated> {
    CURATED.iter().find(|c| c.id == id)
}

// ---------- tool presets ----------

/// The tool presets, smallest first: what a local model is offered.
pub const PRESETS: [&str; 4] = ["none", "essential", "standard", "full"];
const ESSENTIAL: [&str; 9] = ["view", "list_dir", "glob", "grep", "edit_file", "apply_patch", "write_file", "bash", "present_plan"];
const STANDARD: [&str; 16] = ["view", "list_dir", "glob", "grep", "edit_file", "apply_patch", "write_file", "bash", "present_plan", "multi_edit", "shell_output", "kill_shell", "web_fetch", "web_search", "todo_write", "ask_questions"];

/// Scout's own tools a preset offers, or None for every tool (with the user's MCP servers').
pub fn tool_names(preset: &str) -> Option<&'static [&'static str]> {
    match preset {
        "none" => Some(&[]),
        "standard" => Some(&STANDARD),
        "full" => None,
        _ => Some(&ESSENTIAL),
    }
}

// ---------- where things are ----------

fn runtime_root() -> PathBuf {
    home().join("runtime").join("llama")
}

fn runtime_dir(backend: &str) -> PathBuf {
    runtime_root().join(format!("{BUILD}-{backend}"))
}

pub fn models_dir() -> PathBuf {
    home().join("models")
}

const EXE: &str = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };

/// llama-server in an installed build. Archives put it at the top or one folder down, so both are looked in.
fn server_exe(backend: &str) -> Option<PathBuf> {
    let dir = runtime_dir(backend);
    let top = dir.join(EXE);
    if top.is_file() {
        return Some(top);
    }
    std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path().join(EXE)).find(|p| p.is_file())
}

/// The build to run: the chosen one, or the first that's installed, or the best for this platform.
pub fn backend_for(local: &Local) -> Option<&'static str> {
    let list = backends();
    if let Some(b) = local.backend.as_deref().and_then(backend) {
        return Some(b.id);
    }
    let installed = installed_backends();
    list.iter().find(|b| installed.contains(&b.id.to_string())).or(list.first()).map(|b| b.id)
}

// ---------- state the window shows ----------

struct Progress {
    done: u64,
    total: u64,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    /// The build being installed, and how far.
    install: Option<(String, Progress)>,
    install_error: Option<String>,
    /// Which builds are installed. None until looked up, and again after an install or removal.
    installed: Option<Vec<String>>,
    downloads: HashMap<String, Progress>,
    download_errors: HashMap<String, String>,
    server: ServerView,
    /// When the window was last woken for progress, so a download doesn't wake it for every chunk.
    woke: Option<Instant>,
}

#[derive(Default, Clone)]
struct ServerView {
    /// "stopped", "loading", "ready" or "error".
    state: &'static str,
    model: Option<String>,
    error: Option<String>,
    /// How many of the model's layers llama.cpp put on the GPU, of how many, from its log.
    offloaded: Option<(u32, u32)>,
    /// It runs on the CPU because that measured faster than this PC's GPU (see `ensure`).
    cpu_faster: bool,
    /// The model's experts, from its log: more than 0 for a mixture of experts.
    experts: u32,
    /// MiB of the model's weights llama.cpp kept in the computer's memory, and on GPUs, from its log.
    cpu_mib: f64,
    gpu_mib: f64,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static WAKER: OnceLock<Waker> = OnceLock::new();

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.get_or_init(|| Mutex::new(State { server: ServerView { state: "stopped", ..Default::default() }, ..Default::default() })).lock().unwrap_or_else(|e| e.into_inner())
}

/// Lets downloads and the server tell the window something changed.
pub fn set_waker(w: Waker) {
    let _ = WAKER.set(w);
}

fn wake() {
    if let Some(w) = WAKER.get() {
        w.request_repaint();
    }
}

/// Wakes the window for progress at most five times a second.
fn wake_for_progress(s: &mut State) {
    if s.woke.is_none_or(|t| t.elapsed() > Duration::from_millis(200)) {
        s.woke = Some(Instant::now());
        wake();
    }
}

fn installed_backends() -> Vec<String> {
    let mut s = state();
    if s.installed.is_none() {
        s.installed = Some(backends().iter().filter(|b| server_exe(b.id).is_some()).map(|b| b.id.to_string()).collect());
    }
    s.installed.clone().unwrap_or_default()
}

/// Everything Settings and the model menu draw from.
pub fn view(local: &Local) -> Value {
    let installed = installed_backends();
    let chosen = backend_for(local);
    let s = state();
    let prog = |p: &Progress| json!({ "done": p.done, "total": p.total });
    json!({
        "build": BUILD,
        "backend": chosen,
        "backends": backends().iter().map(|b| json!({ "id": b.id, "name": b.name, "about": b.about, "size": b.archives.iter().map(|a| a.size).sum::<u64>(), "installed": installed.iter().any(|i| i == b.id) })).collect::<Vec<_>>(),
        "installing": s.install.as_ref().map(|(b, p)| json!({ "backend": b, "done": p.done, "total": p.total })),
        "installError": s.install_error,
        "curated": CURATED.iter().map(|c| json!({ "id": c.id, "name": c.name, "about": c.about, "size": c.size, "have": local.models.iter().any(|m| m.id == c.id) })).collect::<Vec<_>>(),
        "downloads": s.downloads.iter().map(|(k, p)| (k.clone(), prog(p))).collect::<serde_json::Map<_, _>>(),
        "downloadErrors": s.download_errors,
        "server": { "state": s.server.state, "model": s.server.model, "error": s.server.error, "offloaded": s.server.offloaded.map(|(a, b)| [a, b]), "cpuFaster": s.server.cpu_faster },
    })
}

// ---------- downloads ----------

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// How much of a file one request fetches. Each is a fresh, short connection: Hugging Face's CDN gives one long download
/// a quarter of the line's speed, and a chunk that fails is all that's fetched again.
const SEGMENT: u64 = 32 * 1024 * 1024;
/// How many chunks download at once.
const WORKERS: usize = 4;

/// Downloads `url` to `dest` in chunks, several at a time, carrying on from what an earlier try left (`.part`, with the
/// finished chunks listed in `.part.done`). The file only gets its name once it's all there and matches `sha256`.
/// `progress` hears the bytes so far. A server that doesn't do ranges gets one download from the start.
pub fn download(url: &str, dest: &Path, sha256: &str, size: u64, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<(), String> {
    download_with(url, dest, sha256, size, SEGMENT, WORKERS, cancel, progress)
}

enum ChunkError {
    /// The server sent the whole file instead.
    NoRanges,
    Cancelled,
    /// Why, and how many bytes of the chunk came before it.
    Failed(String, u64),
}

/// Fetches bytes `start..=end` into `part` at the same place.
fn fetch_chunk(url: &str, part: &Path, start: u64, end: u64, cancel: &AtomicBool, have: &std::sync::atomic::AtomicU64) -> Result<(), ChunkError> {
    use std::io::{Seek, SeekFrom};
    let res = crate::api::http().get(url).header("Range", &format!("bytes={start}-{end}")).call().map_err(|e| ChunkError::Failed(e.to_string(), 0))?;
    match res.status().as_u16() {
        206 => {}
        200 => return Err(ChunkError::NoRanges),
        s => return Err(ChunkError::Failed(format!("HTTP {s}"), 0)),
    }
    let io = |e: std::io::Error| ChunkError::Failed(e.to_string(), 0);
    let mut file = std::fs::OpenOptions::new().write(true).open(part).map_err(io)?;
    file.seek(SeekFrom::Start(start)).map_err(io)?;
    let (mut body, mut buf, mut got) = (res.into_body().into_reader(), vec![0u8; 256 * 1024], 0u64);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ChunkError::Cancelled);
        }
        let n = body.read(&mut buf).map_err(|e| ChunkError::Failed(e.to_string(), got))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| ChunkError::Failed(format!("couldn't save it: {e}"), got))?;
        got += n as u64;
        have.fetch_add(n as u64, Ordering::Relaxed);
    }
    if got != end - start + 1 {
        return Err(ChunkError::Failed("it stopped early".into(), got));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn download_with(url: &str, dest: &Path, sha256: &str, size: u64, segment: u64, workers: usize, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<(), String> {
    use std::sync::atomic::AtomicU64;
    let name = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if dest.metadata().is_ok_and(|m| m.len() == size) {
        return Ok(());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let part = dest.with_file_name(format!("{name}.part"));
    let done_file = dest.with_file_name(format!("{name}.part.done"));
    let count = size.div_ceil(segment).max(1) as usize;
    let len = |i: usize| (size.min((i as u64 + 1) * segment)).saturating_sub(i as u64 * segment);
    let mut done = vec![false; count];
    let part_len = part.metadata().map(|m| m.len()).unwrap_or(0);
    if part_len > size {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&done_file);
    } else if let Ok(list) = std::fs::read_to_string(&done_file) {
        for i in list.lines().filter_map(|l| l.trim().parse::<usize>().ok()).filter(|i| *i < count) {
            done[i] = true;
        }
    } else if part_len > 0 {
        // An older download, all from the start: the chunks it covers are done.
        for (i, d) in done.iter_mut().enumerate() {
            *d = i as u64 * segment + len(i) <= part_len;
        }
    }
    let io = |e: std::io::Error| format!("Couldn't save {name}: {e}");
    std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&part).and_then(|f| f.set_len(size)).map_err(io)?;
    let list: String = done.iter().enumerate().filter(|(_, d)| **d).map(|(i, _)| format!("{i}\n")).collect();
    std::fs::write(&done_file, list).map_err(io)?;
    let log = Mutex::new(std::fs::OpenOptions::new().append(true).open(&done_file).map_err(io)?);
    let have = AtomicU64::new((0..count).filter(|i| done[*i]).map(len).sum());
    let queue = Mutex::new((0..count).filter(|i| !done[*i]).collect::<VecDeque<_>>());
    let (failed, no_ranges) = (Mutex::new(None::<String>), AtomicBool::new(false));
    progress(have.load(Ordering::Relaxed));
    std::thread::scope(|s| {
        let running: Vec<_> = (0..workers.max(1))
            .map(|_| {
                s.spawn(|| {
                    while !cancel.load(Ordering::Relaxed) && !no_ranges.load(Ordering::Relaxed) && failed.lock().unwrap().is_none() {
                        let Some(i) = queue.lock().unwrap().pop_front() else { break };
                        let start = i as u64 * segment;
                        for tries in 1.. {
                            match fetch_chunk(url, &part, start, start + len(i) - 1, cancel, &have) {
                                Ok(()) => {
                                    let _ = writeln!(log.lock().unwrap(), "{i}");
                                    break;
                                }
                                Err(ChunkError::NoRanges) => no_ranges.store(true, Ordering::Relaxed),
                                Err(ChunkError::Cancelled) => {}
                                Err(ChunkError::Failed(e, got)) => {
                                    have.fetch_sub(got, Ordering::Relaxed);
                                    if tries < 3 {
                                        std::thread::sleep(Duration::from_secs(tries));
                                        continue;
                                    }
                                    failed.lock().unwrap().get_or_insert(e);
                                }
                            }
                            break;
                        }
                    }
                })
            })
            .collect();
        while !running.iter().all(|t| t.is_finished()) {
            progress(have.load(Ordering::Relaxed));
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    if no_ranges.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&done_file);
        return download_one(url, dest, sha256, size, cancel, progress);
    }
    if let Some(e) = failed.into_inner().unwrap() {
        return Err(format!("The download of {name} stopped: {e}. Start it again to carry on."));
    }
    progress(size);
    // The chunks came in any order, so the hash is over the finished file.
    let mut hash = Sha256::new();
    let (mut f, mut buf) = (std::fs::File::open(&part).map_err(io)?, vec![0u8; 1024 * 1024]);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let n = f.read(&mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    drop(f);
    if hex(hash.finalize()) != sha256 {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&done_file);
        return Err(format!("{name} didn't match its expected hash, so it was deleted. Try the download again."));
    }
    let _ = std::fs::remove_file(&done_file);
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

/// Downloads `url` to `dest` in one go, carrying on from a `.part` file an earlier try left. For servers without ranges.
fn download_one(url: &str, dest: &Path, sha256: &str, size: u64, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<(), String> {
    let name = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if dest.metadata().is_ok_and(|m| m.len() == size) {
        return Ok(());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let part = dest.with_file_name(format!("{name}.part"));
    let mut have = part.metadata().map(|m| m.len()).unwrap_or(0);
    if have > size {
        let _ = std::fs::remove_file(&part);
        have = 0;
    }
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    // What's already there goes into the hash first.
    if have > 0 {
        let mut f = std::fs::File::open(&part).map_err(|e| e.to_string())?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
        progress(have);
    }
    if have < size {
        let mut req = crate::api::http().get(url);
        if have > 0 {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        let res = req.call().map_err(|e| format!("Couldn't download {name}: {e}"))?;
        let append = match res.status().as_u16() {
            206 if have > 0 => true,
            // The server sent all of it, so it starts again.
            200 => {
                have = 0;
                hash = Sha256::new();
                false
            }
            s => return Err(format!("Couldn't download {name}: HTTP {s}")),
        };
        let mut file = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&part).map_err(|e| e.to_string())?;
        let mut body = res.into_body().into_reader();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            let n = body.read(&mut buf).map_err(|e| format!("The download of {name} stopped: {e}. Start it again to carry on."))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| format!("Couldn't save {name}: {e}"))?;
            hash.update(&buf[..n]);
            have += n as u64;
            progress(have);
        }
    }
    if have != size {
        return Err(format!("The download of {name} stopped early. Start it again to carry on."));
    }
    if hex(hash.finalize()) != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{name} didn't match its expected hash, so it was deleted. Try the download again."));
    }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

const CANCELLED: &str = "Cancelled.";

/// Downloads and unpacks a llama.cpp build in the background. Older builds are deleted once it's in.
pub fn start_install(backend_id: &str) {
    let Some(b) = backend(backend_id) else { return };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut s = state();
        if s.install.is_some() {
            return;
        }
        let total = b.archives.iter().map(|a| a.size).sum();
        s.install = Some((b.id.to_string(), Progress { done: 0, total, cancel: cancel.clone() }));
        s.install_error = None;
    }
    wake();
    std::thread::spawn(move || {
        let r = install(b, &cancel);
        let mut s = state();
        s.install = None;
        s.installed = None;
        s.install_error = r.err().filter(|e| e != CANCELLED);
        drop(s);
        wake();
    });
}

fn install(b: &Backend, cancel: &AtomicBool) -> Result<(), String> {
    let root = runtime_root();
    let tmp = root.join(format!("{BUILD}-{}.partial", b.id));
    let _ = std::fs::remove_dir_all(&tmp);
    let mut before = 0;
    for a in b.archives {
        let file = root.join(a.file);
        download(&format!("{RELEASE_URL}/{BUILD}/{}", a.file), &file, a.sha256, a.size, cancel, &mut |n| {
            let mut s = state();
            if let Some((_, p)) = &mut s.install {
                p.done = before + n;
            }
            wake_for_progress(&mut s);
        })?;
        crate::devin::unpack(&file, a.file, &tmp).map_err(|e| format!("Couldn't unpack llama.cpp: {e}"))?;
        before += a.size;
    }
    let dir = runtime_dir(b.id);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&tmp, &dir).map_err(|e| e.to_string())?;
    // The archives and other builds' folders aren't needed any more.
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if b.archives.iter().any(|a| a.file == n) || (!n.starts_with(&format!("{BUILD}-")) && e.path().is_dir()) {
            let _ = if e.path().is_dir() { std::fs::remove_dir_all(e.path()) } else { std::fs::remove_file(e.path()) };
        }
    }
    Ok(())
}

pub fn cancel_install() {
    if let Some((_, p)) = &state().install {
        p.cancel.store(true, Ordering::Relaxed);
    }
}

/// Deletes an installed build.
pub fn remove_backend(backend_id: &str) {
    if server_backend().as_deref() == Some(backend_id) {
        stop();
    }
    let _ = std::fs::remove_dir_all(runtime_dir(backend_id));
    state().installed = None;
    wake();
}

/// Downloads one of the curated models in the background, and hands it to `done` once it's all there.
pub fn start_download(id: &str, done: impl FnOnce(LocalModel) + Send + 'static) {
    let Some(c) = curated(id) else { return };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut s = state();
        if s.downloads.contains_key(c.id) {
            return;
        }
        s.downloads.insert(c.id.to_string(), Progress { done: 0, total: c.size, cancel: cancel.clone() });
        s.download_errors.remove(c.id);
    }
    wake();
    std::thread::spawn(move || {
        let dest = models_dir().join(c.file);
        let url = format!("https://huggingface.co/{}/resolve/main/{}", c.repo, c.file);
        let r = download(&url, &dest, c.sha256, c.size, &cancel, &mut |n| {
            let mut s = state();
            if let Some(p) = s.downloads.get_mut(c.id) {
                p.done = n;
            }
            wake_for_progress(&mut s);
        });
        {
            let mut s = state();
            s.downloads.remove(c.id);
            if let Err(e) = &r {
                if e != CANCELLED {
                    s.download_errors.insert(c.id.to_string(), e.clone());
                }
            }
        }
        if r.is_ok() {
            done(LocalModel { id: c.id.into(), name: c.name.into(), path: dest.to_string_lossy().into(), size: c.size });
        }
        wake();
    });
}

/// Stops a download. What's downloaded stays, so starting it again carries on.
pub fn cancel_download(id: &str) {
    let mut s = state();
    if let Some(p) = s.downloads.remove(id) {
        p.cancel.store(true, Ordering::Relaxed);
    }
    s.download_errors.remove(id);
    drop(s);
    wake();
}

/// A model from a GGUF file the user picked. The file stays where it is.
pub fn model_from_file(path: &Path, taken: &[LocalModel]) -> Result<LocalModel, String> {
    let mut magic = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    if &magic != b"GGUF" {
        return Err(format!("{} isn't a GGUF model file.", path.display()));
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "model".into());
    // A split model is named after its first part.
    let name = stem.split("-00001-of-").next().unwrap_or(&stem).to_string();
    let base: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '-' }).collect();
    let base = base.trim_matches('-').to_string();
    let mut id = format!("file-{base}");
    let mut n = 2;
    while taken.iter().any(|m| m.id == id) {
        id = format!("file-{base}-{n}");
        n += 1;
    }
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(LocalModel { id, name, path: path.to_string_lossy().into(), size })
}

/// Deletes a model's file if Scout downloaded it, along with any half-finished download of it.
pub fn delete_model_file(m: &LocalModel) {
    if server_model().as_deref() == Some(m.id.as_str()) {
        stop();
    }
    let path = Path::new(&m.path);
    if path.parent().is_some_and(|p| crate::session::same_dir(&p.to_string_lossy(), &models_dir().to_string_lossy())) {
        let _ = std::fs::remove_file(path);
    }
    if let Some(c) = curated(&m.id) {
        let _ = std::fs::remove_file(models_dir().join(format!("{}.part", c.file)));
    }
}

/// The model list's entries for the user's local models.
pub fn models(local: &Local) -> Vec<ModelInfo> {
    local
        .models
        .iter()
        .map(|m| ModelInfo {
            id: format!("{PROVIDER}/{}", m.id),
            name: Some(m.name.clone()),
            description: Some(format!("Runs on this PC with llama.cpp. {}.", size_label(m.size))),
            context: Some(local.context as u64),
            tools: Some(local.tools != "none"),
            vision: Some(false),
            cost: Some("Free".into()),
            free: true,
            ..Default::default()
        })
        .collect()
}

pub fn size_label(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

// ---------- the server ----------

struct Server {
    child: Child,
    port: u16,
    /// The API key it was started with, made up for each start.
    api_key: String,
    /// What it was started with: when this changes, it starts again.
    key: String,
    model: String,
    backend: String,
    log: Arc<Mutex<VecDeque<String>>>,
    /// Where what's measured for this build and model is kept in `speed.json`.
    speed_key: String,
    /// What drafting from the chat has drafted and saved so far, until it's decided whether it pays (`observe`).
    drafts: Option<Drafts>,
}

/// Totals over a server's replies, from llama-server's timings.
#[derive(Default, Debug, Clone, Copy)]
pub struct Drafts {
    pub drafted: f64,
    pub accepted: f64,
    /// Prompt tokens read, and the milliseconds it took, from replies that read enough to time.
    pub read_n: f64,
    pub read_ms: f64,
    /// Tokens written, drafted ones included, and the milliseconds it took.
    pub written_n: f64,
    pub written_ms: f64,
}

impl Drafts {
    fn add(&mut self, t: &Value) {
        let n = |k: &str| t.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        self.drafted += n("draft_n");
        self.accepted += n("draft_n_accepted");
        if n("prompt_n") >= 64.0 {
            self.read_n += n("prompt_n");
            self.read_ms += n("prompt_ms");
        }
        self.written_n += n("predicted_n");
        self.written_ms += n("predicted_ms");
    }

    /// Whether drafting is worth keeping, or None until there's enough to tell. Checking a drafted token costs about
    /// what reading a prompt token does (a quarter more, since drafts come in smaller batches than prompts), and each
    /// accepted one saves writing one. The time to write a token without drafting is what's left of the writing time
    /// once the checking is taken out, over the tokens that weren't drafted.
    pub fn pays(&self) -> Option<bool> {
        if self.drafted < 1500.0 || self.read_n < 1000.0 || self.read_ms <= 0.0 {
            return None;
        }
        let check = 1.25 * self.read_ms / self.read_n;
        let written = self.written_n - self.accepted;
        let write = (self.written_ms - self.drafted * check) / written;
        if written <= 0.0 || write <= 0.0 {
            // The checking takes all the writing time, so drafting is all that wrote anything.
            return Some(true);
        }
        Some(self.accepted * write > self.drafted * check)
    }
}

/// Adds a reply's timings from llama-server to what its server has seen. Once there's enough to tell, whether drafting
/// from the chat pays is kept for this build and model, and a server started later goes without it when it doesn't.
pub fn observe(timings: &Value) {
    let mut s = server();
    let Some(srv) = s.as_mut() else { return };
    let Some(d) = srv.drafts.as_mut() else { return };
    d.add(timings);
    if let Some(pays) = d.pays() {
        update_speed(&srv.speed_key, json!({ "ngram": pays, "drafts": [d.drafted, d.accepted] }));
        srv.drafts = None;
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        crate::tools::kill_tree(&mut self.child);
        let _ = std::fs::remove_file(pid_file());
    }
}

static SERVER: Mutex<Option<Server>> = Mutex::new(None);

fn server() -> std::sync::MutexGuard<'static, Option<Server>> {
    SERVER.lock().unwrap_or_else(|e| e.into_inner())
}

fn server_model() -> Option<String> {
    server().as_ref().map(|s| s.model.clone())
}

fn server_backend() -> Option<String> {
    server().as_ref().map(|s| s.backend.clone())
}

fn pid_file() -> PathBuf {
    runtime_root().join("server.pid")
}

/// Ends a llama-server a crashed Scout left running, so it doesn't hold the memory. Only one that's still llama-server.
fn kill_orphan() {
    let Some(pid) = std::fs::read_to_string(pid_file()).ok().and_then(|p| p.trim().parse::<u32>().ok()) else { return };
    let _ = std::fs::remove_file(pid_file());
    let mut c = if cfg!(windows) {
        let mut c = Command::new("taskkill");
        c.args(["/f", "/t", "/pid", &pid.to_string(), "/fi", &format!("IMAGENAME eq {EXE}")]);
        c
    } else {
        let comm = Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if !comm.ends_with(EXE) {
            return;
        }
        let mut c = Command::new("kill");
        c.arg(pid.to_string());
        c
    };
    crate::tools::no_window(&mut c);
    let _ = c.stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// What Scout measured about running a model with a build, beyond the user's settings. Kept in `speed.json`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// GPU memory in MiB that keeps the busiest of a mixture-of-experts model's CPU-side experts on the GPU
    /// (`--moe-cache-mib`). Only when that measured faster (see `try_moe_cache`).
    pub moe_cache: Option<u32>,
    /// Drafting repeated text from the chat (`ngram-mod`). Off once real replies showed it costs more than it saves
    /// (see `observe`).
    pub ngram: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { moe_cache: None, ngram: true }
    }
}

/// What llama-server is started with. `key` keeps web pages in the browser, which may reach 127.0.0.1, from using it.
/// `margin` is the GPU memory in MiB that automatic offload leaves free, when it isn't llama.cpp's own 1024.
pub fn server_args(model_path: &str, port: u16, key: &str, local: &Local, margin: Option<u32>, tuning: Tuning) -> Vec<String> {
    let layers = local.gpu_layers.map(|n| n.to_string()).unwrap_or_else(|| "auto".into());
    let mut args: Vec<String> = [
        "-m", model_path, "--host", "127.0.0.1", "--port", &port.to_string(), "-c", &local.context.to_string(), "-ngl", &layers,
        // One conversation at a time gets the whole context.
        "-np", "1", "--jinja", "--no-webui", "--api-key", key,
        // Level 4 logs how many layers went on the GPU.
        "-lv", "4",
        // When pruning rewrites an old message, the cache of what follows it is moved along instead of worked out again.
        "--cache-reuse", "256",
        // Models whose cache can't be cut back to any point (Qwen3.5's recurrent layers, Gemma's sliding window) can
        // only go back to a checkpoint. llama.cpp spaces them at least 8,192 tokens apart, so a small context gets few
        // or none, and a chat that differs from the cache is read again from the start. A checkpoint every eighth of
        // the context keeps that to about an eighth, for the memory of eight model states.
        "--ctx-checkpoints", "8",
        "--checkpoint-min-step", &checkpoint_step(local.context).to_string(),
        "--reasoning-budget", &local.reasoning_budget.to_string(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if tuning.ngram {
        // Text the model repeats from the chat, like the code it quotes in an edit, is drafted from the chat and only
        // checked, which is quicker than writing it a token at a time. Needs no second model.
        args.extend(["--spec-type".to_string(), "ngram-mod".to_string()]);
    }
    // Prompts llama.cpp keeps in memory for later: 8 GB unless told, which on a 16 GB PC pushes the model itself out.
    if let Some(mib) = total_ram_mib().map(cache_ram) {
        args.extend(["--cache-ram".to_string(), mib.to_string()]);
    }
    if let Some(m) = margin.filter(|_| local.gpu_layers.is_none()) {
        args.extend(["-fitt".to_string(), m.to_string()]);
    }
    if let Some(mib) = tuning.moe_cache.filter(|_| local.gpu_layers.is_none()) {
        args.extend(["--moe-cache-mib".to_string(), mib.to_string()]);
    }
    args
}

/// The fewest tokens between context checkpoints: an eighth of the context, and no less than 512.
pub fn checkpoint_step(context: u32) -> u32 {
    (context / 8).max(512)
}

/// The memory in MiB for prompts kept for later: an eighth of the PC's, between 1 GB and llama.cpp's own 8 GB.
pub fn cache_ram(total_mib: u64) -> u64 {
    (total_mib / 8).clamp(1024, 8192)
}

/// This PC's memory in MiB, or None when it can't be told.
#[cfg(windows)]
fn total_ram_mib() -> Option<u64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `m` is a MEMORYSTATUSEX with its length set, as the call needs.
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    Some(m.ullTotalPhys / (1024 * 1024))
}

#[cfg(target_os = "linux")]
fn total_ram_mib() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.split_whitespace().next()?.parse().ok()?;
    Some(kb / 1024)
}

#[cfg(target_os = "macos")]
fn total_ram_mib() -> Option<u64> {
    let out = Command::new("sysctl").args(["-n", "hw.memsize"]).output().ok()?;
    let bytes: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(bytes / (1024 * 1024))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn total_ram_mib() -> Option<u64> {
    None
}

/// The GPU memory automatic offload leaves free, in MiB, tried in turn when a model doesn't load for lack of memory.
/// Some drivers (Adreno's OpenCL) need room beyond what llama.cpp's fit counts, for buffers they make while loading.
const MARGINS: [Option<u32>; 3] = [None, Some(2048), Some(4096)];

/// The margin that worked last for each model and settings, so a later start doesn't try the ones that failed again.
static MARGIN: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

/// "load_tensors: offloaded 20/33 layers to GPU" says how much of the model is on the GPU.
pub fn parse_offload(line: &str) -> Option<(u32, u32)> {
    let rest = line.split("offloaded ").nth(1)?;
    let (n, rest) = rest.split_once('/')?;
    let total = rest.split_whitespace().next()?;
    rest.contains("layers to GPU").then_some(())?;
    Some((n.trim().parse().ok()?, total.parse().ok()?))
}

/// "print_info: n_expert = 256" says how many experts a mixture-of-experts model has. Not n_expert_used and the like.
pub fn parse_experts(line: &str) -> Option<u32> {
    let rest = line.split("n_expert").nth(1)?;
    rest.starts_with(|c: char| c.is_whitespace() || c == '=').then_some(())?;
    rest.split_once('=')?.1.trim().parse().ok()
}

/// "load_tensors:   CPU_Mapped model buffer size =  3200.00 MiB" says how much of the model is on a device: whether
/// that's the computer's memory (a CPU buffer), and the MiB.
pub fn parse_buffer(line: &str) -> Option<(bool, f64)> {
    let (name, rest) = line.split_once(" model buffer size")?;
    let name = name.rsplit([' ', ':']).find(|s| !s.is_empty())?;
    let mib = rest.split_once('=')?.1.split_whitespace().next()?.parse().ok()?;
    Some((name.starts_with("CPU"), mib))
}

/// Why it stopped, from the end of its log, and whether it ran out of memory.
fn explain(log: &VecDeque<String>) -> (bool, String) {
    let all = log.iter().cloned().collect::<Vec<_>>().join("\n").to_lowercase();
    if ["out of memory", "outofdevicememory", "failed to allocate", "unable to allocate", "alloc failed", "cudamalloc failed"].iter().any(|s| all.contains(s)) {
        return (true, "There isn't enough memory for this model. Lower GPU layers or the context size in Settings > Providers > Local models, or pick a smaller model.".into());
    }
    let line = log.iter().rev().find(|l| { let l = l.to_lowercase(); l.contains("error") || l.contains("failed") }).or(log.back()).cloned().unwrap_or_default();
    (false, format!("llama.cpp stopped: {}", line.trim()))
}

fn set_server(f: impl FnOnce(&mut ServerView)) {
    f(&mut state().server);
    wake();
}

/// Starts llama-server for `model` ("local/<id>") unless it's already running it with these settings, and waits until it
/// takes requests. Returns its port and API key. `status` hears what to show meanwhile. With automatic offload, a model
/// that runs out of GPU memory while loading starts again with more of it left free, so more of it runs on the CPU.
///
/// With automatic offload on a GPU build, the first start of a model also measures it: a short run on the GPU, and
/// unless that's clearly quick, one on the CPU. Some GPUs (a laptop's built-in one) are slower than the CPU beside
/// them, and the model then runs on the CPU from here on. The answer is kept in `speed.json` for this build and model.
pub fn ensure(local: &Local, model: &str, cancel: &AtomicBool, status: &mut dyn FnMut(String)) -> Result<(u16, String), String> {
    // A message beats a load that's only getting ready for one.
    *WANTED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    ensure_now(local, model, cancel, status)
}

/// Starts are one at a time, so a message sent while the model is being measured waits for that instead of undoing it.
static ENSURE: Mutex<()> = Mutex::new(());
/// The model `preload` was last asked for. A load that's still waiting its turn gives up when this has moved on.
static WANTED: Mutex<Option<String>> = Mutex::new(None);

/// How every request a chat sends a local model starts: its system prompt and tools, exactly as `api::stream_chat`
/// sends them.
pub struct Prefix {
    pub system: Message,
    pub tools: Vec<Value>,
    pub parallel_tool_calls: Option<bool>,
}

/// Loads a model in the background when it's chosen, so the first message doesn't wait for it. Only when nothing is
/// loaded: a model that's in use is never swapped out from under a chat. With the chat's `prefix`, llama-server then
/// reads it too (`warm`), which on a CPU is most of the wait for a first reply.
pub fn preload(local: &Local, model: &str, prefix: Option<Prefix>) {
    let (local, model) = (local.clone(), model.to_string());
    *WANTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(model.clone());
    std::thread::spawn(move || {
        drop(ENSURE.lock());
        let wanted = || WANTED.lock().unwrap_or_else(|e| e.into_inner()).as_deref() == Some(model.as_str());
        if !wanted() || server().is_some() {
            return;
        }
        let Ok((port, key)) = ensure_now(&local, &model, &AtomicBool::new(false), &mut |_| {}) else { return };
        // Not once a message has been sent: that reads the prefix itself.
        if let Some(p) = prefix.filter(|_| wanted()) {
            warm(port, &key, &model, &p);
        }
    });
}

/// Has llama-server read `prefix`, so a first message only reads what comes after it. The request asks for a reply to
/// a stand-in message, and is dropped as the reply begins; what was read stays in its cache.
fn warm(port: u16, key: &str, model: &str, prefix: &Prefix) {
    let mut cfg = crate::config::load_config().for_provider(PROVIDER);
    cfg.base_url = format!("http://127.0.0.1:{port}/v1");
    cfg.api_key = Some(key.to_string());
    let messages = [prefix.system.clone(), Message::User { content: Content::Text("Hello.".into()) }];
    let req = ChatRequest { model, messages: &messages, tools: Some(&prefix.tools), tool_choice_none: false, temperature: None, effort: None, parallel_tool_calls: prefix.parallel_tool_calls };
    // A message sent meanwhile waits for the one slot, and reads the same prefix, so it gains from it either way.
    let stop = Arc::new(AtomicBool::new(false));
    let _ = crate::api::stream_chat(&cfg, req, &mut |_| stop.store(true, Ordering::Relaxed), &stop);
}

fn ensure_now(local: &Local, model: &str, cancel: &AtomicBool, status: &mut dyn FnMut(String)) -> Result<(u16, String), String> {
    let id = model.strip_prefix(&format!("{PROVIDER}/")).unwrap_or(model);
    let m = local.models.iter().find(|m| m.id == id).ok_or("That local model isn't set up on this PC any more. Pick another in the model menu.")?;
    if !Path::new(&m.path).is_file() {
        return Err(format!("{} isn't there any more. Download it again or add the file in Settings > Providers > Local models.", m.path));
    }
    let b = backend_for(local).ok_or("llama.cpp has no build for this computer.")?;
    let exe = server_exe(b).ok_or("llama.cpp isn't installed yet. Install it in Settings > Providers > Local models.")?;
    let mut said = false;
    let _one = loop {
        match ENSURE.try_lock() {
            Ok(g) => break g,
            Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
            Err(_) if cancel.load(Ordering::Relaxed) => return Err(CANCELLED.into()),
            Err(_) => {
                if !std::mem::replace(&mut said, true) {
                    status(format!("Getting {} ready…", m.name));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    let speed_key = format!("{BUILD}-{b}|{}|{}", m.path, m.size);
    let tuned = tuning(&speed_key);
    // The expert cache was measured with automatic offload on a GPU, so it's only for that.
    let plain = Tuning { moe_cache: None, ..tuned };
    let run = Run { m, b, exe: &exe, speed_key: &speed_key };
    if local.gpu_layers.is_some() || b == "cpu" {
        return start(local, &run, plain, cancel, status);
    }
    let on_cpu = Local { gpu_layers: Some(0), ..local.clone() };
    let start_on_cpu = |cancel: &AtomicBool, status: &mut dyn FnMut(String)| {
        let r = start(&on_cpu, &run, plain, cancel, status);
        if r.is_ok() {
            set_server(|v| v.cpu_faster = true);
        }
        r
    };
    match cpu_faster(&speed_key) {
        Some(true) => return start_on_cpu(cancel, status),
        Some(false) if moe_tried(&speed_key) => return start(local, &run, tuned, cancel, status),
        // Measured before Scout tried the expert cache: the GPU's speed is what was measured then.
        Some(false) => {
            let gpu = start(local, &run, plain, cancel, status)?;
            return match saved_gpu_speed(&speed_key) {
                Some(g) => try_moe_cache(local, &run, plain, g, gpu, cancel, status),
                None => Ok(gpu),
            };
        }
        None => {}
    }
    let gpu = start(local, &run, plain, cancel, status)?;
    // With nothing on the GPU, the CPU is already what runs it.
    if !state().server.offloaded.is_some_and(|(n, _)| n > 0) {
        return Ok(gpu);
    }
    status(format!("Checking whether {} runs faster on the GPU or the CPU. This happens once…", m.name));
    let cancelled = || cancel.load(Ordering::Relaxed);
    let Some(g) = measure(gpu.0, &gpu.1, cancel) else { return if cancelled() { Err(CANCELLED.into()) } else { Ok(gpu) } };
    if clearly_gpu(g, m.size) {
        save_speed(&speed_key, false, g, None);
        return try_moe_cache(local, &run, plain, g, gpu, cancel, status);
    }
    stop();
    let cpu = start_on_cpu(cancel, status);
    let c = cpu.as_ref().ok().and_then(|(port, key)| measure(*port, key, cancel));
    if cancelled() {
        return Err(CANCELLED.into());
    }
    match (cpu, c) {
        (Ok(r), Some(c)) if c.beats(g) => {
            save_speed(&speed_key, true, g, Some(c));
            return Ok(r);
        }
        // A CPU run that couldn't be measured is tried again next time; one that lost, or didn't load, isn't.
        (Ok(_), None) => {}
        (_, c) => save_speed(&speed_key, false, g, c),
    }
    stop();
    start(local, &run, plain, cancel, status)
}

/// What a start runs: the model, the build and its llama-server, and its entry in `speed.json`.
struct Run<'a> {
    m: &'a LocalModel,
    b: &'a str,
    exe: &'a Path,
    speed_key: &'a str,
}

/// Builds that put the model in a GPU's own memory. The expert cache does nothing where the GPU shares the computer's.
const OWN_MEMORY: [&str; 2] = ["cuda", "vulkan"];

/// Whether a mixture-of-experts model kept enough of itself in the computer's memory for an expert cache to be worth
/// trying: more than a sixth of its weights.
pub fn worth_moe_cache(experts: u32, cpu_mib: f64, gpu_mib: f64) -> bool {
    experts > 0 && gpu_mib > 0.0 && cpu_mib > (cpu_mib + gpu_mib) / 6.0
}

/// The GPU memory to try for the expert cache: a tenth of the model, which leaves most of the room to whole layers,
/// so reading a prompt stays about as quick.
pub fn moe_cache_size(model_bytes: u64) -> u32 {
    ((model_bytes / 10 / (1024 * 1024)) as u32).max(1024)
}

/// With the model running on a GPU (`gpu`, measured at `g`), a mixture-of-experts model that didn't fit is started once
/// more with an expert cache: GPU memory that keeps the experts it picks most often there, though they live in the
/// computer's memory. It's kept only when a usual step is a tenth quicker, since a cache that misses costs time, and
/// whatever the answer, it's kept for this build and model.
#[allow(clippy::too_many_arguments)]
fn try_moe_cache(local: &Local, run: &Run, plain: Tuning, g: Speed, gpu: (u16, String), cancel: &AtomicBool, status: &mut dyn FnMut(String)) -> Result<(u16, String), String> {
    let v = state().server.clone();
    if !OWN_MEMORY.contains(&run.b) || !worth_moe_cache(v.experts, v.cpu_mib, v.gpu_mib) {
        update_speed(run.speed_key, json!({ "moeCache": 0 }));
        return Ok(gpu);
    }
    status(format!("Checking whether {} runs faster with its busiest experts kept on the GPU. This happens once…", run.m.name));
    let mib = moe_cache_size(run.m.size);
    stop();
    let cached = start(local, run, Tuning { moe_cache: Some(mib), ..plain }, cancel, status);
    let c = cached.as_ref().ok().and_then(|(port, key)| measure(*port, key, cancel));
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    match (cached, c) {
        (Ok(r), Some(c)) if c.beats(g) => {
            update_speed(run.speed_key, json!({ "moeCache": mib, "moeSpeed": [c.prompt, c.reply] }));
            return Ok(r);
        }
        // One that couldn't be measured is tried again next time; one that lost, or didn't load, isn't.
        (Ok(_), None) => {}
        (_, c) => update_speed(run.speed_key, json!({ "moeCache": 0, "moeSpeed": c.map(|c| [c.prompt, c.reply]) })),
    }
    stop();
    start(local, run, plain, cancel, status)
}

/// How fast a model ran, in tokens a second: reading a prompt, and writing a reply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speed {
    pub prompt: f64,
    pub reply: f64,
}

impl Speed {
    /// The seconds a usual step of a coding chat takes: 1,500 new tokens read and 300 written.
    fn step(self) -> f64 {
        1500.0 / self.prompt + 300.0 / self.reply
    }

    /// Whether the CPU at this speed is worth choosing over the GPU at `gpu`: only when it's a tenth quicker or more,
    /// since a busy CPU slows the rest of the PC.
    pub fn beats(self, gpu: Speed) -> bool {
        self.step() < gpu.step() * 0.9
    }
}

/// Whether the GPU is quick enough that the CPU needn't be tried. A CPU writes about as many tokens a second as its
/// memory can read the model through, and no PC's memory reads 100 GB a second.
pub fn clearly_gpu(gpu: Speed, model_bytes: u64) -> bool {
    gpu.reply * (model_bytes as f64 / 1e9) > 100.0
}

/// llama-server's own timings for a completion.
pub fn speed_from(body: &Value) -> Option<Speed> {
    let t = body.get("timings")?;
    let n = |k: &str| t.get(k)?.as_f64();
    // A reply that ended after a few tokens says little about its speed.
    (n("predicted_n")? >= 8.0).then_some(())?;
    let s = Speed { prompt: n("prompt_per_second")?, reply: n("predicted_per_second")? };
    (s.prompt > 0.0 && s.reply > 0.0).then_some(s)
}

/// Times a short completion on the running server: about 450 tokens read and 48 written. A stop ends the server, which
/// ends the completion.
fn measure(port: u16, key: &str, cancel: &AtomicBool) -> Option<Speed> {
    let prompt = format!("Counting up, one number on each line:\n{}\n", (1..=150).map(|n| n.to_string()).collect::<Vec<_>>().join("\n"));
    let (url, auth) = (format!("http://127.0.0.1:{port}/completion"), format!("Bearer {key}"));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let body = json!({ "prompt": prompt, "n_predict": 48, "cache_prompt": false, "temperature": 0 });
        let res = crate::api::http().post(&url).header("Authorization", &auth).config().timeout_global(Some(Duration::from_secs(300))).build().send_json(body);
        let _ = tx.send(res.ok().and_then(|r| r.into_body().read_json::<Value>().ok()));
    });
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(body) => return speed_from(&body?),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if cancel.load(Ordering::Relaxed) => {
                stop();
                return None;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return None,
        }
    }
}

fn speed_file() -> PathBuf {
    runtime_root().join("speed.json")
}

/// What's kept for this build and model, if anything.
fn speed_entry(key: &str) -> Option<Value> {
    let mut all: Value = serde_json::from_str(&std::fs::read_to_string(speed_file()).ok()?).ok()?;
    Some(all.get_mut(key)?.take())
}

/// What was measured for this build and model: Some(true) when the CPU was faster, None when it hasn't been measured.
fn cpu_faster(key: &str) -> Option<bool> {
    speed_entry(key)?.get("cpu")?.as_bool()
}

/// Whether the expert cache was tried for this build and model (0 when it wasn't worth it, or lost).
fn moe_tried(key: &str) -> bool {
    speed_entry(key).is_some_and(|e| e.get("moeCache").is_some_and(Value::is_u64))
}

/// The GPU's speed as measured for this build and model.
fn saved_gpu_speed(key: &str) -> Option<Speed> {
    let e = speed_entry(key)?;
    let s = e.get("gpuSpeed")?.as_array()?;
    Some(Speed { prompt: s.first()?.as_f64()?, reply: s.get(1)?.as_f64()? })
}

/// What a start uses from what was measured.
pub fn tuning_from(entry: Option<&Value>) -> Tuning {
    let get = |k: &str| entry.and_then(|e| e.get(k));
    Tuning {
        moe_cache: get("moeCache").and_then(Value::as_u64).filter(|&n| n > 0).map(|n| n as u32),
        ngram: get("ngram").and_then(Value::as_bool).unwrap_or(true),
    }
}

fn tuning(key: &str) -> Tuning {
    tuning_from(speed_entry(key).as_ref())
}

/// Sets `fields` in what's kept for this build and model, leaving the rest as it was.
fn update_speed(key: &str, fields: Value) {
    let mut all: serde_json::Map<String, Value> = std::fs::read_to_string(speed_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let entry = all.entry(key.to_string()).or_insert_with(|| json!({}));
    if let (Some(e), Some(f)) = (entry.as_object_mut(), fields.as_object()) {
        e.extend(f.clone());
    }
    let _ = std::fs::write(speed_file(), Value::Object(all).to_string());
}

fn save_speed(key: &str, cpu: bool, gpu: Speed, on_cpu: Option<Speed>) {
    update_speed(key, json!({ "cpu": cpu, "gpuSpeed": [gpu.prompt, gpu.reply], "cpuSpeed": on_cpu.map(|c| [c.prompt, c.reply]) }));
}

/// Starts llama-server with these settings, leaving more GPU memory free each time it runs out of it while loading.
fn start(local: &Local, run: &Run, tuning: Tuning, cancel: &AtomicBool, status: &mut dyn FnMut(String)) -> Result<(u16, String), String> {
    let m = run.m;
    let key = format!("{}|{}|{:?}|{}|{}|{:?}", run.exe.display(), m.path, local.gpu_layers, local.context, local.reasoning_budget, tuning);
    let tries = if local.gpu_layers.is_none() { MARGINS.len() } else { 1 };
    let from = MARGIN.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|w| w.get(&key).copied()).unwrap_or(0).min(tries - 1);
    for i in from..tries {
        match start_and_wait(local, run, tuning, &key, MARGINS[i], cancel, status) {
            Err((true, _)) if i + 1 < tries && !cancel.load(Ordering::Relaxed) => status(format!("{} didn't fit on the GPU. Trying again with more of it on the CPU…", m.name)),
            Err((_, why)) => return Err(why),
            Ok(r) => {
                MARGIN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, i);
                return Ok(r);
            }
        }
    }
    unreachable!()
}

/// One try at `ensure`: the running server if it has these settings, else a new one with `margin`. An error says
/// whether it ran out of memory.
#[allow(clippy::too_many_arguments)]
fn start_and_wait(local: &Local, run: &Run, tuning: Tuning, key: &str, margin: Option<u32>, cancel: &AtomicBool, status: &mut dyn FnMut(String)) -> Result<(u16, String), (bool, String)> {
    let (m, b, exe) = (run.m, run.b, run.exe);
    let plain = |e: String| (false, e);
    let key = key.to_string();
    let (port, api_key) = {
        let mut s = server();
        let alive = s.as_mut().is_some_and(|s| matches!(s.child.try_wait(), Ok(None)));
        match s.as_ref() {
            Some(srv) if alive && srv.key == key => (srv.port, srv.api_key.clone()),
            _ => {
                *s = None;
                kill_orphan();
                let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| plain(e.to_string()))?;
                let mut random = [0u8; 24];
                getrandom::fill(&mut random).map_err(|e| plain(e.to_string()))?;
                let api_key = hex(random);
                let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
                let mut c = Command::new(exe);
                c.args(server_args(&m.path, port, &api_key, local, margin, tuning)).current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
                if cfg!(target_os = "linux") {
                    c.env("LD_LIBRARY_PATH", &dir);
                }
                crate::tools::no_window(&mut c);
                let mut child = c.spawn().map_err(|e| plain(format!("Couldn't start llama.cpp: {e}")))?;
                let _ = std::fs::write(pid_file(), child.id().to_string());
                let log = Arc::new(Mutex::new(VecDeque::new()));
                // Before its log is read, which fills this in.
                set_server(|v| *v = ServerView { state: "loading", model: Some(m.id.clone()), ..Default::default() });
                for out in [child.stdout.take().map(|o| Box::new(o) as Box<dyn Read + Send>), child.stderr.take().map(|e| Box::new(e) as Box<dyn Read + Send>)].into_iter().flatten() {
                    let log = log.clone();
                    std::thread::spawn(move || {
                        for line in BufReader::new(out).lines().map_while(Result::ok) {
                            if let Some(o) = parse_offload(&line) {
                                set_server(|v| v.offloaded = Some(o));
                            }
                            if let Some(n) = parse_experts(&line) {
                                state().server.experts = n;
                            }
                            if let Some((cpu, mib)) = parse_buffer(&line) {
                                let mut s = state();
                                *(if cpu { &mut s.server.cpu_mib } else { &mut s.server.gpu_mib }) += mib;
                            }
                            let mut l = log.lock().unwrap_or_else(|e| e.into_inner());
                            l.push_back(line);
                            if l.len() > 40 {
                                l.pop_front();
                            }
                        }
                    });
                }
                // Drafting is watched until it's known whether it pays for this build and model.
                let undecided = tuning.ngram && speed_entry(run.speed_key).is_none_or(|e| e.get("ngram").is_none());
                let drafts = undecided.then(Drafts::default);
                *s = Some(Server { child, port, api_key: api_key.clone(), key, model: m.id.clone(), backend: b.to_string(), log, speed_key: run.speed_key.to_string(), drafts });
                (port, api_key)
            }
        }
    };
    if state().server.state == "ready" {
        return Ok((port, api_key));
    }
    status(format!("Loading {} into memory…", m.name));
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(plain(CANCELLED.into()));
        }
        {
            let mut s = server();
            let Some(srv) = s.as_mut().filter(|srv| srv.port == port) else { return Err(plain("llama.cpp was stopped.".into())) };
            if !matches!(srv.child.try_wait(), Ok(None)) {
                let (memory, why) = explain(&srv.log.lock().unwrap_or_else(|e| e.into_inner()));
                *s = None;
                set_server(|v| {
                    v.state = "error";
                    v.error = Some(why.clone());
                });
                return Err((memory, why));
            }
        }
        let ok = crate::api::http().get(&format!("http://127.0.0.1:{port}/health")).call().is_ok_and(|r| r.status().as_u16() == 200);
        if ok {
            set_server(|v| v.state = "ready");
            return Ok((port, api_key));
        }
        if started.elapsed() > Duration::from_secs(900) {
            stop();
            return Err(plain("llama.cpp took more than 15 minutes to load the model, so it was stopped.".into()));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Stops llama-server, freeing the model's memory.
pub fn stop() {
    *WANTED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *server() = None;
    set_server(|v| *v = ServerView { state: "stopped", ..Default::default() });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn auto_offload_lets_llama_cpp_fit_the_model_and_a_number_is_exact() {
        let mut local = Local::default();
        let args = server_args("m.gguf", 9000, "k3y", &local, None, Tuning::default());
        let at = |a: &[String], flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].clone());
        assert_eq!(at(&args, "--api-key").as_deref(), Some("k3y"));
        assert_eq!(at(&args, "-fitt"), None, "llama.cpp's own margin first");
        assert_eq!(at(&server_args("m.gguf", 9000, "k3y", &local, Some(2048), Tuning::default()), "-fitt").as_deref(), Some("2048"));
        assert_eq!(at(&args, "-ngl").as_deref(), Some("auto"));
        assert_eq!(at(&args, "-c").as_deref(), Some("8192"));
        assert_eq!(at(&args, "--cache-reuse").as_deref(), Some("256"), "a pruned chat keeps the cache after the change");
        assert_eq!(at(&args, "--spec-type").as_deref(), Some("ngram-mod"), "repeated text is drafted from the chat");
        assert_eq!(at(&args, "--reasoning-budget").as_deref(), Some("1024"), "thinking is capped unless the user lifts it");
        assert_eq!(at(&args, "--host").as_deref(), Some("127.0.0.1"), "never reachable from other computers");
        assert!(args.contains(&"--jinja".to_string()), "tool calls need the model's own chat template");
        local.gpu_layers = Some(12);
        local.context = 16384;
        let args = server_args("m.gguf", 9000, "k3y", &local, Some(2048), Tuning::default());
        assert_eq!(at(&args, "-ngl").as_deref(), Some("12"));
        assert_eq!(at(&args, "-fitt"), None, "an exact layer count is what the user asked for");
        assert_eq!(at(&args, "-c").as_deref(), Some("16384"));
        assert_eq!(at(&args, "--checkpoint-min-step").as_deref(), Some("2048"), "a checkpoint every eighth of the context");
        let moe = Tuning { moe_cache: Some(2200), ngram: true };
        assert_eq!(at(&server_args("m.gguf", 9000, "k3y", &local, None, moe), "--moe-cache-mib"), None, "measured with automatic offload only");
        local.gpu_layers = None;
        let args = server_args("m.gguf", 9000, "k3y", &local, None, moe);
        assert_eq!(at(&args, "--moe-cache-mib").as_deref(), Some("2200"));
        let args = server_args("m.gguf", 9000, "k3y", &local, None, Tuning { moe_cache: None, ngram: false });
        assert_eq!(at(&args, "--spec-type"), None, "drafting that didn't pay is left off");
        assert_eq!(at(&args, "--moe-cache-mib"), None);
        if total_ram_mib().is_some() {
            assert!(at(&args, "--cache-ram").is_some(), "prompts kept for later are capped by the PC's memory");
        }
    }

    #[test]
    fn memory_for_prompts_and_checkpoints_follows_the_pc_and_the_context() {
        assert_eq!(cache_ram(16 * 1024), 2048, "an eighth of a 16 GB PC");
        assert_eq!(cache_ram(4 * 1024), 1024);
        assert_eq!(cache_ram(256 * 1024), 8192, "never more than llama.cpp's own");
        assert_eq!(checkpoint_step(8192), 1024);
        assert_eq!(checkpoint_step(2048), 512);
        assert_eq!(checkpoint_step(262_144), 32_768);
    }

    #[test]
    fn what_was_measured_tunes_a_start() {
        assert_eq!(tuning_from(None), Tuning { moe_cache: None, ngram: true }, "drafting until it's shown not to pay");
        assert_eq!(tuning_from(Some(&json!({ "cpu": false, "moeCache": 2200, "ngram": false }))), Tuning { moe_cache: Some(2200), ngram: false });
        assert_eq!(tuning_from(Some(&json!({ "moeCache": 0 }))).moe_cache, None, "tried and lost");
    }

    #[test]
    fn drafting_is_kept_only_when_it_saves_more_than_it_costs() {
        // Reading at 110 tokens a second, writing at 20 without drafts.
        let at = |drafted: f64, accepted: f64| {
            let (check, write) = (1.25 / 110.0 * 1000.0, 1000.0 / 20.0);
            let written_n = 2000.0;
            Drafts { drafted, accepted, read_n: 5000.0, read_ms: 5000.0 / 110.0 * 1000.0, written_n, written_ms: (written_n - accepted) * write + drafted * check }
        };
        assert_eq!(at(3000.0, 900.0).pays(), Some(true), "nearly a third accepted");
        assert_eq!(at(3000.0, 150.0).pays(), Some(false), "a twentieth accepted costs more than it saves on a CPU");
        assert_eq!(at(500.0, 400.0).pays(), None, "too little drafted to tell");
        // A GPU reads thousands of tokens a second, so even a little accepted pays.
        let gpu = Drafts { drafted: 3000.0, accepted: 150.0, read_n: 9000.0, read_ms: 3000.0, written_n: 2000.0, written_ms: 1850.0 * 25.0 + 3000.0 * 1.25 / 3.0 };
        assert_eq!(gpu.pays(), Some(true));
        let mut d = Drafts::default();
        d.add(&json!({ "prompt_n": 1200, "prompt_ms": 10000.0, "predicted_n": 300, "predicted_ms": 15000.0, "draft_n": 120, "draft_n_accepted": 40 }));
        d.add(&json!({ "prompt_n": 12, "prompt_ms": 400.0, "predicted_n": 20, "predicted_ms": 1000.0 }));
        assert_eq!((d.drafted, d.accepted, d.read_n, d.written_n), (120.0, 40.0, 1200.0, 320.0), "a short read isn't timed");
    }

    #[test]
    fn a_mixture_of_experts_and_where_it_went_are_read_from_the_log() {
        assert_eq!(parse_experts("print_info: n_expert         = 256"), Some(256));
        assert_eq!(parse_experts("print_info: n_expert_used    = 8"), None);
        assert_eq!(parse_experts("print_info: n_expert_shared  = 1"), None);
        assert_eq!(parse_buffer("load_tensors:   CPU_Mapped model buffer size = 14336.50 MiB"), Some((true, 14336.5)));
        assert_eq!(parse_buffer("load_tensors:        CUDA0 model buffer size =  6850.00 MiB"), Some((false, 6850.0)));
        assert_eq!(parse_buffer("llama_context: CUDA0 compute buffer size = 300.00 MiB"), None);
        assert!(worth_moe_cache(256, 14336.0, 6850.0), "most of its experts in the computer's memory");
        assert!(!worth_moe_cache(256, 400.0, 20000.0), "it fit, apart from its embeddings");
        assert!(!worth_moe_cache(0, 14336.0, 6850.0), "not a mixture of experts");
        assert_eq!(moe_cache_size(22_134_528_992), 2110);
        assert_eq!(moe_cache_size(2_000_000_000), 1024);
    }

    #[test]
    fn cuda_comes_first_only_with_an_nvidia_driver() {
        let ids = |nvidia| ordered("windows", "x86_64", nvidia).iter().map(|b| b.id).collect::<Vec<_>>();
        assert_eq!(ids(false), ["vulkan", "cuda", "cpu"]);
        assert_eq!(ids(true), ["cuda", "vulkan", "cpu"]);
        assert_eq!(ordered("macos", "aarch64", true).len(), 1, "a platform without CUDA keeps its order");
    }

    #[test]
    fn the_cpu_is_chosen_only_when_it_measures_clearly_faster() {
        let gpu = Speed { prompt: 60.0, reply: 7.4 };
        assert!(Speed { prompt: 90.0, reply: 18.0 }.beats(gpu), "a built-in GPU slower than its CPU");
        assert!(!Speed { prompt: 62.0, reply: 7.6 }.beats(gpu), "a near tie stays on the GPU");
        assert!(!Speed { prompt: 20.0, reply: 9.0 }.beats(Speed { prompt: 900.0, reply: 8.0 }), "a much slower read outweighs a slightly faster write");
        // A 2.7 GB model at 7.4 tokens a second could be a CPU's speed; at 80 it couldn't.
        assert!(!clearly_gpu(gpu, 2_740_937_888));
        assert!(clearly_gpu(Speed { prompt: 2000.0, reply: 80.0 }, 2_740_937_888));
        let body = |n: u32| json!({ "content": "…", "timings": { "prompt_n": 450, "prompt_per_second": 61.5, "predicted_n": n, "predicted_per_second": 7.4 } });
        assert_eq!(speed_from(&body(48)), Some(Speed { prompt: 61.5, reply: 7.4 }));
        assert_eq!(speed_from(&body(2)), None, "too short a reply to time");
        assert_eq!(speed_from(&json!({ "error": "no" })), None);
    }

    #[test]
    fn offload_is_read_from_the_log() {
        assert_eq!(parse_offload("load_tensors: offloaded 20/33 layers to GPU"), Some((20, 33)));
        assert_eq!(parse_offload("llm_load_tensors: offloaded 0/29 layers to GPU"), Some((0, 29)));
        assert_eq!(parse_offload("offloaded 3 files"), None);
    }

    #[test]
    fn presets_go_from_nothing_to_everything() {
        assert_eq!(tool_names("none"), Some(&[][..]));
        let essential = tool_names("essential").unwrap();
        assert!(essential.contains(&"view") && essential.contains(&"bash") && essential.contains(&"present_plan"));
        assert!(!essential.contains(&"task") && !essential.contains(&"web_search"));
        assert!(tool_names("standard").unwrap().iter().all(|t| crate::tools::known(t)));
        assert!(essential.iter().all(|t| tool_names("standard").unwrap().contains(t)));
        assert_eq!(tool_names("full"), None);
        assert_eq!(tool_names("something old"), Some(&ESSENTIAL[..]));
    }

    #[test]
    fn every_platform_build_and_model_is_pinned() {
        for b in ALL_BACKENDS {
            for a in b.archives {
                assert_eq!(a.sha256.len(), 64, "{}", a.file);
                assert!(a.file.contains(BUILD) || a.file.starts_with("cudart-"), "{}", a.file);
            }
        }
        assert!(CURATED.iter().all(|c| c.sha256.len() == 64 && c.file.ends_with(".gguf")));
        assert!(!backends().is_empty(), "this platform has a build");
    }

    #[test]
    fn a_file_must_be_gguf_and_gets_its_own_id() {
        let dir = std::env::temp_dir().join(format!("scout-local-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("My Model-Q4_K_M-00001-of-00002.gguf");
        std::fs::write(&f, b"GGUF....").unwrap();
        let m = model_from_file(&f, &[]).unwrap();
        assert_eq!((m.id.as_str(), m.name.as_str()), ("file-my-model-q4-k-m", "My Model-Q4_K_M"));
        assert_eq!(model_from_file(&f, std::slice::from_ref(&m)).unwrap().id, "file-my-model-q4-k-m-2");
        let bad = dir.join("notes.gguf");
        std::fs::write(&bad, b"hello").unwrap();
        assert!(model_from_file(&bad, &[]).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The whole way through on this PC: installs llama.cpp, downloads the smallest model, starts it and asks for a tool
    /// call. Several GB of downloads, so: `SCOUT_HOME=<temp> cargo test -- --ignored live_local --nocapture`.
    /// LOCAL_BACKEND picks the build (the first for this platform otherwise); LOCAL_LAYERS sets GPU layers; LOCAL_MODEL
    /// picks a curated model by id (qwen3.5-4b otherwise).
    #[test]
    #[ignore]
    fn live_local_model() {
        use crate::api::{ChatRequest, Content, Message, stream_chat};
        let no = AtomicBool::new(false);
        let id = std::env::var("LOCAL_BACKEND").unwrap_or_else(|_| backends()[0].id.to_string());
        let b = backend(&id).expect("a build for this platform");
        if server_exe(b.id).is_none() {
            install(b, &no).unwrap();
            state().installed = None;
        }
        let c = curated(&std::env::var("LOCAL_MODEL").unwrap_or_else(|_| "qwen3.5-4b".into())).expect("a curated model's id");
        let model = format!("{PROVIDER}/{}", c.id);
        let dest = models_dir().join(c.file);
        download(&format!("https://huggingface.co/{}/resolve/main/{}", c.repo, c.file), &dest, c.sha256, c.size, &no, &mut |_| {}).unwrap();
        let local = Local {
            backend: Some(b.id.into()),
            gpu_layers: std::env::var("LOCAL_LAYERS").ok().and_then(|n| n.parse().ok()),
            models: vec![LocalModel { id: c.id.into(), name: c.name.into(), path: dest.to_string_lossy().into(), size: c.size }],
            ..Default::default()
        };
        let started = Instant::now();
        let (port, key) = ensure(&local, &model, &no, &mut |s| println!("{s}")).unwrap_or_else(|e| panic!("{e}"));
        println!("{} loaded in {:?} on port {port}; offloaded {:?}", b.name, started.elapsed(), state().server.offloaded);
        assert!(state().server.offloaded.is_some_and(|(_, total)| total > 0), "the log says how much is on the GPU");
        assert_eq!(ensure(&local, &model, &no, &mut |_| {}).unwrap().0, port, "a running server is reused");
        let mut cfg = crate::config::load_config().for_provider(PROVIDER);
        cfg.base_url = format!("http://127.0.0.1:{port}/v1");
        // Without its key, nothing else on this PC can use it.
        assert_eq!(crate::api::http().get(&format!("{}/models", cfg.base_url)).call().unwrap().status().as_u16(), 401);
        cfg.api_key = Some(key);
        let dir = std::env::temp_dir().join(format!("scout-live-local-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let tools = crate::tools::specs(tool_names("essential"), crate::tools::EditTool::StrReplace);
        let messages = [
            Message::System { content: crate::prompt::short_system_prompt(&cwd, crate::tools::EditTool::StrReplace, c.name, &Default::default(), true, true) },
            Message::User { content: Content::Text("Read notes.txt with the view tool.".into()) },
        ];
        let req = ChatRequest { model: &model, messages: &messages, tools: Some(&tools), tool_choice_none: false, temperature: None, effort: None, parallel_tool_calls: Some(false) };
        let started = Instant::now();
        let r = stream_chat(&cfg, req, &mut |_| {}, &Arc::new(AtomicBool::new(false))).unwrap_or_else(|e| panic!("{}", e.message));
        println!("answered in {:?}: {:?} {:?} usage {:?}", started.elapsed(), r.content, r.tool_calls.iter().map(|t| (&t.function.name, &t.function.arguments)).collect::<Vec<_>>(), r.usage.as_ref().map(|u| (u.prompt, u.completion)));
        stop();
        assert!(server().is_none());
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(r.tool_calls.first().map(|t| t.function.name.as_str()), Some("view"));
    }

    /// Serves `body`, honouring Range unless `ranges` is false, each connection on its own thread. The first `cut`
    /// answers stop halfway, as a dropped connection would.
    fn serve(body: Vec<u8>, ranges: bool, cut: usize) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/m.gguf", l.local_addr().unwrap());
        let (body, cut) = (Arc::new(body), Arc::new(std::sync::atomic::AtomicUsize::new(cut)));
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let (body, cut) = (body.clone(), cut.clone());
                std::thread::spawn(move || {
                    let mut s = s;
                    let mut req = String::new();
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    loop {
                        let mut line = String::new();
                        if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        req.push_str(&line);
                    }
                    let range = req.lines().find_map(|l| {
                        let r = l.to_lowercase().strip_prefix("range: bytes=")?.trim().to_string();
                        let (a, b) = r.split_once('-')?;
                        Some((a.parse::<usize>().ok()?, b.parse::<usize>().map(|b| b + 1).unwrap_or(body.len())))
                    });
                    let (status, part) = match range {
                        Some((a, b)) if ranges => ("206 Partial Content", &body[a..b.min(body.len())]),
                        _ => ("200 OK", &body[..]),
                    };
                    let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", part.len());
                    let short = cut.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok();
                    let _ = s.write_all(if short { &part[..part.len() / 2] } else { part });
                });
            }
        });
        url
    }

    #[test]
    fn a_download_comes_in_chunks_carries_on_from_where_it_stopped_and_is_checked() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let sha = hex(Sha256::digest(&body));
        let dir = std::env::temp_dir().join(format!("scout-dl-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let no = AtomicBool::new(false);
        let size = body.len() as u64;
        // An older download got the first 70,000 bytes: the four 16 KB chunks inside that are kept. Two answers are cut
        // short and fetched again. A server without ranges gets one download from the start.
        for (ranges, cut) in [(true, 0), (true, 2), (false, 0)] {
            let dest = dir.join(format!("m-{ranges}-{cut}.gguf"));
            std::fs::write(dest.with_file_name(format!("m-{ranges}-{cut}.gguf.part")), &body[..70_000]).unwrap();
            let mut first = None;
            download_with(&serve(body.clone(), ranges, cut), &dest, &sha, size, 16_384, 4, &no, &mut |n| {
                first.get_or_insert(n);
            })
            .unwrap_or_else(|e| panic!("ranges {ranges}, cut {cut}: {e}"));
            assert_eq!(std::fs::read(&dest).unwrap(), body, "ranges {ranges}, cut {cut}");
            assert_eq!(first, Some(4 * 16_384), "starts from the chunks that were there");
            assert!(!dir.join(format!("m-{ranges}-{cut}.gguf.part.done")).exists());
        }
        // Chunks listed as done are kept, wherever they are.
        let dest = dir.join("listed.gguf");
        let mut part = vec![0u8; body.len()];
        part[16_384 * 5..16_384 * 6].copy_from_slice(&body[16_384 * 5..16_384 * 6]);
        std::fs::write(dir.join("listed.gguf.part"), &part).unwrap();
        std::fs::write(dir.join("listed.gguf.part.done"), "5\n").unwrap();
        let mut first = None;
        download_with(&serve(body.clone(), true, 0), &dest, &sha, size, 16_384, 4, &no, &mut |n| {
            first.get_or_insert(n);
        })
        .unwrap();
        assert_eq!((std::fs::read(&dest).unwrap() == body, first), (true, Some(16_384)));
        // A chunk that keeps failing stops the download, and what came in stays for next time.
        let dest = dir.join("broken.gguf");
        assert!(download_with(&serve(body.clone(), true, 1000), &dest, &sha, size, 16_384, 4, &no, &mut |_| {}).is_err());
        assert!(dir.join("broken.gguf.part").exists() && !dest.exists());
        // A wrong hash leaves nothing behind.
        let dest = dir.join("bad.gguf");
        assert!(download_with(&serve(body.clone(), true, 0), &dest, &"0".repeat(64), size, 16_384, 4, &no, &mut |_| {}).is_err());
        assert!(!dest.exists() && !dir.join("bad.gguf.part").exists() && !dir.join("bad.gguf.part.done").exists());
        // Files smaller than a chunk come in one request.
        let dest = dir.join("small.gguf");
        download(&serve(body.clone(), true, 0), &dest, &sha, size, &no, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        let _ = std::fs::remove_dir_all(dir);
    }
}
