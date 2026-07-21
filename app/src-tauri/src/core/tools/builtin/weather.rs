//! Weather tool (Open-Meteo, no API key) and current-location setter.
use super::web::{http_agent, urlencode};
use crate::core::memory::Memory;
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

fn wmo_desc(code: i64) -> &'static str {
    match code {
        0 => "sereno",
        1 => "prevalentemente sereno",
        2 => "parzialmente nuvoloso",
        3 => "nuvoloso",
        45 | 48 => "nebbia",
        51 | 53 | 55 => "pioggerella",
        56 | 57 => "pioggerella gelata",
        61 | 63 | 65 => "pioggia",
        66 | 67 => "pioggia gelata",
        71 | 73 | 75 => "neve",
        77 => "nevischio",
        80 | 81 | 82 => "rovesci",
        85 | 86 => "rovesci di neve",
        95 => "temporale",
        96 | 99 => "temporale con grandine",
        _ => "condizioni variabili",
    }
}

fn get_json(url: &str) -> Result<Value> {
    // Agent coi root CA inclusi (webpki-roots), NON ureq::get nudo: su Android lo store CA di sistema
    // è inaffidabile → l'HTTPS di meteo/geocoding falliva. Stesso agent del web tool.
    let body = http_agent()
        .get(url)
        .set("User-Agent", "Liara/1.0")
        .timeout(std::time::Duration::from_secs(8))
        .call()
        .map_err(|e| anyhow!("richiesta meteo fallita: {e}"))?
        .into_string()
        .map_err(|e| anyhow!("lettura fallita: {e}"))?;
    serde_json::from_str(&body).map_err(|e| anyhow!("risposta non valida: {e}"))
}

/// Resolve a place name to (lat, lon, label) via Open-Meteo geocoding.
pub(crate) fn geocode(city: &str) -> Result<(f64, f64, String)> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=it&format=json",
        urlencode(city)
    );
    let j = get_json(&url)?;
    let r = j.get("results").and_then(|v| v.as_array()).and_then(|a| a.first())
        .ok_or_else(|| anyhow!("località \"{city}\" non trovata"))?;
    let lat = r.get("latitude").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("geocoding incompleto"))?;
    let lon = r.get("longitude").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("geocoding incompleto"))?;
    let name = r.get("name").and_then(|v| v.as_str()).unwrap_or(city);
    let country = r.get("country").and_then(|v| v.as_str()).unwrap_or("");
    Ok((lat, lon, format!("{name}{}", if country.is_empty() { String::new() } else { format!(", {country}") })))
}

/// Reverse geocoding: coordinate → nome di città ("Modena, Italia"). Serve perché il fix GPS del
/// dispositivo dà SOLO lat/lon: senza questo, la posizione resterebbe l'etichetta muta "posizione
/// GPS" — illeggibile nelle Impostazioni e inutile al modello ("dove sono?").
/// ⚠️ Open-Meteo NON ha un endpoint reverse (verificato: 404) → usiamo BigDataCloud (HTTPS, senza
/// chiave, pensato per il client-side) con Nominatim/OSM come riserva. Entrambi in italiano.
/// PRIVACY: manda le coordinate a un terzo, come già fa il meteo. Si chiama UNA volta per fix
/// (l'esito viene memorizzato come label), non a ogni domanda.
pub(crate) fn reverse_geocode(lat: f64, lon: f64) -> Result<String> {
    let compose = |city: &str, country: &str| {
        let city = city.trim();
        if city.is_empty() {
            return None;
        }
        Some(match country.trim() {
            "" => city.to_string(),
            c => format!("{city}, {c}"),
        })
    };
    // 1) BigDataCloud: `city` a volte è vuoto sui piccoli comuni → ripiego su `locality`.
    let url = format!(
        "https://api.bigdatacloud.net/data/reverse-geocode-client?latitude={lat}&longitude={lon}&localityLanguage=it"
    );
    if let Ok(j) = get_json(&url) {
        let s = |k: &str| j.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let city = if s("city").trim().is_empty() { s("locality") } else { s("city") };
        if let Some(label) = compose(city, s("countryName")) {
            return Ok(label);
        }
    }
    // 2) Nominatim (OSM): zoom=10 = livello comune. User-Agent identificativo come da policy d'uso.
    let url = format!(
        "https://nominatim.openstreetmap.org/reverse?lat={lat}&lon={lon}&format=json&accept-language=it&zoom=10"
    );
    let j = get_json(&url)?;
    let a = j.get("address").ok_or_else(|| anyhow!("reverse geocoding senza indirizzo"))?;
    let s = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let city = ["city", "town", "village", "municipality", "county"]
        .into_iter()
        .map(s)
        .find(|v| !v.trim().is_empty())
        .unwrap_or("");
    compose(city, s("country")).ok_or_else(|| anyhow!("città non determinabile da queste coordinate"))
}

