# Liara — Tool Catalog

> ⚠️ **Status:** this is the TARGET catalog (the plan), **not** what's built. **Implemented today (6):** `datetime`, `calculator`, `email_recent`, `email_sent`, `email_search`, `email_reply`, `email_draft`. Everything else below is **planned**. The tools are a local Rust `Tool` trait — **MCP, GBNF, consent-gates and the ~74 other tools are not yet implemented.** Next batch to prioritize: `web_fetch`, `web_search`, `fs_list/read/search`, `memory_search`.

Legenda: 🟢 locale/offline · 🌐 richiede rete · 🔑 richiede API key o backend · 🖥️ desktop · 📱 telefono (Android) · ⚠️ richiede permesso utente

Tutti i tool passano dal **tool host MCP** con **chiamate a grammatica vincolata (GBNF)** → JSON sempre valido. Ogni tool sensibile chiede **consenso** la prima volta (come i permessi del telefono).

## 🌐 Web & conoscenza
- **web_fetch** 🌐 — URL → testo pulito/markdown (estrazione readability), via anche per leggere articoli/pagine
- **web_search** 🌐🔑 — ricerca avanzata (backend SearXNG self-hosted = gratis+privato, o API Brave/Tavily); risultati ordinati
- **web_screenshot** 🌐 — render pagina → immagine
- **wikipedia / dizionario / traduci** 🌐
- **maps_places** 🌐🔑 — geocoding, vicino a me, percorsi
- **news / quotazioni / meteo** 🌐
- **youtube_transcript** 🌐 — trascrizione video

## 📁 File & gestione dispositivo (il "file manager")
- **fs_list** 🟢 — contenuto di una cartella ("apri Download")
- **fs_read** 🟢 — leggi file (txt, pdf, docx, csv → testo)
- **fs_search** 🟢 — trova file per nome/contenuto
- **fs_write / fs_create** 🟢⚠️
- **fs_move / fs_rename / fs_copy / fs_delete** 🟢⚠️ (con conferma)
- **fs_open** 🟢 — apri con app predefinita
- **fs_organize** 🟢 — riordino intelligente ("ordina la cartella Download per tipo/data")
- **disk_usage / find_large / find_duplicates** 🟢
- **zip / unzip** 🟢

## ✉️ Email & comunicazione
- **email_read / email_search** 🟢 (già salviamo nel DB)
- **email_send** 🌐 — invio SMTP (prossimo step)
- **calendar** 🟢/🌐 — leggi/crea eventi (locale o CalDAV)
- **contacts** 🟢📱⚠️ — rubrica
- **notifications_read** 📱⚠️ — legge notifiche in arrivo (incl. WhatsApp/Telegram, via NotificationListener)
- **sms_read / sms_send** 📱⚠️
- **call** 📱⚠️ — avvia chiamata

## 📱 Dispositivo & sensori (telefono)
- **location** 📱⚠️🌐 — GPS / posizione
- **weather** 🌐🔑 — meteo dalla posizione
- **battery** 🟢📱 — livello, in carica
- **connectivity** 🟢📱 — wifi/cellulare/aereo
- **device_info** 🟢 — modello, OS, RAM, storage libero
- **accelerometro / giroscopio / orientamento** 📱
- **sensore_luce / prossimità** 📱
- **contapassi / attività** 📱⚠️
- **bussola / magnetometro** 📱
- **barometro** 📱 — pressione/altitudine
- **camera** 📱⚠️ — foto, scansione QR/documenti
- **microfono** 📱⚠️ — registra / trascrivi
- **clipboard** 🟢 — leggi/scrivi appunti
- **torcia / vibrazione** 📱
- **luminosità / volume** 📱⚠️
- **screenshot / schermo** 🖥️📱
- **bluetooth / NFC scan** 📱⚠️

## 🧰 Produttività & utilità
- **calculator** 🟢 — matematica
- **convert** 🟢 — unità / valuta (valuta 🌐)
- **datetime** 🟢 — ora, fusi, "tra quanto…"
- **reminders / alarms / timers** 🟢📱⚠️
- **notes / todo** 🟢
- **code_exec** 🟢⚠️ — Python/JS in sandbox
- **ocr** 🟢 — immagine → testo
- **qr_generate / qr_read** 🟢
- **password_generate** 🟢

## ⚙️ Sistema & automazione
- **open_app** 🖥️📱⚠️
- **settings_toggle** 📱⚠️ — wifi, bluetooth, DND, luminosità
- **run_shortcut** 📱 — automazioni / scorciatoie
- **screenshot** 🖥️📱

## 🧠 Memoria & RAG (interni)
- **memory_search** 🟢 — ricerca semantica (sqlite-vec)
- **memory_add / profile_update** 🟢
- **rag_query** 🟢 — pacchetti di dominio

---

## Architettura
- Tutto via **MCP** → si aggiungono anche tool esterni dell'ecosistema senza riscriverli.
- **Permessi granulari**: ogni tool sensibile chiede consenso (revocabile in un pannello "Permessi").
- **Locale prima**: i tool 🟢 funzionano offline; i 🌐 usano la rete solo quando invocati.

## Ordine di costruzione consigliato
1. **MCP tool host + loop agente + chiamate GBNF** (fondazione per TUTTI i tool)
2. **Primo lotto cross-platform**: web_fetch, web_search, file manager (list/read/search/manage), email_send, datetime/calc, memory_search
3. **Tool dispositivo/sensori** quando siamo su **Android** (location, weather, battery, sensori, notifiche)
