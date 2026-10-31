<div align="center">

# LoreLog

**A personal lifelog app that gathers your day as text on your own Android, as raw material for a personal knowledge graph about you.**

Personal use · Encrypted local storage · Cloud or on-device transcription · cloud summaries (your keys)

[**Download APK**](https://github.com/jwgung/LoreLog/releases/latest) · [Releases](https://github.com/jwgung/LoreLog/releases)

English · [한국어](README.ko.md)

</div>

---

Your phone is a sensor that's always with you. LoreLog collects what happens on your phone and the daily life your phone witnesses beside you — voice, app usage, on-screen content, location, notifications, calls/SMS, calendar — into one chronological timeline. Everything stays encrypted on your device, and when you choose, you export it as JSON, files, or HTTP to attach to your personal knowledge graph (*your lore*).

> ⚠️ **This is a personal tool.** It is built to collect your own data on your own device. It is not for covertly surveilling other people, and must not be used that way.

## Principles

- **Your device, your data only.** Not third-party surveillance or covert collection.
- **Visible.** All collection is surfaced through standard OS permission prompts and a **persistent notification**. Nothing is hidden. It collects only while recording.
- **Stored locally, encrypted.** Records live on-device in a SQLCipher-encrypted DB; API keys sit in a Keystore-backed store. This app stores nothing in the cloud.
- **Processing is cloud by default, but transcription can be fully on-device.** Out of the box, voice transcription and summaries run in the cloud with your own keys, so speech audio and text leave the device. You can switch transcription to **on-device whisper.cpp** (download a model once; then nothing leaves the device), and summaries fall back to on-device **Gemini Nano** where supported. Exposure is kept minimal (VAD sends only speech) and you control every endpoint and key. Text is the record; audio is kept only briefly (as AAC) so you can replay a segment and hear what it was. It is deleted as soon as the segment is exported, or after 7 days otherwise, and is never included in exports.

## What it collects

Everything runs in one foreground service, and you toggle each source in settings.

| Source | How | Notes |
|---|---|---|
| **Voice** | Record → transcription (Gemini / on-device whisper) | Silero VAD (neural) captures **only when someone speaks**. Keeps running with the screen off. Text only |
| **App usage** | UsageStatsManager | Which app, when |
| **On-screen content** | AccessibilityService | Text you see inside apps. **Sensitive apps (banking, payments) excluded** |
| **Location** | FusedLocationProvider | Every 5 min, low power, batched |
| **Notifications** | NotificationListenerService | Title and body of notifications you receive |
| **Calls / SMS** | CallLog · SMS · Contacts | Who and when (number → name) |
| **Calendar** | CalendarContract (+attendees) | All calendars on the phone, local included |
| **Contacts** | ContactsContract | Name, email, phone and company only (no photos, notes or birthdays). The whole list is sent to the server's `/ingest/contacts`, only when it changed. The librarian uses it to recognize people. Can be turned off in the collectors |

## Voice

The core of LoreLog, and the trickiest part. Google's live recognizer keeps grabbing audio focus (interrupting your media), which is wrong for always-on capture, so voice is **recording-based**.

- **Mic yielding** — Android lets one app hold the mic. When a voice-recorder or camera app comes to the front (needs usage-access permission) during a call (cellular or VoIP), or when another app actually starts recording, LoreLog releases the mic and reopens it when they are done. For apps not on the list, use "Yield mic 10 min" in the notification or on the main screen.
- **VAD capture** — `AudioRecord` reads raw PCM and **Silero VAD** (a small neural network, ONNX) keeps only the segments where a human is speaking and ignores keyboards, dishes and air-con. Batches shorter than 3 s of speech are dropped. Inference is ~1 ms per 32 ms frame, so the idle load is near zero. It never grabs audio focus, so your media keeps playing.
- **Transcription — you choose the engine** (settings → "Voice transcription"):
  - **Cloud · Gemini** — uses the "Gemini API key" from settings (shared with summaries). Free tier (Flash/Flash-Lite are free; the model dropdown marks free/paid, retired and slow models). (audio goes to Google)
  - **On-device · whisper.cpp** — runs inside the phone, nothing leaves the device. Pick accuracy by model: tiny (~75MB) / base (~142MB, recommended) / small (~466MB); the model downloads once. arm64 devices only. The model is released from RAM when you switch to cloud.

  Cloud is more accurate; on-device trades some accuracy for full privacy and no key. Nearby utterances are batched into ~4-minute chunks before transcription (a batch also closes after a 3-minute lull) — fewer requests (the free tier is ~500 requests/day/model; 4-minute batches keep even non-stop talk at 15/hour) and better accuracy than tiny clips.
- **Offline: keep, then transcribe** — when the chosen engine isn't ready (no key, or the on-device model isn't downloaded), speech segments are kept as voice files in an app folder, and once it's ready they are transcribed **automatically (a periodic worker) or manually ("Transcribe pending voice")**. Recording and keeping continue regardless.
- **Bluetooth earbud mic (optional)** — route recording input to a BT headset mic.
- **Import call recordings** — the OS blocks live call audio. Instead, point LoreLog at the folder where your dialer saves call recordings (via SAF) and it transcribes those files (devices with a built-in recorder, e.g. Samsung).

