// Dati e tipi puri estratti da App.tsx (nessuna logica, nessun React) — così il componente resta
// focalizzato sul comportamento e questi cataloghi vivono in un posto solo, facili da estendere.

export type Role = "user" | "assistant";
export type Node = { id: string; parentId: string; role: Role; content: string; image?: string };
export type Msg = { role: Role; content: string };
export const ROOT = "";

// Le chiavi (primo elemento) restano in italiano: sono usate come chiave di storage (set_profile)
// e NON vanno tradotte. Il secondo elemento è l'etichetta inglese mostrata a schermo.
export const PROFILE_GROUPS: { title: string; titleEn: string; fields: [string, string][] }[] = [
  { title: "Identità", titleEn: "About you", fields: [["Nome", "First name"], ["Cognome", "Last name"], ["Soprannome", "Nickname"], ["Data di nascita", "Date of birth"], ["Genere", "Gender"], ["Città", "City"], ["Nazionalità", "Nationality"], ["Lingue parlate", "Languages spoken"]] },
  { title: "Contatti", titleEn: "Contacts", fields: [["Email", "Email"], ["Telefono", "Phone"], ["Indirizzo", "Address"]] },
  { title: "Lavoro & studi", titleEn: "Work & studies", fields: [["Professione", "Profession"], ["Azienda", "Company"], ["Titolo di studio", "Education"], ["Settore", "Industry"]] },
  { title: "Famiglia & relazioni", titleEn: "Family & relationships", fields: [["Stato sentimentale", "Relationship status"], ["Partner", "Partner"], ["Figli", "Children"], ["Genitori", "Parents"], ["Fratelli/sorelle", "Siblings"], ["Animali domestici", "Pets"], ["Amici importanti", "Close friends"]] },
  { title: "Vita & preferenze", titleEn: "Life & preferences", fields: [["Hobby e passioni", "Hobbies & passions"], ["Cibo preferito", "Favourite food"], ["Musica", "Music"], ["Film e serie", "Films & series"], ["Sport", "Sports"], ["Cosa non ama", "Dislikes"]] },
  { title: "Salute", titleEn: "Health", fields: [["Allergie", "Allergies"], ["Note di salute", "Health notes"], ["Dieta", "Diet"]] },
  { title: "Obiettivi & valori", titleEn: "Goals & values", fields: [["Obiettivi", "Goals"], ["Valori", "Values"], ["Progetti in corso", "Current projects"]] },
  { title: "Note libere", titleEn: "Free notes", fields: [["Note", "Notes"]] },
];

// IMAP host/port per i provider più comuni (auto-compilazione)
export const EMAIL_PROVIDERS: Record<string, { imap_host: string; imap_port: string; smtp_host: string; smtp_port: string; app_pw?: boolean }> = {
  "Gmail": { imap_host: "imap.gmail.com", imap_port: "993", smtp_host: "smtp.gmail.com", smtp_port: "465", app_pw: true },
  "Outlook / Hotmail": { imap_host: "outlook.office365.com", imap_port: "993", smtp_host: "smtp.office365.com", smtp_port: "587", app_pw: true },
  "Yahoo": { imap_host: "imap.mail.yahoo.com", imap_port: "993", smtp_host: "smtp.mail.yahoo.com", smtp_port: "465", app_pw: true },
  "iCloud": { imap_host: "imap.mail.me.com", imap_port: "993", smtp_host: "smtp.mail.me.com", smtp_port: "587", app_pw: true },
  "Libero": { imap_host: "imapmail.libero.it", imap_port: "993", smtp_host: "smtp.libero.it", smtp_port: "465" },
  "Virgilio": { imap_host: "in.virgilio.it", imap_port: "993", smtp_host: "out.virgilio.it", smtp_port: "465" },
  "Alice / TIM": { imap_host: "in.alice.it", imap_port: "993", smtp_host: "out.alice.it", smtp_port: "465" },
  "Tiscali": { imap_host: "imap.tiscali.it", imap_port: "993", smtp_host: "smtp.tiscali.it", smtp_port: "587" },
  "Aruba": { imap_host: "imaps.aruba.it", imap_port: "993", smtp_host: "smtps.aruba.it", smtp_port: "465" },
  "mail.ru": { imap_host: "imap.mail.ru", imap_port: "993", smtp_host: "smtp.mail.ru", smtp_port: "465" },
  "GMX": { imap_host: "imap.gmx.com", imap_port: "993", smtp_host: "mail.gmx.com", smtp_port: "587" },
  "Zoho": { imap_host: "imap.zoho.eu", imap_port: "993", smtp_host: "smtp.zoho.eu", smtp_port: "465" },
};

