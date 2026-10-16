//! Update check and banner for the Android port.
//!
//! On launch, and again when the app comes back to the foreground (at most every
//! [`RECHECK_SECS`]), a background thread asks GitHub for the latest release of this fork
//! (`api.github.com/repos/<repo>/releases/latest`). When it is newer than this build, a banner
//! floats at the bottom of the screen with **Download** (opens the APK link, the browser downloads
//! it and Android offers to install it over this one — same signing key), **What's new** (the
//! release page) and **Later** (hides this version until a newer one appears).
//!
//! The request goes through Android's own `HttpURLConnection` over JNI, so the system TLS stack
//! is used and the APK carries no HTTP or TLS library (it would add about 1 MB). Nothing is
//! downloaded or installed by the app itself, and nothing is sent except the request.
//!
//! Versions: this build is `<upstream version>-android.<PORT_REVISION>`, the tag scheme of the
//! releases (`v0.7.0-android.2`). Release tags are compared as `(major, minor, patch, revision)`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// How long after a check another one may run (on returning to the app).
pub const RECHECK_SECS: u64 = 30 * 60;

/// A newer release than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The tag, such as `v0.7.0-android.3`.
    pub tag: String,
    /// The release page.
    pub page: String,
    /// The arm64 APK to download, when the release has one (else the page is used).
    pub apk: Option<String>,
}

/// `v0.7.0-android.2` → `(0, 7, 0, 2)`; an upstream-style tag without the suffix is revision 0.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let (core, rest) = match v.split_once('-') {
        Some((c, r)) => (c, Some(r)),
        None => (v, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    if parts.next().is_some() {
        return None;
    }
    let revision = match rest {
        None => 0,
        Some(r) => r.strip_prefix("android.")?.parse().ok()?,
    };
    Some((major, minor, patch, revision))
}

/// Whether `latest` is newer than `current`. Anything that does not parse is never newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// Pick what the banner needs out of the GitHub API answer.
pub fn release_from_json(json: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("bad answer from GitHub: {e}"))?;
    let tag = v["tag_name"].as_str().ok_or("the answer has no tag_name")?.to_string();
    let page = v["html_url"].as_str().unwrap_or_default().to_string();
    let apk = v["assets"].as_array().and_then(|assets| {
        let urls: Vec<(&str, &str)> = assets
            .iter()
            .filter_map(|a| Some((a["name"].as_str()?, a["browser_download_url"].as_str()?)))
            .filter(|(n, _)| n.ends_with(".apk"))
            .collect();
        urls.iter().find(|(n, _)| n.contains("arm64")).or(urls.first()).map(|(_, u)| u.to_string())
    });
    Ok(Release { tag, page, apk })
}

enum State {
    Idle,
    Running(Receiver<Result<Release, String>>),
}

/// The check's state and the banner.
pub struct Updater {
    repo: &'static str,
    app_name: &'static str,
    current: String,
    /// Where the "Later" choice is kept, so a hidden version stays hidden after a restart.
    dismissed_file: PathBuf,
    dismissed: Option<String>,
    state: State,
    last_check: Option<Instant>,
    available: Option<Release>,
}

impl Updater {
    pub fn new(repo: &'static str, app_name: &'static str, current: String, data_dir: &std::path::Path) -> Self {
        let dismissed_file = data_dir.join("update-dismissed.txt");
        let dismissed = std::fs::read_to_string(&dismissed_file).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Self { repo, app_name, current, dismissed_file, dismissed, state: State::Idle, last_check: None, available: None }
    }

    /// This build's version, as the release tags write it (without the `v`).
    pub fn current(&self) -> &str {
        &self.current
    }

