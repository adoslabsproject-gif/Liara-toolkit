//! Android: inizializza `ndk_context` (JavaVM + Application context). Tauri NON lo fa (non usa
//! android-activity), e senza di esso `cpal`/`oboe` (l'audio) panicano con "android context was not
//! initialized" — e il panic, attraversando il confine JNI, abbatte l'app. `JNI_OnLoad` cattura il
//! JavaVM al caricamento di libapp_lib.so; `init()` ricava l'Application context via
//! `ActivityThread.currentApplication()` e lo registra in `ndk_context`. Tutto in Rust, zero Kotlin.
use jni::sys::{jint, JavaVM as RawVM, JNI_VERSION_1_6};
use jni::JavaVM;
use std::ffi::c_void;
use std::sync::{Once, OnceLock};

static VM: OnceLock<JavaVM> = OnceLock::new();
static INIT: Once = Once::new();

/// Chiamata dalla JVM quando libapp_lib.so viene caricata → catturiamo il JavaVM.
#[no_mangle]
pub extern "C" fn JNI_OnLoad(vm: *mut RawVM, _reserved: *mut c_void) -> jint {
    if let Ok(vm) = unsafe { JavaVM::from_raw(vm) } {
        let _ = VM.set(vm);
    }
    JNI_VERSION_1_6
}

/// Registra (UNA sola volta) JavaVM + Application context in `ndk_context`. Va chiamata PRIMA di
/// qualunque uso dell'audio. Se qualcosa manca, esce in silenzio (l'app resta viva).
pub fn init() {
    INIT.call_once(|| {
        let Some(vm) = VM.get() else { return };
        let Ok(mut env) = vm.attach_current_thread_permanently() else { return };
        // android.app.ActivityThread.currentApplication() → l'Application (che è un Context)
        let app = match env
            .call_static_method(
                "android/app/ActivityThread",
                "currentApplication",
                "()Landroid/app/Application;",
                &[],
            )
            .and_then(|v| v.l())
        {
            Ok(o) if !o.is_null() => o,
            _ => return,
        };
        let Ok(global) = env.new_global_ref(&app) else { return };
        let ctx_ptr = global.as_obj().as_raw() as *mut c_void;
        let vm_ptr = vm.get_java_vm_pointer() as *mut c_void;
        // SAFETY: vm_ptr e ctx_ptr sono validi; chiamata esattamente una volta (Once).
        unsafe { ndk_context::initialize_android_context(vm_ptr, ctx_ptr) };
        // il context deve restare vivo per tutta la vita dell'app
        std::mem::forget(global);
    });
}
