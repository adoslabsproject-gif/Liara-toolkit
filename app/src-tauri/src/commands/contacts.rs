//! Comandi RUBRICA: sincronizzazione con la rubrica di sistema (Android, permesso READ_CONTACTS su
//! consenso informato) + import GRANULARE nello store cifrato di Liara (core/contacts).
//!
//! Flusso "Sincronizza rubrica": il frontend chiama `contacts_sync` → il Rust apre il flusso nativo
//! (MainActivity.launchContactsRead: dialog di permesso se serve, poi lettura completa) → il JSON
//! torna dalla callback `nativeContactsRead` → qui lo annotiamo con `imported` (già nello store?)
//! così la UI evidenzia i NUOVI. L'utente sceglie quali importare → `contacts_import`. I numeri
//! restano nel liara.db cifrati (AES-256-GCM), MAI sul server.
use crate::core::contacts::Contact;
use crate::AppState;
use tauri::State;

#[cfg(target_os = "android")]
use std::sync::{mpsc, Mutex, OnceLock};

/// Slot per consegnare il JSON della rubrica dal callback nativo (thread Kotlin) al comando in attesa.
#[cfg(target_os = "android")]
fn read_slot() -> &'static Mutex<Option<mpsc::Sender<String>>> {
    static SLOT: OnceLock<Mutex<Option<mpsc::Sender<String>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Callback dal Kotlin (MainActivity) col JSON `[["nome","numero"],…]` (o "DENIED") → sblocca `contacts_sync`.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_liara_app_MainActivity_nativeContactsRead<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    json: jni::objects::JString<'local>,
) {
    let json = env
        .get_string(&json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(tx) = read_slot().lock().unwrap().take() {
        let _ = tx.send(json);
    }
}

/// Un contatto della rubrica DI SISTEMA, annotato per la UI di selezione: `imported` = già nello
/// store di Liara → la UI lo mostra come acquisito e preseleziona solo i NUOVI. `customized` = quel
/// contatto è stato modificato nell'app: la UI lo marca "personalizzato" e NON lo re-importa (il
/// re-import ripristinerebbe il numero di sistema, cancellando la modifica dell'utente).
#[derive(serde::Serialize)]
pub struct SysContact {
    pub name: String,
    pub number: String,
    pub imported: bool,
    pub customized: bool,
}

/// Legge la rubrica di sistema (Android: dialog permesso READ_CONTACTS se serve) e la ritorna
/// annotata con `imported`. NON scrive nulla: l'import è un passo separato e granulare.
#[tauri::command]
pub async fn contacts_sync(state: State<'_, AppState>) -> Result<Vec<SysContact>, String> {
    #[cfg(target_os = "android")]
    {
        let (tx, rx) = mpsc::channel();
        *read_slot().lock().unwrap() = Some(tx);
        super::phone::android::call_static_void("launchContactsRead").map_err(|e| e.to_string())?;
        // attesa fuori dal thread async: il dialog di permesso + una rubrica grande possono richiedere tempo
        let json = tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(std::time::Duration::from_secs(180))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "Lettura rubrica annullata o scaduta".to_string())?;
        if json == "DENIED" {
            return Err("Permesso rubrica negato: puoi concederlo dalle impostazioni Android.".into());
        }
        let items: Vec<(String, String)> = serde_json::from_str(&json)
            .map_err(|e| format!("rubrica di sistema illeggibile: {e}"))?;
        // stato per ORIGINE: assente = nuovo, false = già importato, true = personalizzato
        let state_map = state.contacts.sync_state().map_err(|e| e.to_string())?;
        Ok(items
            .into_iter()
            .filter(|(_, number)| !crate::core::contacts::number_key(number).is_empty())
            .map(|(name, number)| {
                let key = crate::core::contacts::number_key(&number);
                let (imported, customized) = match state_map.get(&key) {
                    Some(&cust) => (true, cust),
                    None => (false, false),
                };
                SysContact { name, number, imported, customized }
            })
            .collect())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = state;
        Err("La sincronizzazione rubrica è disponibile solo sull'app Android.".into())
    }
}

/// Importa i contatti SELEZIONATI dall'utente nello store cifrato. Ritorna quanti NUOVI.
#[tauri::command]
pub fn contacts_import(
    items: Vec<(String, String)>,
    state: State<AppState>,
) -> Result<usize, String> {
    state.contacts.import(&items).map_err(|e| e.to_string())
}

/// Contatti già importati (decifrati), per la lista "Rubrica di Liara" nella UI.
#[tauri::command]
pub fn contacts_list(state: State<AppState>) -> Result<Vec<Contact>, String> {
    state.contacts.all(1000).map_err(|e| e.to_string())
}

/// Modifica un contatto NELLA rubrica dell'app (nome e/o numero). Lo marca personalizzato → il sync
/// non lo sovrascrive più. Errore se il numero è già di un altro contatto.
#[tauri::command]
pub fn contacts_update(
    id: i64,
    name: String,
    number: String,
    state: State<AppState>,
) -> Result<(), String> {
    state.contacts.update(id, &name, &number).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn contacts_delete(id: i64, state: State<AppState>) -> Result<(), String> {
    state.contacts.delete(id).map_err(|e| e.to_string())
}
