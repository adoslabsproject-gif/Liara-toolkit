<div align="center">

<img src="icon-128.png" width="112" height="112" alt="Liara" />

# Liara

### Un "second brain" AI, **100% locale**, sul tuo telefono

Assistente AI personale che gira **interamente sul dispositivo** — Android e desktop.<br>
Nessun cloud, nessun account, funziona anche **senza rete**. I tuoi dati **non escono dal telefono**.

<br>

<img src="https://img.shields.io/badge/privacy-100%25%20on--device-2ea44f" alt="on-device" />
<img src="https://img.shields.io/badge/rete-funziona%20offline-2ea44f" alt="offline" />
<img src="https://img.shields.io/badge/core-Rust-orange" alt="Rust" />
<img src="https://img.shields.io/badge/UI-React%20%2B%20Tauri-61dafb" alt="React + Tauri" />
<img src="https://img.shields.io/badge/piattaforme-Android%20%7C%20Desktop-blue" alt="platforms" />
<img src="https://img.shields.io/badge/licenza-PolyForm%20Noncommercial%201.0.0-lightgrey" alt="license" />

</div>

---

> 🔒 **Nessuna telemetria. Nessun server. Nessun crypto / token / wallet.**
> "Crittografato" qui significa che i tuoi dati sono **cifrati sul dispositivo** (AES-256), non criptovalute.
> Il codice è **pubblico e verificabile** proprio per questo: *non fidarti, leggi il codice.*

<br>

## ✨ Cosa fa

<table>
<tr>
<td width="50%" valign="top">

🧠 **LLM on-device**<br>
<sub>Qwen3 **1.5B** e **4B** (GGUF quantizzati), chat in streaming, completamente **offline**.</sub>

🛠️ **Agente con strumenti**<br>
<sub>Loop ReAct + chiamate a **grammatica vincolata (GBNF)** → JSON sempre valido, mai malformato.</sub>

🗂️ **Memoria vettoriale cifrata**<br>
<sub>Ricorda le conversazioni; profilo e fatti iniettati a ogni turno.</sub>

📝 **Appunti intelligenti**<br>
<sub>Salvi note di qualsiasi tipo, il modello le **rielabora** e le ritrova dopo.</sub>

</td>
<td width="50%" valign="top">

🗣️ **Voce offline**<br>
<sub>Whisper (STT) + Piper (TTS) via sherpa-onnx — gli parli, ti risponde.</sub>

✉️ **Email integrata**<br>
<sub>IMAP + SMTP su rustls, direttamente dall'app.</sub>

📊 **Grafici e tabelle**<br>
<sub>Genera grafici (barre, linee, aree, torta) e tabelle direttamente in chat.</sub>

🔐 **Cifratura at-rest**<br>
<sub>AES-256-GCM; chiave nel keystore del sistema operativo / sandbox privata.</sub>

</td>
</tr>
</table>

<br>

## 🧰 24 strumenti inclusi

<table>
<tr><td><b>🌐 Web</b></td><td><code>web_search</code> · <code>web_fetch</code></td></tr>
<tr><td><b>✉️ Email</b></td><td><code>email_recent</code> · <code>email_sent</code> · <code>email_search</code> · <code>email_reply</code> · <code>email_draft</code></td></tr>
<tr><td><b>📅 Calendario</b></td><td><code>calendar_add</code> · <code>calendar_list</code> · <code>calendar_search</code> · <code>calendar_delete</code></td></tr>
<tr><td><b>📝 Note</b></td><td><code>note_add</code> · <code>note_list</code> · <code>note_search</code></td></tr>
<tr><td><b>📁 File</b></td><td><code>fs_list</code> · <code>fs_read</code> · <code>fs_search</code> · <code>fs_write</code> · <code>fs_move</code> · <code>fs_delete</code></td></tr>
<tr><td><b>🧮 Utility</b></td><td><code>datetime</code> · <code>calculator</code> · <code>weather</code> · <code>set_location</code></td></tr>
</table>

Ogni strumento sensibile chiede **consenso esplicito**, revocabile da un pannello permessi.

<br>

<details>
<summary><b>📌 Stato & limiti (onestà prima di tutto)</b></summary>

<br>

- **Audio**: gestito da una pipeline **Whisper/Piper** separata — non è audio nativo nel modello.
- **Visione / input immagini**: il codice è presente ma **non ancora viabile** sull'hardware target (limiti di RAM/GPU on-device) → è **roadmap**, non promessa.
- **File (`fs_*`)**: completi su desktop, **limitati su Android** (scoped storage).
- **Non è "il migliore al mondo"**: è ingegneria seria del core, ma l'on-device AI nel 2026 è un campo affollato (Gemma + AI Edge, Apple Intelligence, PocketPal…). Liara punta su **privacy reale** e **codice verificabile**, non su record di benchmark.

</details>

<br>

## 📱 Requisiti

<table>
<thead><tr><th align="left">Modello</th><th>RAM consigliata</th><th align="left">Ideale per</th></tr></thead>
<tbody>
<tr><td>Qwen3 <b>1.5B</b></td><td align="center">~6 GB</td><td>telefoni di fascia media, risposte rapide</td></tr>
<tr><td>Qwen3 <b>4B</b></td><td align="center">8–12 GB</td><td>flagship, qualità migliore</td></tr>
</tbody>
</table>

I **modelli non sono nel repo**: si scaricano al primo avvio da `https://nothumanallowed.com/models/`,
con **resume** del download e verifica **SHA256**. Nessun token, nessun account.

<br>

## 🏗️ Architettura

Un solo **core in Rust** (`app/src-tauri/src/core/`, ~8k righe), molti frontend. UI **React/TS** sottile (Tauri).
Inferenza **llama.cpp in-process** (compila su ogni piattaforma, Android incluso), con KV-cache persistente e
prefix-caching. `trait` per engine / memory / tool → sottosistemi **swappabili**.

```
app/
├── src/                 # frontend React/TS (chat, drawer, grafici, voce)
└── src-tauri/
    ├── src/core/        # engine · agent · tools · memory · crypto · email · audio
    └── vendor/          # llama.cpp (build patchato per Android)
```

Dettaglio completo in **[`ARCHITECTURE.md`](./ARCHITECTURE.md)** · elenco strumenti in **[`TOOLS.md`](./TOOLS.md)**.

<br>

## 🚀 Build (sviluppo)

```bash
cd app
npm install            # o pnpm install
npm run tauri dev      # desktop (dev)
npm run tauri build    # release desktop
# Android:
npm run tauri android init
npm run tauri android build
```

<br>

## 📜 Licenza

Il **codice** è **source-available** sotto **[PolyForm Noncommercial License 1.0.0](./LICENSE.md)** —
uso, studio, modifica e fork liberi **solo per scopi non commerciali**. L'uso commerciale richiede una
licenza separata dal titolare. *(Non è "open source" in senso OSI: l'open source deve permettere anche il
commerciale.)*

I **modelli e i LoRA sono proprietari**: **non** inclusi nel repo e **non** concessi in licenza per
redistribuzione, riaddestramento o uso commerciale. Sono il "cervello" del progetto e restano privati.
Vedi **[`LICENSE.md`](./LICENSE.md)**.

<br>

<div align="center">
<sub>Costruito con Rust 🦀 + Tauri + llama.cpp · I tuoi dati restano tuoi.</sub>
</div>