## Conversations · Summaries

- **Conversation grouping** — voice segments are grouped by time gaps into "conversations," and attendees are auto-linked from overlapping calls (contacts) and meetings (calendar attendees). Ambiguous ones go to a **review queue** that shows a transcript excerpt and a ▶ play button for each conversation, so you can tell who it was before tagging it (or skip it).
- **Summaries (optional, off by default)** — summarizing on the phone is optional, because the knowledge graph is better fed a faithful export and left to summarize there (and to re-summarize as its schema evolves). Turn it on and the collection is summarized periodically for in-app reading, and summaries are included in exports. Engine is Gemini Flash (default 3.5 Flash-Lite, switching to another Flash model automatically when the quota runs out — nothing to pick), falling back to on-device **Gemini Nano** on supported devices.

## Export / Delete

- **JSON** — the whole log via the share sheet.
- **Folder (incremental NDJSON)** — only new records appended to an app-private external folder. `Android/data/com.lorelog/files/exports/`. Reachable from a PC over USB/adb.
- **HTTP endpoint** — POST new records to a URL you set (`Authorization: Bearer <token>`, NDJSON). Straight into a knowledge-graph ingest.
- Every line carries a `type` (voice/app/screen/location/notification/comm/calendar/conversation/summary); `summary` lines are included only when summaries are enabled.
- **Delete all** — wipes every record after confirmation (irreversible).

## Storage · Security · Retention

- **Encryption** — the DB is encrypted with SQLCipher. The key is a random 32 bytes stored in `EncryptedSharedPreferences` protected by the Android Keystore (never leaves the device). API keys are stored the same way and never committed.
- **Retention** — only the last **30 days** on the device. Originals that have been exported and are older than 30 days are auto-deleted (summaries and conversations are kept). Long-term storage is the job of the knowledge graph you export to.
- **Voice audio** — the audio behind each transcribed segment is kept as AAC (~0.35MB/min) in `Android/data/com.lorelog/files/voice_kept/` and can be replayed from the timeline (**▶ 듣기**). Audio for segments included in a folder/HTTP export is deleted right away; audio never exported is deleted after **7 days**. Files still waiting for transcription also show up in the timeline as "전사 대기" and can be played. Capture keeps running during playback: the acoustic echo canceller (AEC) subtracts the played sound from the mic, anything still captured is labelled "재생 중", and a transcript that merely repeats the played text is dropped (mixed-in conversation is kept).
- **Notification / display** — the persistent notification comes back if dismissed (always kept). In settings you can hide notifications (hide the idle one, minimize the recording one) and enable a floating "monitoring" badge over other apps (overlay).
- Note: exported files, HTTP bodies, and cloud transcription/summary inputs leave the device **in plaintext**. If that's sensitive, manage the destination yourself (folder access, HTTPS endpoint, no-training terms).