/// Approximate device location from the public IP (city-level; GPS-free fallback).
/// PRIVACY: usa ipwho.is via HTTPS. Prima era `http://ip-api.com` in CHIARO → la geolocalizzazione
/// dell'utente viaggiava in cleartext, incoerente con un'app che si presenta "privata e locale"
/// (il free tier di ip-api NON supporta HTTPS; ipwho.is sì, senza chiave).
fn ip_locate() -> Result<(f64, f64, String)> {
    let j = get_json("https://ipwho.is/?fields=success,latitude,longitude,city,country")?;
    if j.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return Err(anyhow!("posizione non determinabile"));
    }
    let lat = j.get("latitude").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("posizione incompleta"))?;
    let lon = j.get("longitude").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("posizione incompleta"))?;
    let city = j.get("city").and_then(|v| v.as_str()).unwrap_or("la tua zona");
    let country = j.get("country").and_then(|v| v.as_str()).unwrap_or("");
    Ok((lat, lon, format!("{city}{}", if country.is_empty() { String::new() } else { format!(", {country}") })))
}

pub struct Weather {
    pub mem: Arc<Memory>,
}
impl Tool for Weather {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "weather".into(),
            description: "Meteo attuale. Se non indichi una località, usa la posizione corrente del dispositivo.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": { "type": "string", "description": "Città o luogo (opzionale)" },
                    "latitude": { "type": "number", "description": "Latitudine GPS (opzionale)" },
                    "longitude": { "type": "number", "description": "Longitudine GPS (opzionale)" }
                },
                "required": []
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let (lat, lon, place) = match (
            args.get("latitude").and_then(|v| v.as_f64()),
            args.get("longitude").and_then(|v| v.as_f64()),
        ) {
            (Some(la), Some(lo)) => (la, lo, "la tua posizione".to_string()),
            _ => match args.get("location").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
                Some(loc) => geocode(loc)?,
                // no explicit place → stored location (GPS/manual) first, IP as last resort
                None => match self.mem.location() {
                    Some((la, lo, label)) => (la, lo, label),
                    None => ip_locate()?,
                },
            },
        };
        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
&current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m&timezone=auto"
        );
        let j = get_json(&url)?;
        let c = j.get("current").ok_or_else(|| anyhow!("dati meteo assenti"))?;
        let temp = c.get("temperature_2m").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let feels = c.get("apparent_temperature").and_then(|v| v.as_f64()).unwrap_or(temp);
        let hum = c.get("relative_humidity_2m").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let wind = c.get("wind_speed_10m").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let code = c.get("weather_code").and_then(|v| v.as_i64()).unwrap_or(-1);
        Ok(format!(
            "Meteo a {place}: {desc}, {temp:.0}°C (percepiti {feels:.0}°C), umidità {hum:.0}%, vento {wind:.0} km/h.",
            desc = wmo_desc(code)
        ))
    }
}

/// Etichetta segnaposto scritta da `set_gps` prima che il reverse-geocoding risolva la città.
/// Se la vediamo qui significa che la risoluzione non è ancora avvenuta (o è fallita) → la
/// rifacciamo su richiesta e la memorizziamo. Fonte unica: commands/memory.rs la usa per scriverla.
pub(crate) const GPS_PLACEHOLDER: &str = "posizione GPS";

