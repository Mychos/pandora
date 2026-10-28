//! JNI plumbing into the one Kotlin class that owns the keystore prompt, the picker,
//! the clipboard, the browser and the token; `cfg(target_os = "android")` only.

use android_activity::AndroidApp;
use jni::objects::{JByteArray, JClass, JObject, JString, JValue, JValueOwned};
use jni::refs::LoaderContext;
use jni::strings::JNIString;
use jni::{jni_sig, jni_str, Env, JavaVM};
use std::sync::{Mutex, OnceLock};

/// The activity every platform call needs, handed to us once in `android_main`.
static ACTIVITY: OnceLock<AndroidApp> = OnceLock::new();

pub fn init(app: &AndroidApp) {
    let _ = ACTIVITY.set(app.clone());
}

/// Attach this thread to the JVM and run `job`; frame and unlock worker both come here.
fn call<T>(job: impl FnOnce(&mut Env) -> jni::errors::Result<T>) -> Result<T, String> {
    let Some(app) = ACTIVITY.get() else {
        return Err("the app has no activity".to_owned());
    };
    // SAFETY: the pointer is the activity's own JVM and lives as long as the process.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    vm.attach_current_thread(job)
        .map_err(|e| format!("android: {e}"))
}

/// `FindClass` fails on the native thread (no Java frame, no class loader), so the
/// lookup goes through the activity object, which carries one.
fn class<'local>(env: &mut Env<'local>) -> jni::errors::Result<JClass<'local>> {
    let Some(app) = ACTIVITY.get() else {
        return Err(jni::errors::Error::NullPtr("the app has no activity"));
    };
    let raw: jni::sys::jobject = app.activity_as_ptr() as _;
    // SAFETY: an unowned global reference that lives as long as the activity
    // handle does — deliberately not wrapped in `Global`, which would delete it.
    let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
    LoaderContext::FromObject(activity.as_ref()).load_class(
        env,
        jni_str!("dev.pandora.Pandora"),
        false,
    )
}

fn text_of(env: &mut Env, value: JValueOwned) -> jni::errors::Result<Option<String>> {
    let obj = value.l()?;
    if obj.is_null() {
        return Ok(None);
    }
    let text = env.cast_local::<JString>(obj)?;
    Ok(Some(text.try_to_string(env)?))
}

/// `Pandora.<name>()` answering a Boolean; anything short of a plain yes is a no.
pub fn flag(name: &str) -> bool {
    let name = JNIString::new(name);
    call(|env| {
        let class = class(env)?;
        Ok(env
            .call_static_method(class, &name, jni_sig!("()Z"), &[])?
            .z()?)
    })
    .unwrap_or(false)
}

pub fn foreground() -> bool {
    flag("foreground")
}

pub fn take_system_lock() -> bool {
    flag("takeSystemLock")
}

pub fn take_resumed() -> bool {
    flag("takeResumed")
}

/// The activity went to the background since the last frame: the only moment a
/// phone is still ours to write to.
pub fn take_paused() -> bool {
    flag("takePaused")
}

/// `Pandora.<name>()` answering a number; anything unreadable is zero.
fn number(name: &str) -> f32 {
    let name = JNIString::new(name);
    call(|env| {
        let class = class(env)?;
        env.call_static_method(class, &name, jni_sig!("()F"), &[])?
            .f()
    })
    .unwrap_or(0.0)
}

/// The phone's navigation bar height in points: the app draws behind the system bars,
/// so keeping clear of it is its own job; asked each frame because it moves.
pub fn footer() -> f32 {
    number("footer")
}

/// `Pandora.<name>() : String?`
pub fn text(name: &str) -> Result<Option<String>, String> {
    let name = JNIString::new(name);
    call(|env| {
        let class = class(env)?;
        let out = env.call_static_method(class, &name, jni_sig!("()Ljava/lang/String;"), &[])?;
        text_of(env, out)
    })
}

/// `Pandora.<name>(text) : String?`
pub fn text_with(name: &str, arg: &str) -> Result<Option<String>, String> {
    let name = JNIString::new(name);
    call(|env| {
        let class = class(env)?;
        let arg = env.new_string(arg)?;
        let out = env.call_static_method(
            class,
            &name,
            jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
            &[JValue::Object(&arg)],
        )?;
        text_of(env, out)
    })
}

/// The bytes behind a `byte[]` the class answered, or `None` for its `null`.
fn array_of(env: &mut Env, out: JValueOwned) -> jni::errors::Result<Option<Vec<u8>>> {
    let obj = out.l()?;
    if obj.is_null() {
        return Ok(None);
    }
    let array = env.cast_local::<JByteArray>(obj)?;
    let mut bytes = vec![0i8; array.len(env)?];
    array.get_region(env, 0, &mut bytes)?;
    Ok(Some(
        bytes.into_iter().map(|b| b as u8).collect::<Vec<u8>>(),
    ))
}

/// `Pandora.<name>() : byte[]`. A camera frame, which is a few hundred
/// kilobytes of pixels.
pub fn bytes(name: &str) -> Result<Option<Vec<u8>>, String> {
    let name = JNIString::new(name);
    call(|env| {
        let class = class(env)?;
        let out = env.call_static_method(class, &name, jni_sig!("()[B"), &[])?;
        array_of(env, out)
    })
}

/// `Pandora.<name>(text) : byte[]`. A whole file the class kept for us: one
/// version out of the folder mirror.
pub fn bytes_with(name: &str, arg: &str) -> Result<Option<Vec<u8>>, String> {
    let name = JNIString::new(name);
    call(|env| {
        let class = class(env)?;
        let arg = env.new_string(arg)?;
        let out = env.call_static_method(
            class,
            &name,
            jni_sig!("(Ljava/lang/String;)[B"),
            &[JValue::Object(&arg)],
        )?;
        array_of(env, out)
    })
}

