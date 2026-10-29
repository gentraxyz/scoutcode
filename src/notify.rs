//! Windows notifications ("toasts") for when Scout finishes or needs you while its window is in the background.

/// Who the toasts come from. Registered under HKCU so Windows shows "Scout" and its icon on them.
#[cfg(windows)]
const APP_ID: &str = "ScoutCode.Scout";

/// Shows a toast without blocking the window. Failures are silent: a missed toast isn't worth an error.
pub fn toast(title: &str, body: &str) {
    #[cfg(windows)]
    {
        let (title, body) = (title.to_string(), body.to_string());
        std::thread::spawn(move || {
            register();
            let _ = show(&title, &body);
        });
    }
    #[cfg(not(windows))]
    let _ = (title, body);
}

/// Makes the app id known to Windows, once per run. Unpackaged apps need this before their toasts appear.
#[cfg(windows)]
fn register() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        let icon = crate::config::home().join("scout-icon.png");
        if !icon.exists() {
            let _ = std::fs::create_dir_all(crate::config::home());
            let _ = std::fs::write(&icon, include_bytes!("../assets/icon.png"));
        }
        let key = format!(r"HKCU\Software\Classes\AppUserModelId\{APP_ID}");
        for (name, value) in [("DisplayName", "Scout".to_string()), ("IconUri", icon.to_string_lossy().to_string())] {
            let mut c = std::process::Command::new("reg");
            c.args(["add", &key, "/v", name, "/t", "REG_SZ", "/d", &value, "/f"]);
            crate::tools::no_window(&mut c);
            let _ = c.output();
        }
    });
}

#[cfg(windows)]
fn show(title: &str, body: &str) -> windows::core::Result<()> {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::HSTRING;
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
        esc(title),
        esc(body)
    )))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)
}
