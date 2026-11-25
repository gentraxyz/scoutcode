//! Asks WebView2 to give memory back while Scout's window is minimized: with the memory target set to Low, the webview
//! trims its caches and working set (the UI keeps running, so chats carry on and notifications still come). It goes back
//! to Normal as soon as the window is restored.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Runtime, WebviewWindow};

/// Whether the target is Low now, so a resize that doesn't change minimized-ness doesn't ask again.
static LOW: AtomicBool = AtomicBool::new(false);

/// Call on the main window's resize events.
pub fn on_resized<R: Runtime>(window: &WebviewWindow<R>) {
    let minimized = window.is_minimized().unwrap_or(false);
    if LOW.swap(minimized, Ordering::Relaxed) == minimized {
        return;
    }
    set_low(window, minimized);
}

#[cfg(windows)]
fn set_low<R: Runtime>(window: &WebviewWindow<R>, low: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use windows::core::Interface;
    let _ = window.with_webview(move |w| unsafe {
        // Older WebView2 runtimes don't have this interface; then there's nothing to do.
        let Ok(core) = w.controller().CoreWebView2() else { return };
        let Ok(core) = core.cast::<ICoreWebView2_19>() else { return };
        let level = if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
        let _ = core.SetMemoryUsageTargetLevel(level);
    });
}

#[cfg(not(windows))]
fn set_low<R: Runtime>(_window: &WebviewWindow<R>, _low: bool) {}