/// `Pandora.<name>(text)`
pub fn send(name: &str, arg: &str) -> Result<(), String> {
    let name = JNIString::new(name);
    call(|env| {
        let class = class(env)?;
        let arg = env.new_string(arg)?;
        env.call_static_method(
            class,
            &name,
            jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::Object(&arg)],
        )?;
        Ok(())
    })
}

/// `Pandora.<name>(text, bytes) : String?`. A whole file handed over to the
/// class; anything it answers is its own sentence saying why it did not take it.
pub fn send_bytes_with(name: &str, arg: &str, bytes: &[u8]) -> Result<Option<String>, String> {
    let name = JNIString::new(name);
    call(|env| {
        let class = class(env)?;
        let arg = env.new_string(arg)?;
        let array = env.byte_array_from_slice(bytes)?;
        let out = env.call_static_method(
            class,
            &name,
            jni_sig!("(Ljava/lang/String;[B)Ljava/lang/String;"),
            &[JValue::Object(&arg), JValue::Object(&array)],
        )?;
        text_of(env, out)
    })
}

/// `Pandora.<name>()`
pub fn send_none(name: &str) -> Result<(), String> {
    let name = JNIString::new(name);
    call(|env| {
        let class = class(env)?;
        env.call_static_method(class, &name, jni_sig!("()V"), &[])?;
        Ok(())
    })
}

/// What the soft keyboard held, the last time this app looked at it.
#[derive(Default)]
struct Keyboard {
    /// The keyboard's own text: the next character the user types arrives as a
    /// change to *this*, not as a key event.
    shadow: String,
    /// The field egui had focused, so a move to another one is noticed.
    field: Option<egui::Id>,
}

static KEYBOARD: Mutex<Keyboard> = Mutex::new(Keyboard {
    shadow: String::new(),
    field: None,
});

/// winit and eframe deliver no IME characters on Android; `android-activity` publishes
/// the keyboard's editor state, read once a frame before drawing so events reach the focus.
/// One buffer serves every field: a field move or Enter empties it.
// ponytail: appends and backspaces are followed, not the cursor, and text already in a
// field is not editable; a full IME bridge belongs upstream in winit.
pub fn feed_keyboard(ctx: &egui::Context) {
    let Some(app) = ACTIVITY.get() else {
        return;
    };
    let typed = app.text_input_state().text;
    let Ok(mut keyboard) = KEYBOARD.lock() else {
        return;
    };
    let field = ctx.memory(|m| m.focused());
    if keyboard.field != field {
        keyboard.field = field;
        keyboard.shadow.clear();
        app.set_text_input_state(Default::default());
        return;
    }
    if field.is_none() || typed == keyboard.shadow {
        return;
    }
    let entered = typed.contains('\n');
    feed(ctx, &keyboard.shadow, &typed);
    keyboard.shadow = typed;
    if entered {
        keyboard.shadow.clear();
        app.set_text_input_state(Default::default());
    }
}

/// The difference between the two readings, as events egui understands.
fn feed(ctx: &egui::Context, before: &str, now: &str) {
    ctx.input_mut(|input| {
        if let Some(added) = now.strip_prefix(before) {
            if added.contains('\n') {
                press(input, egui::Key::Enter);
            } else if !added.is_empty() {
                input.events.push(egui::Event::Text(added.to_owned()));
            }
        } else if let Some(gone) = before.strip_prefix(now) {
            for _ in gone.chars() {
                press(input, egui::Key::Backspace);
            }
        } else {
            // A suggestion replaced the lot: select all and type over it.
            input.events.push(egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            });
            input.events.push(egui::Event::Text(now.to_owned()));
        }
    });
}

fn press(input: &mut egui::InputState, key: egui::Key) {
    input.events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}

/// Read a single document by its content uri.
pub fn document_read(uri: &str) -> Result<Option<Vec<u8>>, String> {
    bytes_with("documentRead", uri)
}

/// Write bytes to a single document by its content uri; Ok(()) on success.
pub fn document_write(uri: &str, bytes: &[u8]) -> Result<(), String> {
    let name = JNIString::new("documentWrite");
    let why: Option<String> = call(|env| {
        let class = class(env)?;
        let arg = env.new_string(uri)?;
        let arr = env.new_byte_array(bytes.len())?;
        if !bytes.is_empty() {
            let signed: Vec<i8> = bytes.iter().map(|b| *b as i8).collect();
            arr.set_region(env, 0, &signed)?;
        }
        let out = env.call_static_method(
            class,
            &name,
            jni_sig!("(Ljava/lang/String;[B)Ljava/lang/String;"),
            &[JValue::Object(&arg), JValue::Object(&arr)],
        )?;
        text_of(env, out)
    })?;
    if let Some(msg) = why {
        if !msg.is_empty() {
            return Err(msg);
        }
    }
    Ok(())
}

/// Ask the system to create (or pick) a document and return its content uri.
pub fn document_create(mime: &str, name: &str) -> Result<Option<String>, String> {
    let namej = JNIString::new("documentCreate");
    call(|env| {
        let class = class(env)?;
        let m = env.new_string(mime)?;
        let n = env.new_string(name)?;
        let out = env.call_static_method(
            class,
            &namej,
            jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
            &[JValue::Object(&m), JValue::Object(&n)],
        )?;
        text_of(env, out)
    })
}
