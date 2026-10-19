<div align="center">

# LoreLog

**A personal lifelog app that gathers your day as text on your own Android, as raw material for a personal knowledge graph about you.**

Personal use · Encrypted local storage · Cloud transcription & summaries (your keys)

[**Download APK**](https://github.com/jwgung/lifelog/releases/latest) · [Releases](https://github.com/jwgung/lifelog/releases)

English · [한국어](README.ko.md)

</div>

---

Your phone is a sensor that's always with you. LoreLog collects what happens on your phone and the daily life your phone witnesses beside you — voice, app usage, on-screen content, location, notifications, calls/SMS, calendar — into one chronological timeline. Everything stays encrypted on your device, and when you choose, you export it as JSON, files, or HTTP to attach to your personal knowledge graph (*your lore*).

> ⚠️ **This is a personal tool.** It is built to collect your own data on your own device. It is not for covertly surveilling other people, and must not be used that way.

## Principles

- **Your device, your data only.** Not third-party surveillance or covert collection.
- **Visible.** All collection is surfaced through standard OS permission prompts and a **persistent notification**. Nothing is hidden. It collects only while recording.
- **Stored locally, encrypted.** Records live on-device in a SQLCipher-encrypted DB; API keys sit in a Keystore-backed store. This app stores nothing in the cloud.
- **But processing is cloud by default — this is not a local-only app.** Voice transcription (Groq) and summaries (Gemini) run in the cloud with your own keys, so speech audio and text leave the device for that. Exposure is kept minimal (VAD sends only speech, speaker ID stays on-device, summaries can run on-device via Gemini Nano where supported) and you control every endpoint and key. Only text is stored; audio is discarded after transcription (kept offline only until it can be sent).

## What it collects

Everything runs in one foreground service, and you toggle each source in settings.

| Source | How | Notes |
|---|---|---|
| **Voice** | Record → Groq Whisper transcription | WebRTC VAD captures **only when someone speaks**. Keeps running with the screen off. Text only |
| **App usage** | UsageStatsManager | Which app, when |
| **On-screen content** | AccessibilityService | Text you see inside apps. **Sensitive apps (banking, payments) excluded** |
| **Location** | FusedLocationProvider | Every 5 min, low power, batched |
| **Notifications** | NotificationListenerService | Title and body of notifications you receive |
| **Calls / SMS** | CallLog · SMS · Contacts | Who and when (number → name) |
| **Calendar** | CalendarContract (+attendees) | All calendars on the phone, local included |

## Voice

The core of LoreLog, and the trickiest part. Google's live recognizer keeps grabbing audio focus (interrupting your media), which is wrong for always-on capture, so voice is **recording-based**.

- **VAD capture** — `AudioRecord` reads raw PCM and **WebRTC VAD** keeps only the segments where a human is speaking. When it's quiet, it just watches the sound level, so the load is near zero. It never grabs audio focus, so your media keeps playing.
- **Transcription** — speech segments go to Groq Whisper (`/audio/transcriptions`). A free key is required.
- **Offline: keep, then transcribe** — with no AI connection, speech segments are kept as voice files in an app folder, and once connected they are transcribed **automatically (a periodic worker) or manually ("Transcribe pending voice")**. Recording and keeping continue even without a key.
- **Speaker ID (optional)** — enroll your own voice once (**on-device Picovoice Eagle**) and each segment is tagged `me / other / unknown`. Audio never leaves the device.
- **Bluetooth earbud mic (optional)** — route recording input to a BT headset mic.
- **Import call recordings** — the OS blocks live call audio. Instead, point LoreLog at the folder where your dialer saves call recordings (via SAF) and it transcribes those files (devices with a built-in recorder, e.g. Samsung).

## Conversations · Summaries

- **Conversation grouping** — voice segments are grouped by time gaps into "conversations," and attendees are auto-linked from overlapping calls (contacts) and meetings (calendar attendees). Ambiguous ones go to a **review queue** to tag by hand.
- **Summaries** — the collection is summarized periodically. Default is a cloud LLM (OpenAI-compatible, default Gemini 2.0 Flash), falling back to on-device **Gemini Nano** on supported devices. Models are fetched from the API's `/models` and chosen from a **dropdown** (no app update needed).

## Export / Delete

- **JSON** — the whole log via the share sheet.
- **Folder (incremental NDJSON)** — only new records appended to an app-private external folder. `Android/data/com.lorelog/files/exports/`. Reachable from a PC over USB/adb.
- **HTTP endpoint** — POST new records to a URL you set (`Authorization: Bearer <token>`, NDJSON). Straight into a knowledge-graph ingest.
- Every line carries a `type` (voice/app/screen/location/notification/comm/calendar/conversation/summary).
- **Delete all** — wipes every record after confirmation (irreversible).

## Storage · Security · Retention

- **Encryption** — the DB is encrypted with SQLCipher. The key is a random 32 bytes stored in `EncryptedSharedPreferences` protected by the Android Keystore (never leaves the device). API keys are stored the same way and never committed.
- **Retention** — only the last **30 days** on the device. Originals that have been exported and are older than 30 days are auto-deleted (summaries and conversations are kept). Long-term storage is the job of the knowledge graph you export to.
- **Notification / display** — the persistent notification comes back if dismissed (always kept). In settings you can hide notifications (hide the idle one, minimize the recording one) and enable a floating "monitoring" badge over other apps (overlay).
- Note: exported files, HTTP bodies, and cloud transcription/summary inputs leave the device **in plaintext**. If that's sensitive, manage the destination yourself (folder access, HTTPS endpoint, no-training terms).

## Install & Setup

1. Download `app-release.apk` from [Releases](https://github.com/jwgung/lifelog/releases/latest) and install it.
2. **Play Protect warning**: with this many permissions, sideloading triggers a warning. It can't be removed in code; to use the app, turn off Play Protect scanning in the Play Store (or "install without scanning"), and on Samsung turn off Auto Blocker.
3. In the app, tap **Start** and grant the prompted permissions (it starts with just the mic granted).
4. Paste the **keys** for the features you want into settings: voice transcription (Groq), summaries (Gemini), speaker ID (Picovoice). Each field has a "get key" link. Without a key, only that feature sits idle; everything else works.

**Special permissions** (you enable these yourself): usage access, accessibility (on-screen content), notification access, background location.

> 🔒 **New Android / Samsung note**: accessibility (on-screen content capture) may be hard-blocked for sideloaded apps by security hardening. You have to go through App info → ⋮ → "Allow restricted settings", and if it's still blocked, give up on-screen capture on that device and use the rest. The sources that don't depend on accessibility are the backbone.

## Build

```bash
./gradlew :app:assembleDebug      # debug APK
./gradlew :app:assembleRelease    # release APK (signing config required)
```

- Toolchain: AGP 9.4.1 (built-in Kotlin), Gradle 9.8.0, compileSdk 37 / targetSdk 36 / minSdk 29, Jetpack Compose.
- Key dependencies: SQLCipher (net.zetetic), security-crypto, WorkManager, play-services-location, ML Kit genai-summarization, Picovoice Eagle, android-vad (WebRTC).
- Set the Android SDK path via `sdk.dir` in `local.properties`. Keep the release signing keystore and passwords outside the repo.

## Structure

```
MainActivity (Compose)   start/stop · timeline · permission guidance · settings · review queue · export/delete
LoggerService (FGS)      persistent notification + collectors (starts foreground with only granted types)
collect/
  RecordingVoiceCollector  AudioRecord + WebRTC VAD -> WAV -> transcription
  PendingVoice             offline voice keeping + pending transcription
  AppUsage / Location / Comms / Calendar Collector
  CallRecordingImporter    transcribe a call-recording folder (SAF)
  SensitiveApps · CollectorPrefs · AudioPrefs
speaker/  SpeakerId (Picovoice Eagle) · PcmDecoder
service/  ScreenContentService (accessibility) · NotificationService (notification listener)
data/     LogStore (SQLCipher) · LogRepository (StateFlow) · DbKey · event models
net/      SttConfig · GroqTranscriber · LlmConfig · ModelCatalog · EndpointConfig · HttpSink
summary/  Summarizer · CloudSummarizer · GeminiNanoSummarizer
work/     Export · Summary · Prune · Conversation · PendingVoice · CallRecording Worker
OverlayBadge · NotifPrefs · NotifReviveReceiver · BootReceiver
```

**Design note** — the phone captures what only exists because the phone is beside you, or what the phone gives for free locally (the sources above); things that live only in the cloud and need auth (email, etc.) are pulled on the server / knowledge-graph side. Voice STT, summarization, and speaker ID all sit behind interfaces so the engines can be swapped.

## Legal note

In Korea, recording a conversation **you take part in** is legal, but recording others' conversations you are not part of is not. The rules for recording calls and conversations differ by country (some require all-party consent). Keep logs personal, and don't publish or distribute other people's data.

## Roadmap

- An on-screen-capture alternative for new devices (MediaProjection + on-device OCR) to bypass the accessibility block.
- Real-device verification of call/meeting auto-linking; battery-impact measurement and tuning.
- Candidate life sources: health (steps, sleep), photo/media metadata. Email belongs on the server / KG side, not the phone.

## Disclaimer

A personal learning/experimentation project. Provided as-is, no warranty. Complying with local laws on collection and recording is the user's responsibility.
