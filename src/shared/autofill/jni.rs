//! The JNI half: what `dev.pandora.Autofill` calls from the service.
//!
//! Every answer is a string or nothing, the same rule `shared/android.rs` follows
//! in the other direction. The one thing kept from the first call is the VM and
//! the application context: a fill request can start this process before any
//! activity exists, so `android_main` may never run and leave the app's paths set.

use super::{self as autofill, State};
use jni::objects::{Global, JClass, JObject, JString};
use jni::sys::jstring;
use jni::{jni_sig, jni_str, Env, EnvUnowned, JavaVM};
use std::sync::OnceLock;
use zeroize::Zeroizing;

/// A reply, or null for "nothing to offer". JNI has those two answers, and a
/// caller parsing JSON gets no half-written object.
fn reply(env: &mut Env<'_>, text: Option<String>) -> jstring {
    match text.map(|text| JString::from_str(env, text)) {
        Some(Ok(text)) => text.into_raw(),
        _ => std::ptr::null_mut(),
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Option<String> {
    serde_json::to_string(value).ok()
}

/// What the first call leaves for the rest of the process: the VM, so a later
/// thread can attach, and the application context, so a directory can be reached
/// in a process whose first entry point is a fill request.
struct Service {
    vm: JavaVM,
    app: Global<JObject<'static>>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

impl Service {
    /// The app's private files directory, through the cached context: the one
    /// back-call this side makes, and the proof that the cache is usable.
    fn files(&self) -> Option<String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<String> {
                let dir = env
                    .call_method(
                        &self.app,
                        jni_str!("getFilesDir"),
                        jni_sig!("()Ljava/io/File;"),
                        &[],
                    )?
                    .l()?;
                let path = env
                    .call_method(
                        &dir,
                        jni_str!("getAbsolutePath"),
                        jni_sig!("()Ljava/lang/String;"),
                        &[],
                    )?
                    .l()?;
                let path = env.cast_local::<JString>(path)?;
                path.try_to_string(env)
            })
            .ok()
    }
}

/// Cache the VM and the context once, then read the app's directory through them.
/// Reading it is the point: caching something nothing calls proves nothing.
fn init(env: &mut Env<'_>, context: JObject<'_>) {
    if SERVICE.get().is_none() {
        match (env.get_java_vm(), env.new_global_ref(context)) {
            (Ok(vm), Ok(app)) => {
                let _ = SERVICE.set(Service { vm, app });
            }
            _ => log::warn!("autofill: no vm or context to cache"),
        }
    }
    match SERVICE.get().and_then(Service::files) {
        Some(dir) => log::info!("autofill: ready, files at {dir}"),
        None => log::warn!("autofill: ready, but the files directory is unreadable"),
    }
}

/// `Autofill.nativeInit(Context)`: a phone has no console, so a process that
/// starts without `android_main` would drop everything it says. The context is
/// kept here and nowhere else — nothing else on this side calls into Java.
#[no_mangle]
pub extern "system" fn Java_dev_pandora_Autofill_nativeInit<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
    context: JObject<'local>,
) {
    unowned
        .with_env(|env| -> jni::errors::Result<()> {
            android_logger::init_once(
                android_logger::Config::default()
                    .with_max_level(log::LevelFilter::Info)
                    .with_tag("pandora"),
            );
            init(env, context);
            Ok(())
        })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// `Autofill.nativeForget()`: the screen went off or the app locked, so the copy
/// goes now instead of at a frame that may never be painted.
#[no_mangle]
pub extern "system" fn Java_dev_pandora_Autofill_nativeForget<'local>(
    _unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    autofill::clear();
}

/// `Autofill.nativeState() : String` — `"ready"` or `"locked"`.
#[no_mangle]
pub extern "system" fn Java_dev_pandora_Autofill_nativeState<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jstring {
    let state = match autofill::state() {
        State::Ready => "ready",
        State::Locked => "locked",
    };
    unowned
        .with_env(|env| -> jni::errors::Result<jstring> { Ok(reply(env, Some(state.to_owned()))) })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// `Autofill.nativeQuery(String) : String` — the entries claiming the page, as
/// `Candidate` carries them: an id, a name, a username and the URL that matched.
#[no_mangle]
pub extern "system" fn Java_dev_pandora_Autofill_nativeQuery<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
    url: JString<'local>,
) -> jstring {
    unowned
        .with_env(|env| -> jni::errors::Result<jstring> {
            let page = url.try_to_string(env)?;
            Ok(reply(env, encode(&autofill::query(&page))))
        })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// `Autofill.nativeFill(String) : String` — `{"username","password"}` for one id,
/// or null. The only reply that carries a password, and only for the id the user
/// picked in the panel.
#[no_mangle]
pub extern "system" fn Java_dev_pandora_Autofill_nativeFill<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
    id: JString<'local>,
) -> jstring {
    unowned
        .with_env(|env| -> jni::errors::Result<jstring> {
            let id = id.try_to_string(env)?;
            let fill = autofill::reveal(&id).map(|(username, password)| {
                let payload = serde_json::json!({
                    "username": username,
                    "password": password.as_str(),
                });
                payload.to_string()
            });
            Ok(reply(env, fill))
        })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// `Autofill.nativeSave(name, username, password, url) : String` — `"queued"`
/// when the app has a box open to put it in, null when it has not. The service
/// cannot write: the vault lives on the app's own thread, so this queues and the
/// app lands it with the same `serve::create_login` the browser panel uses.
#[no_mangle]
pub extern "system" fn Java_dev_pandora_Autofill_nativeSave<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
    name: JString<'local>,
    username: JString<'local>,
    password: JString<'local>,
    url: JString<'local>,
) -> jstring {
    unowned
        .with_env(|env| -> jni::errors::Result<jstring> {
            let queued = autofill::queue(autofill::Pending {
                name: name.try_to_string(env)?,
                username: username.try_to_string(env)?,
                // Moved into `Zeroizing`, never copied: the buffer is wiped on drop.
                password: Zeroizing::new(password.try_to_string(env)?),
                url: url.try_to_string(env)?,
            });
            Ok(reply(env, queued.then(|| "queued".to_owned())))
        })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// `Autofill.nativeCodes() : String` — every seed by name, as `CodeInfo` carries
/// it. A listing never holds a code; this one is what lets the panel name the
/// rows before the user picks one.
#[no_mangle]
pub extern "system" fn Java_dev_pandora_Autofill_nativeCodes<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jstring {
    unowned
        .with_env(|env| -> jni::errors::Result<jstring> {
            Ok(reply(env, encode(&autofill::codes())))
        })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// `Autofill.nativeCode(String) : String` — `{"code","remaining"}` for one seed,
/// or null. The seed itself never crosses; the derivation stays here.
#[no_mangle]
pub extern "system" fn Java_dev_pandora_Autofill_nativeCode<'local>(
    mut unowned: EnvUnowned<'local>,
    _class: JClass<'local>,
    id: JString<'local>,
) -> jstring {
    unowned
        .with_env(|env| -> jni::errors::Result<jstring> {
            let id = id.try_to_string(env)?;
            let code = autofill::code(&id).map(|(code, remaining)| {
                serde_json::json!({ "code": code.as_str(), "remaining": remaining }).to_string()
            });
            Ok(reply(env, code))
        })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}