    /// Start a check unless one ran within [`RECHECK_SECS`] or is running.
    pub fn check_if_due(&mut self, ctx: &egui::Context) {
        if matches!(self.state, State::Running(_)) {
            return;
        }
        if self.last_check.is_some_and(|t| t.elapsed() < Duration::from_secs(RECHECK_SECS)) {
            return;
        }
        self.last_check = Some(Instant::now());
        let (tx, rx) = std::sync::mpsc::channel();
        let url = format!("https://api.github.com/repos/{}/releases/latest", self.repo);
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let result = fetch(&url).and_then(|body| release_from_json(&body));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.state = State::Running(rx);
        }
    }

    /// Pick up a finished check (every frame).
    pub fn poll(&mut self) {
        let State::Running(rx) = &self.state else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the check stopped".into()),
        };
        self.state = State::Idle;
        match result {
            Ok(r) if is_newer(&r.tag, &self.current) => {
                log::info!("update available: {} (this is {})", r.tag, self.current);
                self.available = Some(r);
            }
            Ok(r) => {
                log::info!("up to date: latest release {} (this is {})", r.tag, self.current);
                self.available = None;
            }
            // Offline, rate-limited, no releases yet: nothing to tell the user, try next time.
            Err(e) => log::warn!("update check failed: {e}"),
        }
    }

    /// The banner, when a newer release is waiting and was not put off.
    pub fn banner(&mut self, ctx: &egui::Context) {
        let Some(r) = self.available.clone() else { return };
        if self.dismissed.as_deref() == Some(r.tag.as_str()) {
            return;
        }
        let new = r.tag.trim_start_matches(['v', 'V']).to_string();
        let mut action: Option<&str> = None;
        egui::Area::new(egui::Id::new("android_update_banner"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -28.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(18, 14)).corner_radius(12).show(ui, |ui| {
                    ui.spacing_mut().button_padding = egui::vec2(16.0, 10.0);
                    ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(format!("{} {new} is available", self.app_name)).strong().size(16.0));
                            ui.label(egui::RichText::new(format!("You have {}. Your files and settings are kept.", self.current)).weak());
                        });
                        ui.add_space(24.0);
                        if ui.add(egui::Button::new(egui::RichText::new("Download").strong())).clicked() {
                            action = Some("download");
                        }
                        if ui.button("What's new").clicked() {
                            action = Some("notes");
                        }
                        if ui.button("Later").clicked() {
                            action = Some("later");
                        }
                    });
                });
            });
        match action {
            Some("download") => open(r.apk.as_deref().unwrap_or(&r.page)),
            Some("notes") => open(&r.page),
            Some("later") => {
                let _ = std::fs::write(&self.dismissed_file, &r.tag);
                self.dismissed = Some(r.tag);
            }
            _ => {}
        }
    }
}

fn open(url: &str) {
    if let Err(e) = platform::open_url(url) {
        log::warn!("couldn't open {url}: {e}");
    }
}

#[cfg(target_os = "android")]
fn fetch(url: &str) -> Result<String, String> {
    platform::http_get(url)
}

#[cfg(not(target_os = "android"))]
fn fetch(_url: &str) -> Result<String, String> {
    Err("update check is Android-only".into())
}

/// Set once by `android_main`, before the first check.
#[cfg(target_os = "android")]
pub fn set_app(app: &winit::platform::android::activity::AndroidApp) {
    platform::set_app(app);
}

#[cfg(target_os = "android")]
mod platform {
    use std::sync::Mutex;

    use jni::objects::{JObject, JString, JValue};
    use jni::signature::RuntimeMethodSignature;
    use jni::strings::JNIStr;
    use winit::platform::android::activity::AndroidApp;

    static APP: Mutex<Option<AndroidApp>> = Mutex::new(None);

    /// Largest answer read (the release JSON is a few kB).
    const MAX_BODY: usize = 2 * 1024 * 1024;

    pub fn set_app(app: &AndroidApp) {
        *APP.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(app.clone());
    }

