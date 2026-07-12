#!/usr/bin/env bash
# =============================================================================
# smoke-android.sh — "coverage" di avvio per l'APK su device reale.
#
# È il test che cattura la classe di bug che i test Rust NON vedono: l'app che
# si builda e installa ma poi CRASHA all'avvio sul telefono (panic Rust nel
# motore, OOM/lowmemorykiller, SIGSEGV nativo OpenCL/ggml, DB non decifrabile).
#
# Cosa fa, in ordine:
#   1. trova adb, verifica che UN device sia collegato
#   2. (opzionale) reinstalla l'APK passato come argomento
#   3. avvia l'app da stato pulito e azzera logcat
#   4. tiene d'occhio il PID per N secondi: se muore → FALLITO
#   5. scansiona logcat per panic/SIGSEGV/SIGABRT/OOM/kill → se trovati → FALLITO
#   6. exit 0 = avvio sano, exit !=0 = regressione (usabile in CI / pre-deploy)
#
# Uso:
#   scripts/smoke-android.sh                  # testa l'app già installata
#   scripts/smoke-android.sh path/al.apk      # reinstalla poi testa
#   WATCH_SECONDS=90 scripts/smoke-android.sh  # avvio + 90s sotto osservazione
#   WIPE=1 scripts/smoke-android.sh           # cancella i dati app prima (DB vecchio)
# =============================================================================
set -uo pipefail

PKG="com.liara.app"
ACT="$PKG/.MainActivity"
WATCH_SECONDS="${WATCH_SECONDS:-45}"
APK="${1:-}"

# --- trova adb (non sempre nel PATH su macOS) -------------------------------
ADB="$(command -v adb 2>/dev/null || true)"
for cand in "$HOME/Library/Android/sdk/platform-tools/adb" \
            "$HOME/Android/Sdk/platform-tools/adb" \
            "/opt/homebrew/bin/adb" "/usr/local/bin/adb"; do
  [ -z "$ADB" ] && [ -x "$cand" ] && ADB="$cand"
done
[ -z "$ADB" ] && { echo "❌ adb non trovato. Installa platform-tools."; exit 2; }

# --- esattamente un device ---------------------------------------------------
N_DEV="$("$ADB" devices | grep -cw "device")"
[ "$N_DEV" -eq 0 ] && { echo "❌ Nessun device collegato (controlla cavo + debug USB)."; exit 2; }
[ "$N_DEV" -gt 1 ] && { echo "❌ Più device collegati: scollegane uno o usa ANDROID_SERIAL."; exit 2; }
echo "📱 device: $("$ADB" devices -l | grep -w device | awk '{print $1, $4}')"

# --- attesa attiva senza sleep locale (roundtrip adb) -----------------------
adb_wait() { local n="$1"; for _ in $(seq 1 "$n"); do "$ADB" shell true >/dev/null 2>&1; done; }

# --- reinstall opzionale -----------------------------------------------------
if [ -n "$APK" ]; then
  [ -f "$APK" ] || { echo "❌ APK non trovato: $APK"; exit 2; }
  echo "📦 reinstallo $APK"
  "$ADB" install -r "$APK" >/dev/null 2>&1 || { echo "❌ install fallita"; exit 2; }
fi

# --- wipe dati app opzionale (DB cifrato con vecchia chiave, ecc.) -----------
if [ "${WIPE:-0}" = "1" ]; then
  echo "🧹 cancello i dati dell'app (DB/sandbox)…"
  "$ADB" shell pm clear "$PKG" >/dev/null 2>&1
fi

# --- avvio pulito ------------------------------------------------------------
echo "🚀 avvio pulito di ${PKG}…"
"$ADB" shell am force-stop "$PKG" >/dev/null 2>&1
"$ADB" logcat -c -b all >/dev/null 2>&1
"$ADB" shell am start -n "$ACT" >/dev/null 2>&1
# il PRIMO avvio dopo install/wipe è lento (dexopt + estrazione native libs): aspetta il PID con retry
START_PID=""
for _ in $(seq 1 30); do
  START_PID="$("$ADB" shell pidof "$PKG" 2>/dev/null | tr -d '\r')"
  [ -n "$START_PID" ] && break
  adb_wait 5
done
[ -z "$START_PID" ] && { echo "❌ l'app non è partita affatto (nessun PID dopo ~8s)."; exit 1; }
echo "   PID=$START_PID — osservo per ~${WATCH_SECONDS}s…"

# --- watch del PID -----------------------------------------------------------
# ~20 roundtrip adb ≈ 1s; loop finché muore o scade il tempo
DIED=""
for i in $(seq 1 "$WATCH_SECONDS"); do
  PID="$("$ADB" shell pidof "$PKG" 2>/dev/null | tr -d '\r')"
  if [ -z "$PID" ]; then DIED="dopo ~${i}s"; break; fi
  adb_wait 20
done

# --- scansione logcat per cause note ----------------------------------------
LOG="$("$ADB" logcat -d -b all 2>/dev/null)"
FATAL="$(printf '%s\n' "$LOG" | grep -iE "Fatal signal|SIGSEGV|SIGABRT|signal (6|11)|>>> $PKG|thread '.*' panicked|RustStdoutStderr.*panic|abort message" | head -20)"
OOM="$(printf '%s\n' "$LOG"   | grep -iE "lowmemorykiller|lmkd.*killing|Out of memory|OutOfMemory" | grep -i "$PKG" | head -10)"

echo "----------------------------------------------------------------------"
FAIL=0
if [ -n "$DIED" ]; then echo "❌ PROCESSO MORTO $DIED dall'avvio"; FAIL=1; else echo "✅ processo vivo per tutta la finestra (${WATCH_SECONDS}s)"; fi
if [ -n "$FATAL" ]; then echo "❌ CRASH NATIVO/PANIC nel log:"; printf '   %s\n' "$FATAL"; FAIL=1; else echo "✅ nessun panic/SIGSEGV/SIGABRT"; fi
if [ -n "$OOM" ];   then echo "❌ OOM / lowmemorykiller:";        printf '   %s\n' "$OOM";   FAIL=1; else echo "✅ nessun OOM/kill"; fi
echo "----------------------------------------------------------------------"

if [ "$FAIL" -eq 0 ]; then
  echo "🟢 SMOKE OK — l'avvio è sano."
else
  echo "🔴 SMOKE FALLITO — log completo: $ADB logcat -d -b all | grep $PKG"
fi
exit "$FAIL"
