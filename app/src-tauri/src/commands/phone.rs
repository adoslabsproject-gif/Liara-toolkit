//! Selettore contatti PERMISSION-FREE (Android). Il frontend chiama `pick_contact`; il Rust apre il
//! picker di sistema via JNI (MainActivity.launchContactPick), l'utente sceglie un contatto, e il
//! risultato torna qui dalla callback nativa `nativeContactPicked` che sblocca l'attesa. Nessun
//! permesso READ_CONTACTS: il sistema concede l'accesso al solo contatto scelto.
#[cfg(target_os = "android")]
use std::sync::{mpsc, Mutex, OnceLock};

/// Slot per consegnare l'esito dal callback nativo (thread UI) al comando in attesa.
#[cfg(target_os = "android")]
fn pick_slot() -> &'static Mutex<Option<mpsc::Sender<(String, String)>>> {
    static SLOT: OnceLock<Mutex<Option<mpsc::Sender<(String, String)>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Slot per l'esito del lettore QR nativo.
#[cfg(target_os = "android")]
fn scan_slot() -> &'static Mutex<Option<mpsc::Sender<String>>> {
    static SLOT: OnceLock<Mutex<Option<mpsc::Sender<String>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Apre il lettore QR NATIVO (ZXing, fotocamera live a schermo intero) e ritorna il testo del QR letto.
/// Su Android il video getUserMedia nella WebView è nero (hardwareAccelerated=false) → serve il nativo.
#[tauri::command]
pub async fn scan_qr() -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        let (tx, rx) = mpsc::channel();
        *scan_slot().lock().unwrap() = Some(tx);
        android::launch_scan().map_err(|e| e.to_string())?;
        let res = tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(std::time::Duration::from_secs(180))
        })
        .await
        .map_err(|e| e.to_string())?;
        match res {
            Ok(text) if !text.is_empty() => Ok(text),
            Ok(_) => Err("Scansione annullata".into()),
            Err(_) => Err("Scansione annullata o scaduta".into()),
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        Err("Il lettore QR nativo è disponibile solo sull'app Android.".into())
    }
}

/// Callback dal Kotlin (MainActivity) col testo del QR letto → sblocca `scan_qr`.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_liara_app_MainActivity_nativeQrScanned<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    text: jni::objects::JString<'local>,
) {
    let text = env
        .get_string(&text)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(tx) = scan_slot().lock().unwrap().take() {
        let _ = tx.send(text);
    }
}

/// Apre il selettore contatti di sistema e ritorna (nome, numero) del contatto scelto.
#[tauri::command]
pub async fn pick_contact() -> Result<(String, String), String> {
    #[cfg(target_os = "android")]
    {
        let (tx, rx) = mpsc::channel();
        *pick_slot().lock().unwrap() = Some(tx);
        android::launch().map_err(|e| e.to_string())?;
        // il picker è UI-bloccante per l'utente ma async per noi: aspettiamo l'esito fuori dal thread async
        let res = tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(std::time::Duration::from_secs(180))
        })
        .await
        .map_err(|e| e.to_string())?;
        match res {
            Ok((name, number)) if !name.is_empty() || !number.is_empty() => Ok((name, number)),
            Ok(_) => Err("Nessun contatto selezionato".into()),
            Err(_) => Err("Selezione annullata o scaduta".into()),
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        Err("Il selettore contatti è disponibile solo sull'app Android.".into())
    }
}

/// Callback dal Kotlin (MainActivity) col contatto scelto → sblocca `pick_contact`.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_liara_app_MainActivity_nativeContactPicked<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    name: jni::objects::JString<'local>,
    number: jni::objects::JString<'local>,
) {
    let name = env
        .get_string(&name)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let number = env
        .get_string(&number)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(tx) = pick_slot().lock().unwrap().take() {
        let _ = tx.send((name, number));
    }
}

/// JNI verso `MainActivity.launchContactPick()` (statico companion), stesso pattern di android_keystore.
#[cfg(target_os = "android")]
mod android {
    use anyhow::{anyhow, Context, Result};
    use jni::objects::{JObject, JString, JValue};

    const ACTIVITY_CLASS: &str = "com.liara.app.MainActivity";

    pub(super) fn launch() -> Result<()> {
        call_static_void("launchContactPick")
    }
    pub(super) fn launch_scan() -> Result<()> {
        call_static_void("launchQrScan")
    }

    /// Chiama un metodo statico `()V` del companion di MainActivity via JNI (class loader dell'app).
    fn call_static_void(method: &str) -> Result<()> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.context("JavaVM")?;
        let mut env = vm.attach_current_thread().context("attach thread JNI")?;

        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        let loader = env
            .call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
            .and_then(|v| v.l())
            .context("getClassLoader")?;
        let class_name: JString = env.new_string(ACTIVITY_CLASS).context("class name")?;
        let activity = env
            .call_method(
                &loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&class_name)],
            )
            .and_then(|v| v.l())
            .map_err(|e| anyhow!("loadClass {ACTIVITY_CLASS}: {e}"))?;

        let res = env.call_static_method(jni::objects::JClass::from(activity), method, "()V", &[]);
        // cattura il MESSAGGIO dell'eccezione Java (non solo "eccezione generica") per poterla debuggare
        if env.exception_check().unwrap_or(false) {
            let exc = env.exception_occurred().ok();
            let _ = env.exception_clear();
            let desc = exc
                .and_then(|e| env.call_method(&e, "toString", "()Ljava/lang/String;", &[]).ok())
                .and_then(|v| v.l().ok())
                .and_then(|o| {
                    let js = JString::from(o);
                    env.get_string(&js).ok().map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "eccezione Java non descrivibile".into());
            return Err(anyhow!("MainActivity.{method}: {desc}"));
        }
        res.map(|_| ()).map_err(|e| anyhow!("MainActivity.{method}: {e}"))
    }
}