// per-provider sign-in help: app-password where required, "enable IMAP" otherwise.
// Testo bilingue: tradotto a schermo con t(title/titleEn), t(note/noteEn), t(linkText/linkTextEn).
export const PROVIDER_HELP: Record<string, { title: string; titleEn: string; note: string; noteEn: string; link?: string; linkText?: string; linkTextEn?: string }> = {
  "Gmail": { title: "Gmail: serve la PASSWORD PER LE APP", titleEn: "Gmail: you need an APP PASSWORD", note: "Attiva la verifica in due passaggi, poi genera la password (16 lettere) e incollala qui — NON la password dell'account.", noteEn: "Turn on two-step verification, then generate the password (16 letters) and paste it here — NOT your account password.", link: "https://myaccount.google.com/apppasswords", linkText: "Crea password per le app ↗", linkTextEn: "Create an app password ↗" },
  "Outlook / Hotmail": { title: "Outlook: password per le app", titleEn: "Outlook: app password", note: "Con la verifica in due passaggi attiva, crea una password per le app e usala qui.", noteEn: "With two-step verification on, create an app password and use it here.", link: "https://account.live.com/proofs/AppPassword", linkText: "Crea password per le app ↗", linkTextEn: "Create an app password ↗" },
  "Yahoo": { title: "Yahoo: password per le app", titleEn: "Yahoo: app password", note: "Genera una password per le app dalle impostazioni di sicurezza Yahoo e usala qui.", noteEn: "Generate an app password from your Yahoo security settings and use it here.", link: "https://login.yahoo.com/account/security/app-passwords", linkText: "Genera password per le app ↗", linkTextEn: "Generate an app password ↗" },
  "iCloud": { title: "iCloud: password specifica per app", titleEn: "iCloud: app-specific password", note: "Da Apple ID → Sicurezza, genera una 'password per le app' e usala qui (non la password Apple ID).", noteEn: "From Apple ID → Security, generate an 'app-specific password' and use it here (not your Apple ID password).", link: "https://account.apple.com/account/manage", linkText: "Gestisci Apple ID ↗", linkTextEn: "Manage Apple ID ↗" },
  "mail.ru": { title: "mail.ru: password per app esterne", titleEn: "mail.ru: password for external apps", note: "Crea una password per le applicazioni esterne e usala al posto di quella normale.", noteEn: "Create a password for external apps and use it instead of your normal one.", link: "https://account.mail.ru/user/2-step-auth/passwords/", linkText: "Password per app ↗", linkTextEn: "App passwords ↗" },
  "Zoho": { title: "Zoho: password specifica", titleEn: "Zoho: app-specific password", note: "Con 2FA attiva, genera una password specifica per l'app e usala qui.", noteEn: "With 2FA on, generate an app-specific password and use it here.", link: "https://accounts.zoho.eu/home#security/device", linkText: "Password specifiche ↗", linkTextEn: "App-specific passwords ↗" },
  "Libero": { title: "Libero: abilita IMAP", titleEn: "Libero: enable IMAP", note: "Usa la password normale della casella; assicurati che l'accesso IMAP sia abilitato dalle impostazioni del webmail.", noteEn: "Use your normal mailbox password; make sure IMAP access is enabled in the webmail settings." },
  "Virgilio": { title: "Virgilio: abilita IMAP", titleEn: "Virgilio: enable IMAP", note: "Usa la password normale; abilita l'accesso IMAP/POP dalle impostazioni del webmail.", noteEn: "Use your normal password; enable IMAP/POP access in the webmail settings." },
  "Alice / TIM": { title: "Alice/TIM: abilita IMAP", titleEn: "Alice/TIM: enable IMAP", note: "Usa la password normale; verifica che IMAP sia abilitato nelle impostazioni TIM Mail.", noteEn: "Use your normal password; check that IMAP is enabled in your TIM Mail settings." },
  "Tiscali": { title: "Tiscali: abilita IMAP", titleEn: "Tiscali: enable IMAP", note: "Usa la password normale dell'account, con l'accesso IMAP abilitato.", noteEn: "Use your normal account password, with IMAP access enabled." },
  "Aruba": { title: "Aruba", titleEn: "Aruba", note: "Usa la password della casella; l'accesso IMAP è in genere già attivo.", noteEn: "Use your mailbox password; IMAP access is usually already on." },
  "GMX": { title: "GMX: abilita IMAP", titleEn: "GMX: enable IMAP", note: "Abilita l'accesso POP3/IMAP nelle impostazioni GMX, poi usa la password normale.", noteEn: "Enable POP3/IMAP access in your GMX settings, then use your normal password." },
};

export const TOOL_LABELS: Record<string, [string, string]> = {
  datetime: ["Controllo data e ora", "Checking date & time"],
  calculator: ["Calcolo", "Calculating"],
  email_recent: ["Leggo le email ricevute", "Reading received emails"],
  email_sent: ["Leggo le email inviate", "Reading sent emails"],
  email_search: ["Cerco nelle email", "Searching emails"],
  email_reply: ["Preparo la risposta", "Drafting the reply"],
  email_draft: ["Preparo l'email", "Drafting the email"],
};

// Per-theme menu icons: each palette gets its own set so the menu feels native to the theme.
export type MenuIcons = { profile: string; email: string; agenda: string; perms: string; theme: string; voiceOn: string; voiceOff: string };
export const MENU_ICONS: Record<string, MenuIcons> = {
  "": { profile: "👤", email: "✉️", agenda: "📅", perms: "🔐", theme: "🎨", voiceOn: "🔊", voiceOff: "🔇" },
  dusk: { profile: "🧑", email: "📨", agenda: "🗓️", perms: "🔒", theme: "🖌️", voiceOn: "🔊", voiceOff: "🔇" },
  sage: { profile: "🙂", email: "💌", agenda: "📆", perms: "🛡️", theme: "🎭", voiceOn: "📣", voiceOff: "🤫" },
  copper: { profile: "🧑‍💼", email: "✉", agenda: "🗓", perms: "🗝️", theme: "🖼️", voiceOn: "🎙️", voiceOff: "🔇" },
  cream: { profile: "😊", email: "📧", agenda: "📔", perms: "🔏", theme: "🎨", voiceOn: "🗣️", voiceOff: "🤐" },
};

export const THEMES: { id: string; name: string; en: string; c: string }[] = [
  { id: "", name: "Notte blu", en: "Blue night", c: "#7c5cff" },
  { id: "dusk", name: "Crepuscolo caldo", en: "Warm dusk", c: "#e89b73" },
  { id: "sage", name: "Salvia notturna", en: "Night sage", c: "#8fc9a6" },
  { id: "copper", name: "Inchiostro e rame", en: "Ink & copper", c: "#d99a5b" },
  { id: "cream", name: "Crema diurna", en: "Daytime cream", c: "#4a9e92" },
];
