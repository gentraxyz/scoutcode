//! Keeps Scout one exe. With the gnullvm toolchain the webview crates link `WebView2Loader.dll` when the exe starts, so
//! it would have to sit beside the exe, and self-updates only replace the exe. These definitions take the place of that
//! import: the DLL is built into the exe and loaded from ~/.scoutcode/runtime the first time the webview needs it.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::OnceLock;

type Hresult = i32;
const E_FAIL: Hresult = 0x8000_4005_u32 as i32;

#[cfg(target_arch = "x86_64")]
const DLL: &[u8] = include_bytes!("../assets/webview2/x64/WebView2Loader.dll");
#[cfg(target_arch = "aarch64")]
const DLL: &[u8] = include_bytes!("../assets/webview2/arm64/WebView2Loader.dll");

unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

/// Writes the DLL unless the same bytes are there already. Named per architecture, since the folder can be roamed.
fn unpack() -> std::io::Result<PathBuf> {
    let dir = crate::config::home().join("runtime");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("WebView2Loader-{}.dll", std::env::consts::ARCH));
    if std::fs::read(&path).map_or(true, |b| b != DLL) {
        // Another Scout may have it loaded, in which case it can't be replaced and a new name is used.
        let tmp = dir.join(format!("WebView2Loader-{}.{}.tmp", std::env::consts::ARCH, std::process::id()));
        std::fs::write(&tmp, DLL)?;
        if std::fs::rename(&tmp, &path).is_err() {
            let other = dir.join(format!("WebView2Loader-{}-{}.dll", std::env::consts::ARCH, std::process::id()));
            std::fs::rename(&tmp, &other)?;
            return Ok(other);
        }
    }
    Ok(path)
}

fn module() -> *mut c_void {
    static MODULE: OnceLock<usize> = OnceLock::new();
    *MODULE.get_or_init(|| {
        let Ok(path) = unpack() else { return 0 };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        unsafe { LoadLibraryW(wide.as_ptr()) as usize }
    }) as *mut c_void
}

macro_rules! forward {
    ($name:ident($($arg:ident: $ty:ty),*)) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn $name($($arg: $ty),*) -> Hresult {
            let m = module();
            if m.is_null() {
                return E_FAIL;
            }
            let f = unsafe { GetProcAddress(m, concat!(stringify!($name), "\0").as_ptr()) };
            if f.is_null() {
                return E_FAIL;
            }
            let f: unsafe extern "system" fn($($ty),*) -> Hresult = unsafe { std::mem::transmute(f) };
            unsafe { f($($arg),*) }
        }
    };
}

forward!(CompareBrowserVersions(a: *const u16, b: *const u16, result: *mut i32));
forward!(CreateCoreWebView2Environment(handler: *mut c_void));
forward!(CreateCoreWebView2EnvironmentWithOptions(folder: *const u16, data: *const u16, options: *mut c_void, handler: *mut c_void));
forward!(GetAvailableCoreWebView2BrowserVersionString(folder: *const u16, version: *mut *mut u16));
forward!(GetAvailableCoreWebView2BrowserVersionStringWithOptions(folder: *const u16, options: *mut c_void, version: *mut *mut u16));