    fn name(n: &'static std::ffi::CStr) -> Result<&'static JNIStr, jni::errors::Error> {
        JNIStr::from_cstr(n).ok_or(jni::errors::Error::NullPtr("JNI name"))
    }

    fn sig(s: &str) -> Result<RuntimeMethodSignature, jni::errors::Error> {
        RuntimeMethodSignature::from_str(s)
    }

    fn with_activity<T>(f: impl FnOnce(&mut jni::Env, &JObject) -> jni::errors::Result<T>) -> Result<T, String> {
        let app = APP.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().ok_or("no Android activity")?;
        // SAFETY: `vm_as_ptr` is the JavaVM of this process, which outlives every Activity.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
        vm.attach_current_thread(|env| -> jni::errors::Result<T> {
            // SAFETY: the Activity's own reference, valid while its `AndroidApp` is; `JObject`
            // does not own it, so the platform's reference is never deleted here.
            let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr() as jni::sys::jobject) };
            let out = f(env, &activity);
            if out.is_err() && env.exception_check() {
                env.exception_clear();
            }
            out
        })
        .map_err(|e| e.to_string())
    }

    /// `ACTION_VIEW` on `url`: the browser opens it (an APK link downloads).
    pub fn open_url(url: &str) -> Result<(), String> {
        with_activity(|env, activity| {
            let jurl = JString::from_str(env, url)?;
            let uri = env
                .call_static_method(name(c"android/net/Uri")?, name(c"parse")?, sig("(Ljava/lang/String;)Landroid/net/Uri;")?.method_signature(), &[JValue::Object(&jurl)])?
                .l()?;
            let action = JString::from_str(env, "android.intent.action.VIEW")?;
            let intent = env.new_object(
                name(c"android/content/Intent")?,
                sig("(Ljava/lang/String;Landroid/net/Uri;)V")?.method_signature(),
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            env.call_method(activity, name(c"startActivity")?, sig("(Landroid/content/Intent;)V")?.method_signature(), &[JValue::Object(&intent)])?;
            Ok(())
        })
    }

    /// GET `url` with `HttpURLConnection` (system TLS) and return the body as text.
    pub fn http_get(url: &str) -> Result<String, String> {
        with_activity(|env, _activity| {
            let jurl = JString::from_str(env, url)?;
            let u = env.new_object(name(c"java/net/URL")?, sig("(Ljava/lang/String;)V")?.method_signature(), &[JValue::Object(&jurl)])?;
            let conn = env.call_method(&u, name(c"openConnection")?, sig("()Ljava/net/URLConnection;")?.method_signature(), &[])?.l()?;
            let set_int = sig("(I)V")?;
            env.call_method(&conn, name(c"setConnectTimeout")?, set_int.method_signature(), &[JValue::Int(10_000)])?;
            env.call_method(&conn, name(c"setReadTimeout")?, set_int.method_signature(), &[JValue::Int(10_000)])?;
            let set_prop = sig("(Ljava/lang/String;Ljava/lang/String;)V")?;
            for (k, v) in [("Accept", "application/vnd.github+json"), ("User-Agent", "craft-android-update-check")] {
                let (k, v) = (JString::from_str(env, k)?, JString::from_str(env, v)?);
                env.call_method(&conn, name(c"setRequestProperty")?, set_prop.method_signature(), &[JValue::Object(&k), JValue::Object(&v)])?;
            }
            let code = env.call_method(&conn, name(c"getResponseCode")?, sig("()I")?.method_signature(), &[])?.i()?;
            if code != 200 {
                env.call_method(&conn, name(c"disconnect")?, sig("()V")?.method_signature(), &[])?;
                return Err(jni::errors::Error::NullPtr(if code == 404 { "no release yet (HTTP 404)" } else { "GitHub answered with an error status" }));
            }
            let stream = env.call_method(&conn, name(c"getInputStream")?, sig("()Ljava/io/InputStream;")?.method_signature(), &[])?.l()?;
            let chunk = env.new_byte_array(16 * 1024)?;
            let mut buf = vec![0i8; 16 * 1024];
            let mut body: Vec<u8> = Vec::new();
            let read_sig = sig("([B)I")?;
            loop {
                let n = env.call_method(&stream, name(c"read")?, read_sig.method_signature(), &[JValue::Object(&chunk)])?.i()?;
                if n < 0 {
                    break;
                }
                let n = (n as usize).min(buf.len());
                env.get_byte_array_region(&chunk, 0, &mut buf[..n])?;
                body.extend(buf[..n].iter().map(|b| *b as u8));
                if body.len() > MAX_BODY {
                    break;
                }
            }
            let _ = env.call_method(&stream, name(c"close")?, sig("()V")?.method_signature(), &[]);
            let _ = env.call_method(&conn, name(c"disconnect")?, sig("()V")?.method_signature(), &[]);
            Ok(String::from_utf8_lossy(&body).into_owned())
        })
    }
}

#[cfg(not(target_os = "android"))]
mod platform {
    pub fn open_url(_url: &str) -> Result<(), String> {
        Err("Android-only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_with_the_port_revision() {
        assert_eq!(parse_version("v0.7.0-android.2"), Some((0, 7, 0, 2)));
        assert_eq!(parse_version("0.8.0"), Some((0, 8, 0, 0)));
        assert!(is_newer("v0.7.0-android.3", "0.7.0-android.2"));
        assert!(is_newer("v0.8.0-android.1", "0.7.0-android.9"));
        assert!(!is_newer("v0.7.0-android.2", "0.7.0-android.2"));
        assert!(!is_newer("v0.7.0-android.1", "0.7.0-android.2"));
        assert!(!is_newer("nightly", "0.7.0-android.2"));
        assert!(!is_newer("v0.7.0-beta.1", "0.6.0-android.1"), "a foreign suffix is not a release of this port");
    }

    #[test]
    fn the_arm64_apk_is_picked() {
        let json = r#"{"tag_name":"v0.7.0-android.3","html_url":"https://github.com/x/y/releases/tag/v0.7.0-android.3",
            "assets":[{"name":"SHA256SUMS.txt","browser_download_url":"https://e/sums"},
                      {"name":"gridcraft-0.7.0-android.3-arm64-v8a.apk","browser_download_url":"https://e/a.apk"}]}"#;
        let r = release_from_json(json).unwrap();
        assert_eq!(r.tag, "v0.7.0-android.3");
        assert_eq!(r.apk.as_deref(), Some("https://e/a.apk"));
        assert!(release_from_json("{}").is_err());
    }
}