/// "Dove sono?" — la posizione CORRENTE del dispositivo, in chiaro (città).
/// Legge quella già rilevata dall'app (GPS o impostata a mano): non attiva sensori, non chiede
/// permessi. Chiude il cerchio della posizione — prima l'app la rilevava ma il modello non aveva
/// modo di DIRLA, quindi improvvisava.
pub struct MyLocation {
    pub mem: Arc<Memory>,
}
impl Tool for MyLocation {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "my_location".into(),
            description: "Dice dove si trova l'utente adesso (città), usando la posizione già rilevata dal \
dispositivo. Usalo per \"dove sono?\", \"dove siamo?\", \"in che città siamo?\"."
                .into(),
            parameters: json!({ "type": "object", "properties": {}, "required": [] }),
        }
    }
    fn execute(&self, _args: &Value) -> Result<String> {
        let Some((lat, lon, label)) = self.mem.location() else {
            return Ok("Posizione non disponibile: chiedi all'utente di attivare il GPS (Impostazioni → \
Su di me → Sincronizza) oppure di dirti in che città si trova."
                .into());
        };
        // label già risolta (città) → risposta immediata, ZERO rete
        if label.trim() != GPS_PLACEHOLDER {
            return Ok(format!("L'utente si trova a {label}."));
        }
        // fix GPS non ancora tradotto in città (risoluzione in background non finita o fallita):
        // risolviamo ORA e memorizziamo, così la prossima volta è istantanea e le Impostazioni
        // mostrano la città invece dell'etichetta grezza.
        match reverse_geocode(lat, lon) {
            Ok(city) => {
                let _ = self.mem.set_location(lat, lon, &city, "gps");
                Ok(format!("L'utente si trova a {city}."))
            }
            Err(_) => Ok("La posizione GPS è rilevata ma non sono riuscita a risalire alla città \
(rete non disponibile). Chiedi all'utente in che città si trova."
                .into()),
        }
    }
}

/// Lets Liara set the user's current location when they state it (e.g. "sono a Bologna").
/// Persists until a fresh GPS fix overrides it. Used by the weather/location tools.
pub struct SetLocation {
    pub mem: Arc<Memory>,
}
impl Tool for SetLocation {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "set_location".into(),
            description: "Imposta la posizione corrente dell'utente quando te la comunica (es. \"sono a Bologna\", \"la mia città è Modena\"). Resta finché non arriva un GPS aggiornato.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "location": { "type": "string", "description": "Città o luogo indicato dall'utente" } },
                "required": ["location"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let loc = args.get("location").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'location'"))?;
        let (lat, lon, label) = geocode(loc)?;
        self.mem.set_location(lat, lon, &label, "manual").map_err(|e| anyhow!("{e}"))?;
        Ok(format!("Posizione impostata su {label}. La userò finché non cambia."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::crypto::Crypto;

    fn mem() -> Arc<Memory> {
        Arc::new(Memory::open(":memory:", Arc::new(Crypto::from_key(&[21u8; 32]))).unwrap())
    }

    #[test]
    fn my_location_senza_posizione_spiega_come_attivarla() {
        // niente GPS e niente città impostata → NON deve inventare un luogo, deve dire come fare
        let out = MyLocation { mem: mem() }.execute(&json!({})).unwrap();
        assert!(out.contains("non disponibile"), "{out}");
        assert!(out.to_lowercase().contains("gps") && out.contains("città"));
    }

    #[test]
    fn my_location_legge_la_citta_senza_toccare_la_rete() {
        // il caso NORMALE: posizione già risolta (da set_gps o da set_location) → lettura locale.
        // Il test gira offline in CI: se my_location facesse rete qui, fallirebbe.
        let m = mem();
        m.set_location(44.6471, 10.9252, "Modena, Italia", "gps").unwrap();
        let out = MyLocation { mem: m }.execute(&json!({})).unwrap();
        assert_eq!(out, "L'utente si trova a Modena, Italia.");
    }

    #[test]
    fn my_location_non_spaccia_il_segnaposto_per_una_citta() {
        // fix GPS non ancora tradotto: senza rete NON deve rispondere "sei a posizione GPS"
        // (era il rischio: etichetta interna presentata come luogo reale).
        let m = mem();
        m.set_location(44.6471, 10.9252, GPS_PLACEHOLDER, "gps").unwrap();
        let out = MyLocation { mem: m }.execute(&json!({})).unwrap();
        assert!(!out.contains(&format!("a {GPS_PLACEHOLDER}")), "segnaposto spacciato per città: {out}");
    }
}
