//! The Android system services the desktop gets from `arboard`, `webbrowser` and the OS file
//! association, reached through JNI on the running Activity.
//!
//! - **System clipboard**: copy publishes plain text (and HTML when the app has it) through
//!   `ClipboardManager`, paste reads it back, so cells move between GridCraft and any other app.
//! - **Open links**: an `ACTION_VIEW` intent, so the About/Backstage links open in the browser.
//! - **Open with**: a workbook shared or opened from another app (Files, a chat app, email) arrives
//!   as the launch intent's `content://` URI; its bytes are read through the `ContentResolver`
//!   and handed to the app's inbox, exactly like a file picked in the app.
//!
//! Every call is fallible and returns `Err(String)`; nothing here panics, and a failed JNI call
//! clears its pending Java exception so the next call starts clean.

#![cfg(target_os = "android")]

use std::sync::Mutex;

use jni::objects::{JObject, JString, JValue};
use jni::signature::RuntimeMethodSignature;
use jni::strings::JNIStr;
use winit::platform::android::activity::AndroidApp;

/// The running Activity's app handle, set once by `android_main`.
static APP: Mutex<Option<AndroidApp>> = Mutex::new(None);

/// Largest file accepted through "Open with" (workbooks bigger than this are not phone/tablet work).
const MAX_SHARED_FILE: usize = 256 * 1024 * 1024;

pub fn set_app(app: &AndroidApp) {
    *APP.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(app.clone());
}

fn app() -> Result<AndroidApp, String> {
    APP.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().ok_or_else(|| "no Android activity".to_string())
}

fn name(n: &'static std::ffi::CStr) -> Result<&'static JNIStr, jni::errors::Error> {
    JNIStr::from_cstr(n).ok_or(jni::errors::Error::NullPtr("JNI name"))
}

fn sig(s: &str) -> Result<RuntimeMethodSignature, jni::errors::Error> {
    RuntimeMethodSignature::from_str(s)
}

/// Run `f` with a JNI env and this Activity, mapping every failure to a readable string.
fn with_activity<T>(f: impl FnOnce(&mut jni::Env, &JObject) -> jni::errors::Result<T>) -> Result<T, String> {
    let app = app()?;
    // SAFETY: `vm_as_ptr` is the JavaVM of this process, which outlives every Activity.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    vm.attach_current_thread(|env| -> jni::errors::Result<T> {
        // SAFETY: the Activity's own reference, valid while its `AndroidApp` is; `JObject` does
        // not own it (no `Drop`), so the platform's reference is never deleted here.
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr() as jni::sys::jobject) };
        let out = f(env, &activity);
        if out.is_err() && env.exception_check() {
            env.exception_clear();
        }
        out
    })
    .map_err(|e| e.to_string())
}

fn clipboard_manager<'l>(env: &mut jni::Env<'l>, activity: &JObject) -> jni::errors::Result<JObject<'l>> {
    let service = JString::from_str(env, "clipboard")?;
    env.call_method(activity, name(c"getSystemService")?, sig("(Ljava/lang/String;)Ljava/lang/Object;")?.method_signature(), &[JValue::Object(&service)])?
        .l()
}

/// Put `text` (and `html`, when given) on the system clipboard.
pub fn copy(text: &str, html: Option<&str>) -> Result<(), String> {
    with_activity(|env, activity| {
        let manager = clipboard_manager(env, activity)?;
        let label = JString::from_str(env, "GridCraft")?;
        let jtext = JString::from_str(env, text)?;
        let clip_class = name(c"android/content/ClipData")?;
        let clip = match html {
            Some(html) => {
                let jhtml = JString::from_str(env, html)?;
                env.call_static_method(
                    clip_class,
                    name(c"newHtmlText")?,
                    sig("(Ljava/lang/CharSequence;Ljava/lang/CharSequence;Ljava/lang/String;)Landroid/content/ClipData;")?.method_signature(),
                    &[JValue::Object(&label), JValue::Object(&jtext), JValue::Object(&jhtml)],
                )?
            }
            None => env.call_static_method(
                clip_class,
                name(c"newPlainText")?,
                sig("(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;")?.method_signature(),
                &[JValue::Object(&label), JValue::Object(&jtext)],
            )?,
        }
        .l()?;
        env.call_method(&manager, name(c"setPrimaryClip")?, sig("(Landroid/content/ClipData;)V")?.method_signature(), &[JValue::Object(&clip)])?;
        Ok(())
    })
}

/// The first clipboard item, as `(plain text, html)`. `None` when the clipboard is empty.
pub fn paste() -> Result<Option<(String, Option<String>)>, String> {
    with_activity(|env, activity| {
        let manager = clipboard_manager(env, activity)?;
        let clip = env.call_method(&manager, name(c"getPrimaryClip")?, sig("()Landroid/content/ClipData;")?.method_signature(), &[])?.l()?;
        if clip.is_null() {
            return Ok(None);
        }
        let count = env.call_method(&clip, name(c"getItemCount")?, sig("()I")?.method_signature(), &[])?.i()?;
        if count < 1 {
            return Ok(None);
        }
        let item = env
            .call_method(&clip, name(c"getItemAt")?, sig("(I)Landroid/content/ClipData$Item;")?.method_signature(), &[JValue::Int(0)])?
            .l()?;
        let chars = env
            .call_method(&item, name(c"coerceToText")?, sig("(Landroid/content/Context;)Ljava/lang/CharSequence;")?.method_signature(), &[JValue::Object(activity)])?
            .l()?;
        if chars.is_null() {
            return Ok(None);
        }
        let jtext = env.call_method(&chars, name(c"toString")?, sig("()Ljava/lang/String;")?.method_signature(), &[])?.l()?;
        let text = env.cast_local::<JString>(jtext)?.try_to_string(env)?;
        let jhtml = env.call_method(&item, name(c"getHtmlText")?, sig("()Ljava/lang/String;")?.method_signature(), &[])?.l()?;
        let html = if jhtml.is_null() { None } else { Some(env.cast_local::<JString>(jhtml)?.try_to_string(env)?) };
        Ok(Some((text, html)))
    })
}

