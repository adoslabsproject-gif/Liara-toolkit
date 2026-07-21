//! Comandi SMS: sincronizzazione della copia LOCALE cifrata dei messaggi (core/sms).
//!
//! Flusso "Sincronizza SMS" (bottone nel drawer Rubrica, consenso informato): `sms_sync` assicura
//! il permesso READ_SMS via MainActivity (dialog al primo uso → callback `nativeSmsPermission`),
//! poi legge i messaggi recenti in modo SINCRONO via SmsBridge.read (JNI con ritorno: qui basta il
//! Context) e li importa nello store cifrato. I tool `sms_recent`/`sms_search` leggono dallo store.
use crate::AppState;
use tauri::State;

#[cfg(target_os = "android")]
use std::sync::{mpsc, Mutex, OnceLock};

/// Quanti SMS al massimo si sincronizzano per giro (i più recenti): abbastanza storia per la
/// ricerca, senza trascinarsi dietro anni di messaggi al primo import.
#[cfg(target_os = "android")]
const SYNC_LIMIT: i32 = 500;

/// Slot per l'esito del dialog di permesso (thread UI Kotlin → comando in attesa).
#[cfg(target_os = "android")]
fn perm_slot() -> &'static Mutex<Option<mpsc::Sender<bool>>> {
    static SLOT: OnceLock<Mutex<Option<mpsc::Sender<bool>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Callback dal Kotlin (MainActivity) con l'esito del permesso READ_SMS → sblocca `sms_sync`.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_liara_app_MainActivity_nativeSmsPermission<'local>(
    _env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    granted: jni::sys::jboolean,
) {
    if let Some(tx) = perm_slot().lock().unwrap().take() {
        let _ = tx.send(granted != 0);
    }
}

/// Sincronizza gli SMS di sistema nello store cifrato. Ritorna (nuovi, totale nello store).
#[tauri::command]
pub async fn sms_sync(state: State<'_, AppState>) -> Result<(usize, usize), String> {
    #[cfg(target_os = "android")]
    {
        let (tx, rx) = mpsc::channel();
        *perm_slot().lock().unwrap() = Some(tx);
        super::phone::android::call_static_void("launchSmsPermission").map_err(|e| e.to_string())?;
        let granted = tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(std::time::Duration::from_secs(180))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "Richiesta permesso SMS annullata o scaduta".to_string())?;
        if !granted {
            return Err("Permesso SMS negato: puoi concederlo dalle impostazioni Android.".into());
        }
        // lettura fuori dal thread async (query al provider SMS, può essere lenta su storici grandi)
        let json = tauri::async_runtime::spawn_blocking(move || android::read_sms(SYNC_LIMIT))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        if json == "DENIED" {
            return Err("Permesso SMS negato: puoi concederlo dalle impostazioni Android.".into());
        }
        let items: Vec<(String, String, i64, String)> =
            serde_json::from_str(&json).map_err(|e| format!("SMS di sistema illeggibili: {e}"))?;
        let new = state.sms.import(&items).map_err(|e| e.to_string())?;
        let total = state.sms.count().map_err(|e| e.to_string())?;
        Ok((new, total))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = state;
        Err("La sincronizzazione SMS è disponibile solo sull'app Android.".into())
    }
}

/// Quanti SMS ci sono nello store cifrato (per la UI del drawer Rubrica).
#[tauri::command]
pub fn sms_count(state: State<AppState>) -> Result<usize, String> {
    state.sms.count().map_err(|e| e.to_string())
}

/// Un SMS per la LISTA nella UI: mittente/destinatario già risolto col nome di rubrica (o numero),
/// testo, timestamp (secondi) e verso ("in"/"out"). Il frontend formatta la data.
#[derive(serde::Serialize)]
pub struct SmsView {
    pub who: String,
    pub body: String,
    pub ts: i64,
    pub kind: String,
}

/// Gli ultimi SMS (decifrati) per il drawer: dal più recente, col nome di rubrica quando c'è.
#[tauri::command]
pub fn sms_list(limit: Option<usize>, state: State<AppState>) -> Result<Vec<SmsView>, String> {
    let limit = limit.unwrap_or(200).clamp(1, 500);
    let msgs = state.sms.recent(limit).map_err(|e| e.to_string())?;
    let names = state.contacts.names_by_key().unwrap_or_default();
    Ok(msgs
        .into_iter()
        .map(|m| {
            let key = crate::core::contacts::number_key(&m.number);
            let who = names.get(&key).cloned().unwrap_or_else(|| m.number.clone());
            SmsView { who, body: m.body, ts: m.ts, kind: m.kind }
        })
        .collect())
}

/// JNI verso `SmsBridge.read(Context, int): String` — come builtin/phone.rs ma CON valore di ritorno.
#[cfg(target_os = "android")]
mod android {
    use anyhow::{anyhow, Context, Result};
    use jni::objects::{JObject, JString, JValue};

    const BRIDGE_CLASS: &str = "com.liara.app.SmsBridge";

    pub(super) fn read_sms(limit: i32) -> Result<String> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.context("JavaVM")?;
        let mut env = vm.attach_current_thread().context("attach thread JNI")?;

        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        let loader = env
            .call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
            .and_then(|v| v.l())
            .context("getClassLoader")?;
        let class_name: JString = env.new_string(BRIDGE_CLASS).context("class name")?;
        let bridge = env
            .call_method(
                &loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&class_name)],
            )
            .and_then(|v| v.l())
            .map_err(|e| anyhow!("loadClass {BRIDGE_CLASS}: {e}"))?;

        let res = env.call_static_method(
            jni::objects::JClass::from(bridge),
            "read",
            "(Landroid/content/Context;I)Ljava/lang/String;",
            &[JValue::Object(&context), JValue::Int(limit)],
        );
        // eccezione Java pendente: pulita e riportata col SUO messaggio (debuggabile)
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
            return Err(anyhow!("SmsBridge.read: {desc}"));
        }
        let obj = res
            .and_then(|v| v.l())
            .map_err(|e| anyhow!("SmsBridge.read: {e}"))?;
        let js = JString::from(obj);
        // bind locale PRIMA del return: la tail-expression terrebbe vivo il JavaStr temporaneo
        // oltre il drop di `vm`/`js` (E0597 sul target Android)
        let out = env
            .get_string(&js)
            .context("stringa di ritorno")?
            .to_string_lossy()
            .into_owned();
        Ok(out)
    }
}