## Install & Setup

1. Download the APK (`LoreLog-<version>.apk`) from [Releases](https://github.com/jwgung/LoreLog/releases/latest) and install it.
2. **Play Protect warning**: with this many permissions, sideloading triggers a warning. It can't be removed in code; to use the app, turn off Play Protect scanning in the Play Store (or "install without scanning"), and on Samsung turn off Auto Blocker.
3. In the app, tap **Start** and grant the prompted permissions (it starts with just the mic granted).
4. Paste one **Gemini API key** into settings (shared by transcription and summaries; saved as you type, "키 확인" validates it). For transcription you can pick on-device whisper and download a model instead. Without a key (or a downloaded model), only that feature sits idle; everything else works.

**Special permissions** (you enable these yourself): usage access, accessibility (on-screen content), notification access, background location.

> 🔒 **New Android / Samsung note**: accessibility (on-screen content capture) may be hard-blocked for sideloaded apps by security hardening. You have to go through App info → ⋮ → "Allow restricted settings", and if it's still blocked, give up on-screen capture on that device and use the rest. The sources that don't depend on accessibility are the backbone.

## Build

```bash
./gradlew :app:assembleDebug      # debug APK
./gradlew :app:assembleRelease    # release APK (signing config required)
```

- Toolchain: AGP 9.4.1 (built-in Kotlin), Gradle 9.8.0, compileSdk 37 / targetSdk 36 / minSdk 29, Jetpack Compose.
- Key dependencies: SQLCipher (net.zetetic), security-crypto, WorkManager, play-services-location, ML Kit genai-summarization, android-vad (Silero/ONNX), whisper-android (dev.ffmpegkit-maintained, no NDK).
- Set the Android SDK path via `sdk.dir` in `local.properties`. Keep the release signing keystore and passwords outside the repo.

## Structure

```
MainActivity (Compose)   start/stop · timeline · permission guidance · settings · review queue · export/delete
LoggerService (FGS)      persistent notification + collectors (starts foreground with only granted types)
collect/
  RecordingVoiceCollector  AudioRecord + Silero VAD -> WAV -> transcription
  PendingVoice             offline voice keeping + pending transcription
  AppUsage / Location / Comms / Calendar Collector
  CallRecordingImporter    transcribe a call-recording folder (SAF)
  SensitiveApps · CollectorPrefs · AudioPrefs
service/  ScreenContentService (accessibility) · NotificationService (notification listener)
data/     LogStore (SQLCipher) · LogRepository (StateFlow) · DbKey · event models
net/      GeminiTranscriber · LlmConfig · ModelCatalog · TranscribeConfig · WhisperModels · EndpointConfig · HttpSink
  stt/      WhisperEngine (whisper.cpp) · ModelDownloader
  summary/  Summarizer · CloudSummarizer · GeminiNanoSummarizer
work/     Export · Summary · Prune · Conversation · PendingVoice · CallRecording Worker
OverlayBadge · NotifPrefs · NotifReviveReceiver · BootReceiver
```

**Design note** — the phone captures what only exists because the phone is beside you, or what the phone gives for free locally (the sources above); things that live only in the cloud and need auth (email, etc.) are pulled on the server / knowledge-graph side. Voice STT and summarization sit behind interfaces so the engines can be swapped.

## Legal note

In Korea, recording a conversation **you take part in** is legal, but recording others' conversations you are not part of is not. The rules for recording calls and conversations differ by country (some require all-party consent). Keep logs personal, and don't publish or distribute other people's data.

## Roadmap

- An on-screen-capture alternative for new devices (MediaProjection + on-device OCR) to bypass the accessibility block.
- Real-device verification of call/meeting auto-linking; battery-impact measurement and tuning.
- Candidate life sources: health (steps, sleep), photo/media metadata. Email belongs on the server / KG side, not the phone.

## Disclaimer

A personal learning/experimentation project. Provided as-is, no warranty. Complying with local laws on collection and recording is the user's responsibility.