/// Open `url` in the default browser (or whatever handles it).
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

/// The file this Activity was launched with ("Open with" / "Share"), as `(display name, bytes)`.
/// `Ok(None)` for an ordinary launch from the home screen.
pub fn launch_file() -> Result<Option<(String, Vec<u8>)>, String> {
    with_activity(|env, activity| {
        let intent = env.call_method(activity, name(c"getIntent")?, sig("()Landroid/content/Intent;")?.method_signature(), &[])?.l()?;
        if intent.is_null() {
            return Ok(None);
        }
        let mut uri = env.call_method(&intent, name(c"getData")?, sig("()Landroid/net/Uri;")?.method_signature(), &[])?.l()?;
        if uri.is_null() {
            // ACTION_SEND carries the file as the EXTRA_STREAM parcelable instead.
            let key = JString::from_str(env, "android.intent.extra.STREAM")?;
            uri = env
                .call_method(&intent, name(c"getParcelableExtra")?, sig("(Ljava/lang/String;)Landroid/os/Parcelable;")?.method_signature(), &[JValue::Object(&key)])?
                .l()?;
        }
        if uri.is_null() {
            return Ok(None);
        }
        let resolver = env.call_method(activity, name(c"getContentResolver")?, sig("()Landroid/content/ContentResolver;")?.method_signature(), &[])?.l()?;
        let display = display_name(env, &resolver, &uri).unwrap_or(None);
        let display = match display {
            Some(n) => n,
            None => {
                let seg = env.call_method(&uri, name(c"getLastPathSegment")?, sig("()Ljava/lang/String;")?.method_signature(), &[])?.l()?;
                if seg.is_null() { "Shared workbook.xlsx".to_string() } else { env.cast_local::<JString>(seg)?.try_to_string(env)? }
            }
        };
        let stream = env
            .call_method(&resolver, name(c"openInputStream")?, sig("(Landroid/net/Uri;)Ljava/io/InputStream;")?.method_signature(), &[JValue::Object(&uri)])?
            .l()?;
        if stream.is_null() {
            return Ok(None);
        }
        let chunk = env.new_byte_array(64 * 1024)?;
        let mut buf = vec![0i8; 64 * 1024];
        let mut bytes: Vec<u8> = Vec::new();
        let read_sig = sig("([B)I")?;
        loop {
            let n = env.call_method(&stream, name(c"read")?, read_sig.method_signature(), &[JValue::Object(&chunk)])?.i()?;
            if n < 0 {
                break;
            }
            let n = (n as usize).min(buf.len());
            env.get_byte_array_region(&chunk, 0, &mut buf[..n])?;
            bytes.extend(buf[..n].iter().map(|b| *b as u8));
            if bytes.len() > MAX_SHARED_FILE {
                let _ = env.call_method(&stream, name(c"close")?, sig("()V")?.method_signature(), &[]);
                return Err(jni::errors::Error::NullPtr("shared file is larger than 256 MB"));
            }
        }
        let _ = env.call_method(&stream, name(c"close")?, sig("()V")?.method_signature(), &[]);
        Ok(Some((display, bytes)))
    })
}

/// `OpenableColumns.DISPLAY_NAME` for a `content://` URI, when the provider reports one.
fn display_name(env: &mut jni::Env, resolver: &JObject, uri: &JObject) -> jni::errors::Result<Option<String>> {
    let null = JObject::null();
    let cursor = env
        .call_method(
            resolver,
            name(c"query")?,
            sig("(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;")?.method_signature(),
            &[JValue::Object(uri), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null)],
        )?
        .l()?;
    if cursor.is_null() {
        return Ok(None);
    }
    let mut out = None;
    if env.call_method(&cursor, name(c"moveToFirst")?, sig("()Z")?.method_signature(), &[])?.z()? {
        let column = JString::from_str(env, "_display_name")?;
        let index = env.call_method(&cursor, name(c"getColumnIndex")?, sig("(Ljava/lang/String;)I")?.method_signature(), &[JValue::Object(&column)])?.i()?;
        if index >= 0 {
            let value = env.call_method(&cursor, name(c"getString")?, sig("(I)Ljava/lang/String;")?.method_signature(), &[JValue::Int(index)])?.l()?;
            if !value.is_null() {
                out = Some(env.cast_local::<JString>(value)?.try_to_string(env)?);
            }
        }
    }
    let _ = env.call_method(&cursor, name(c"close")?, sig("()V")?.method_signature(), &[]);
    Ok(out)
}
