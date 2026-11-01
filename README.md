<div align="center">

# LoreLog

**A personal lifelog app that gathers your day as text on your own Android, as raw material for a personal knowledge graph about you.**

Personal use · Encrypted local storage · The phone only collects · Transcription and summaries in your vault (models you choose)

[**Download APK**](https://github.com/jwgung/LoreLog/releases/latest) · [Releases](https://github.com/jwgung/LoreLog/releases)

English · [한국어](README.ko.md)

</div>

---

Your phone is a sensor that's always with you. LoreLog collects what happens on your phone and the daily life your phone witnesses beside you — voice, app usage, on-screen content, location, notifications, calls/SMS, calendar — into one chronological timeline. Everything stays encrypted on your device, and when you choose, you export it as JSON, files, or HTTP to attach to your personal knowledge graph (*your lore*).

> ⚠️ **This is a personal tool.** It is built to collect your own data on your own device. It is not for covertly surveilling other people, and must not be used that way.

## Principles

- **Your device, your data only.** Not third-party surveillance or covert collection.
- **Visible.** All collection is surfaced through standard OS permission prompts and a **persistent notification**. Nothing is hidden. It collects only while recording.
- **Stored locally, encrypted.** Records live on-device in a SQLCipher-encrypted DB; the vault token sits in a Keystore-backed store. This app stores nothing in the cloud.
- **The phone only collects. Processing happens in your vault, with models you choose.** The phone neither transcribes nor summarizes. Speech is sent to your **vault** (금고: your own server or the LoreLog desktop), where the speech model you configured transcribes it; summaries and the daily digest are made there too, with the model you chose. The phone never sends records or audio to Google or any other cloud. When the vault can't be reached, records and audio stay on the phone and are sent later. Exposure is kept minimal (VAD records only speech) and you set the vault address and token. Text is the record; audio is kept only briefly (as AAC) so you can replay a segment and hear what it was. It is deleted as soon as the segment is exported, or after 7 days otherwise, and is never included in exports.

## What it collects

Everything runs in one foreground service, and you toggle each source in settings.

| Source | How | Notes |
|---|---|---|
| **Voice** | Record → sent to the vault for transcription | Silero VAD (neural) captures **only when someone speaks**. Keeps running with the screen off. Text only |
| **App usage** | UsageStatsManager | Which app, when |
| **On-screen content** | AccessibilityService | Text you see inside apps. **Sensitive apps (banking, payments) excluded** |
| **Location** | FusedLocationProvider | Every 5 min, low power, batched |
| **Notifications** | NotificationListenerService | Title and body of notifications you receive |
| **Calls / SMS** | CallLog · SMS · Contacts | Who and when (number → name) |
| **Calendar** | CalendarContract (+attendees) | All calendars on the phone, local included |
| **Contacts** | ContactsContract | Everything the address book holds (name, nickname, emails, phones, company and title, addresses, websites, notes, birthdays and anniversaries, relations, messengers, groups), the same as from CardDAV; photos are left out. The whole list is sent to the vault's `/ingest/contacts`, only when it changed. The librarian uses it to recognize people. Sent only over https or to a tailnet address. Turning it off in the collectors sends an empty list once, so it leaves the vault's index too |

## Voice

The core of LoreLog, and the trickiest part. Google's live recognizer keeps grabbing audio focus (interrupting your media), which is wrong for always-on capture, so voice is **recording-based**.

- **Mic yielding** — Android lets one app hold the mic. When a voice-recorder or camera app comes to the front (needs usage-access permission) during a call (cellular or VoIP), or when another app actually starts recording, LoreLog releases the mic and reopens it when they are done. For apps not on the list, use "Yield mic 10 min" in the notification or on the main screen.
- **VAD capture** — `AudioRecord` reads raw PCM and **Silero VAD** (a small neural network, ONNX) keeps only the segments where a human is speaking and ignores keyboards, dishes and air-con. Batches shorter than 3 s of speech are dropped. Inference is ~1 ms per 32 ms frame, so the idle load is near zero. It never grabs audio focus, so your media keeps playing.
- **The vault transcribes** — the phone packs speech into WAV and sends it to the vault's `/transcribe`, which forwards it to the speech model you chose (`LIFE_ASR_URL` / `LIFE_ASR_MODEL`, or the settings screen on the desktop). There is no transcription engine and no API key on the phone. Nearby utterances are batched into ~4-minute chunks (a batch also closes after a 3-minute lull): fewer requests, and better accuracy than tiny clips.
- **Vault unreachable: keep, then send** — when there is no vault address or the vault can't be reached, speech segments are kept as voice files in an app folder and sent once it's reachable, **automatically (a periodic worker) or manually ("밀린 음성 전사")**. Recording and keeping continue regardless.
- **Dictation** — "말하기" on the chat screen also uses the vault's `/transcribe`. The recording is deleted right after it's transcribed.
- **Bluetooth earbud mic (optional)** — route recording input to a BT headset mic.
- **Import call recordings** — the OS blocks live call audio. Instead, point LoreLog at the folder where your dialer saves call recordings (via SAF) and it sends those files to the vault for transcription (devices with a built-in recorder, e.g. Samsung). If the vault can't be reached, they are retried on the next check.

## Conversations

- **Conversation grouping** — voice segments are grouped by time gaps into "conversations," and attendees are auto-linked from overlapping calls (contacts) and meetings (calendar attendees). Ambiguous ones go to a **review queue** that shows a transcript excerpt and a ▶ play button for each conversation, so you can tell who it was before tagging it (or skip it).
- **No summaries on the phone** — the phone sends a faithful copy to the vault, and the vault makes the daily digest and summaries with the model you chose (and can redo them as its schema evolves).

## Export / Delete

- **JSON** — the whole log via the share sheet.
- **Folder (incremental NDJSON)** — only new records appended to an app-private external folder. `Android/data/com.lorelog/files/exports/`. Reachable from a PC over USB/adb.
- **Send to the vault (HTTP)** — POST new records to your vault address every 15 minutes (`Authorization: Bearer <token>`, NDJSON). Turn on **"와이파이에 연결됐을 때만 보내기"** (send only on Wi-Fi) under settings → 금고 연결 and the automatic send runs only on an unmetered network (Wi-Fi); records wait on the phone until then. It is off by default. "지금 전송" (send now) always sends right away.
- Every line carries a `type` (voice/app/screen/location/notification/comm/calendar/conversation).
- **Delete all** — wipes every record after confirmation (irreversible).

## Storage · Security · Retention

- **Encryption** — the DB is encrypted with SQLCipher. The key is a random 32 bytes stored in `EncryptedSharedPreferences` protected by the Android Keystore (never leaves the device). The vault token is stored the same way and never committed.
- **Retention** — only the last **30 days** on the device. Originals that have been exported and are older than 30 days are auto-deleted (conversations are kept). Long-term storage is the job of the knowledge graph you export to.
- **Voice audio** — the audio behind each transcribed segment is kept as AAC (~0.35MB/min) in `Android/data/com.lorelog/files/voice_kept/` and can be replayed from the timeline (**▶ 듣기**). Audio for segments included in a folder/HTTP export is deleted right away; audio never exported is deleted after **7 days**. Files still waiting for transcription also show up in the timeline as "전사 대기" and can be played. Capture keeps running during playback: the acoustic echo canceller (AEC) subtracts the played sound from the mic, anything still captured is labelled "재생 중", and a transcript that merely repeats the played text is dropped (mixed-in conversation is kept).
- **Notification / display** — the persistent notification comes back if dismissed (always kept). In settings you can hide notifications (hide the idle one, minimize the recording one) and enable a floating "monitoring" badge over other apps (overlay).
- Note: the app doesn't add its own encryption to exported files or to what it sends to the vault (records and audio). Keep the vault address on HTTPS or a tailnet, and manage folder access yourself.

## Install & Setup

1. Download the APK (`LoreLog-<version>.apk`) from [Releases](https://github.com/jwgung/LoreLog/releases/latest) and install it.
2. **Play Protect warning**: with this many permissions, sideloading triggers a warning. It can't be removed in code; to use the app, turn off Play Protect scanning in the Play Store (or "install without scanning"), and on Samsung turn off Auto Blocker.
3. In the app, tap **Start** and grant the prompted permissions (it starts with just the mic granted).
4. In the drawer → 수집·연동 설정 → **금고 연결** (vault connection), enter your vault address (e.g. `https://<vault>/ingest/lorelog`) and token, then save. Transcription, dictation and the diary all use the same address and token. Without an address, collection keeps going; records and audio wait on the phone and are sent once it's connected.

**Special permissions** (you enable these yourself): usage access, accessibility (on-screen content), notification access, background location.

> 🔒 **New Android / Samsung note**: accessibility (on-screen content capture) may be hard-blocked for sideloaded apps by security hardening. You have to go through App info → ⋮ → "Allow restricted settings", and if it's still blocked, give up on-screen capture on that device and use the rest. The sources that don't depend on accessibility are the backbone.

## Build

```bash
./gradlew :app:assembleDebug      # debug APK
./gradlew :app:assembleRelease    # release APK (signing config required)
```

- Toolchain: AGP 9.4.1 (built-in Kotlin), Gradle 9.8.0, compileSdk 37 / targetSdk 36 / minSdk 29, Jetpack Compose.
- Key dependencies: SQLCipher (net.zetetic), security-crypto, WorkManager, play-services-location, android-vad (Silero/ONNX).
- Set the Android SDK path via `sdk.dir` in `local.properties`. Keep the release signing keystore and passwords outside the repo.

## Structure

```
MainActivity (Compose)   start/stop · timeline · permission guidance · settings · review queue · export/delete
LoggerService (FGS)      persistent notification + collectors (starts foreground with only granted types)
collect/
  RecordingVoiceCollector  AudioRecord + Silero VAD -> WAV -> vault transcription
  PendingVoice             keeps voice while the vault is unreachable + pending transcription
  Dictation                chat-screen dictation (vault /transcribe)
  AppUsage / Location / Comms / Calendar Collector
  CallRecordingImporter    transcribe a call-recording folder (SAF)
  SensitiveApps · CollectorPrefs · AudioPrefs
service/  ScreenContentService (accessibility) · NotificationService (notification listener)
data/     LogStore (SQLCipher) · LogRepository (StateFlow) · DbKey · event models
net/      ServerTranscriber · EndpointConfig · HttpSink · DiaryClient · UpdateChecker
work/     Push (+PushPrefs) · Export · Prune · Conversation · PendingVoice · CallRecording · DiaryReminder Worker
OverlayBadge · NotifPrefs · NotifReviveReceiver · BootReceiver
```

**Design note** — the phone captures what only exists because the phone is beside you, or what the phone gives for free locally (the sources above); things that live only in the cloud and need auth (email, etc.) are pulled on the vault side. Anything that needs a model, like transcription and summaries, runs in the vault with the model you chose.

## Legal note

In Korea, recording a conversation **you take part in** is legal, but recording others' conversations you are not part of is not. The rules for recording calls and conversations differ by country (some require all-party consent). Keep logs personal, and don't publish or distribute other people's data.

## Roadmap

- An on-screen-capture alternative for new devices (MediaProjection + on-device OCR) to bypass the accessibility block.
- Real-device verification of call/meeting auto-linking; battery-impact measurement and tuning.
- Candidate life sources: health (steps, sleep), photo/media metadata. Email belongs on the vault side, not the phone.

## Disclaimer

A personal learning/experimentation project. Provided as-is, no warranty. Complying with local laws on collection and recording is the user's responsibility.
