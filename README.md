<div align="center">

# LoreLog

**A personal lifelog app that gathers your day as text on your own Android, as raw material for a personal knowledge graph about you.**

Personal use · Encrypted local storage · The phone only collects · Transcription and summaries in your drawer (models you choose)

[**Download APK · desktop app**](https://github.com/jwgung/LoreLog/releases/latest) · [Releases](https://github.com/jwgung/LoreLog/releases)

English · [한국어](README.ko.md)

</div>

---

Your phone is a sensor that's always with you. LoreLog collects what happens on your phone and the daily life your phone witnesses beside you — voice, app usage, on-screen content, location, notifications, calls/SMS, calendar — into one chronological timeline. Everything stays encrypted on your device, and when you choose, you export it as JSON, files, or HTTP to attach to your personal knowledge graph (*your lore*).

> ⚠️ **This is a personal tool.** It is built to collect your own data on your own device. It is not for covertly surveilling other people, and must not be used that way.

## Getting started

From installing to the first conversation, in this order. The desktop app walks you through steps 2–6 itself on the first start (**처음 설정**; skip any step and reopen it later from Settings → "처음 설정 안내 다시 열기").

1. **Install.** Get the desktop app for your OS and the Android APK from [Releases](https://github.com/jwgung/LoreLog/releases/latest): files and first-start warnings are under [Desktop app](#desktop-app-windows--macos--linux) and [Install & Setup](#install--setup).
2. **The drawer.** On the first start choose **이 PC에 두기** (keep it on this PC). The app then runs the drawer: records, mail and calendar notes, and the search index all stay on this PC.
3. **The model.** The assistant and the librarian ask a model you choose, at an OpenAI-compatible address. There is no built-in one, and your records go to that address, so use your own PC or a server you trust. Examples:
   - [Ollama](https://ollama.com): `http://127.0.0.1:11434/v1`, model `qwen3:8b` (first `ollama pull qwen3:8b`). Give Ollama `OLLAMA_CONTEXT_LENGTH=8192` so long mails fit, and `OLLAMA_HOST=0.0.0.0` if the drawer calls it from another machine; restart Ollama after setting them.
   - [LM Studio](https://lmstudio.ai): start the server under Developer, `http://127.0.0.1:1234/v1`, with the model name as LM Studio shows it.

   Press **연결 확인** (check): the app sends the model a tiny request and tells you what to fix if it does not answer.
   > **A small model on a CPU** (8B or so) is fine for chatting and search, but the relation graph is too slow with it: about 4 minutes per mail, and few relations survive. For the graph, use a PC with a GPU or a model on a server.
4. **Mail and calendar.** Your IMAP account (read only; services with two-step login need an app password) and CalDAV calendars, with the same forms as in Settings.
5. **The phone.** Install [Tailscale](https://tailscale.com/download) on the PC and the phone and sign in with the same account. In the guide's phone step, click **연결 코드 보기** (show the connection code): one line `lorelog://connect?…` with a copy button and a QR code. On the phone, open drawer → 수집·연동 설정 → **서랍 연결**, paste it into **연결 코드 붙여 넣기**, and tap **코드로 채우고 저장**. The phone asks for this on its first start too. **The code contains the drawer's token**: whoever has it can read and write your drawer, so put it only on your own phone. The phone accepts it only for an https or tailnet address, like an address typed by hand.
6. **First sync.** The last step shows the sync: records indexed, and how many mails the graph has read and how many wait. It continues every hour; you can start chatting before it is done.

## Principles

- **Your device, your data only.** Not third-party surveillance or covert collection.
- **Visible.** All collection is surfaced through standard OS permission prompts and a **persistent notification**. Nothing is hidden. It collects only while recording.
- **Stored locally, encrypted.** Records live on-device in a SQLCipher-encrypted DB; the drawer token sits in a Keystore-backed store. This app stores nothing in the cloud.
- **The phone only collects. Processing happens in your drawer, with models you choose.** The phone neither transcribes nor summarizes. Speech is sent to your **drawer** (서랍: your own server or the LoreLog desktop), where the speech model you configured transcribes it; summaries and the daily digest are made there too, with the model you chose. The phone never sends records or audio to Google or any other cloud. When the drawer can't be reached, records and audio stay on the phone and are sent later. Exposure is kept minimal (VAD records only speech) and you set the drawer address and token. Text is the record; audio is kept only briefly (as AAC) so you can replay a segment and hear what it was. It is deleted as soon as the segment is exported, or after 7 days otherwise, and is never included in exports.

## What it collects

Everything runs in one foreground service, and you toggle each source in settings.

| Source | How | Notes |
|---|---|---|
| **Voice** | Record → sent to the drawer for transcription | Silero VAD (neural) captures **only when someone speaks**. Keeps running with the screen off. Text only |
| **App usage** | UsageStatsManager | Which app, when |
| **On-screen content** | AccessibilityService | Text you see inside apps. **Sensitive apps (banking, payments) excluded** |
| **Location** | FusedLocationProvider | Every 5 min, low power, batched |
| **Notifications** | NotificationListenerService | Title and body of notifications you receive |
| **Calls / SMS** | CallLog · SMS · Contacts | Who and when (number → name) |
| **Calendar** | CalendarContract (+attendees) | All calendars on the phone, local included. Every registered event from a year back to a year ahead (title, time, place, notes, attendees' addresses) goes to the drawer as `calendar_event` lines, again when it changes and once as `deleted` when it goes; the drawer indexes them as events (the same event from CalDAV counts once). Calendars hidden in the calendar app and cancelled events are left out. Turning it off in the collectors sends what was sent as deleted once, so it leaves the drawer's index too |
| **Contacts** | ContactsContract | Everything the address book holds (name, nickname, emails, phones, company and title, addresses, websites, notes, birthdays and anniversaries, relations, messengers, groups), the same as from CardDAV; photos are left out. The whole list is sent to the drawer's `/ingest/contacts`, only when it changed. The librarian uses it to recognize people. Sent only over https or to a tailnet address. Turning it off in the collectors sends an empty list once, so it leaves the drawer's index too |

## Voice

The core of LoreLog, and the trickiest part. Google's live recognizer keeps grabbing audio focus (interrupting your media), which is wrong for always-on capture, so voice is **recording-based**.

- **Mic yielding** — Android lets one app hold the mic. When a voice-recorder or camera app comes to the front (needs usage-access permission) during a call (cellular or VoIP), or when another app actually starts recording, LoreLog releases the mic and reopens it when they are done. For apps not on the list, use "Yield mic 10 min" in the notification or on the main screen.
- **VAD capture** — `AudioRecord` reads raw PCM and **Silero VAD** (a small neural network, ONNX) keeps only the segments where a human is speaking and ignores keyboards, dishes and air-con. Batches shorter than 3 s of speech are dropped. Inference is ~1 ms per 32 ms frame, so the idle load is near zero. It never grabs audio focus, so your media keeps playing.
- **The drawer transcribes** — the phone packs speech into WAV and sends it to the drawer's `/transcribe`, which forwards it to the speech model you chose (`LIFE_ASR_URL` / `LIFE_ASR_MODEL`, or the settings screen on the desktop). There is no transcription engine and no API key on the phone. Nearby utterances are batched into ~4-minute chunks (a batch also closes after a 3-minute lull): fewer requests, and better accuracy than tiny clips.
- **Drawer unreachable: keep, then send** — when there is no drawer address or the drawer can't be reached, speech segments are kept as voice files in an app folder and sent once it's reachable, **automatically (a periodic worker) or manually ("밀린 음성 전사")**. Recording and keeping continue regardless.
- **Dictation** — "말하기" on the chat screen also uses the drawer's `/transcribe`. The recording is deleted right after it's transcribed.
- **Bluetooth earbud mic (optional)** — route recording input to a BT headset mic.
- **Import call recordings** — the OS blocks live call audio. Instead, point LoreLog at the folder where your dialer saves call recordings (via SAF) and it sends those files to the drawer for transcription (devices with a built-in recorder, e.g. Samsung). If the drawer can't be reached, they are retried on the next check.

## Conversations

- **Conversation grouping** — voice segments are grouped by time gaps into "conversations," and attendees are auto-linked from overlapping calls (contacts) and meetings (calendar attendees). Ambiguous ones go to a **review queue** that shows a transcript excerpt and a ▶ play button for each conversation, so you can tell who it was before tagging it (or skip it).
- **No summaries on the phone** — the phone sends a faithful copy to the drawer, and the drawer makes the daily digest and summaries with the model you chose (and can redo them as its schema evolves).

## Export / Delete

- **JSON** — the whole log via the share sheet.
- **Folder (incremental NDJSON)** — only new records appended to an app-private external folder. `Android/data/com.lorelog/files/exports/`. Reachable from a PC over USB/adb.
- **Send to the drawer (HTTP)** — POST new records to your drawer address every 15 minutes (`Authorization: Bearer <token>`, NDJSON). Turn on **"와이파이에 연결됐을 때만 보내기"** (send only on Wi-Fi) under settings → 서랍 연결 and the automatic sends of records and voice (recordings, call recordings) run only on an unmetered network (Wi-Fi); they wait on the phone until then. The phone's calendar is sent with the records and follows the same switch. It is off by default. "지금 전송" (send now), "밀린 음성 전사" (send pending voice) and dictation always send right away.
- Every line carries a `type` (voice/app/screen/location/notification/comm/calendar/calendar_event/conversation).
- **Delete all** — wipes every record after confirmation (irreversible).

## Storage · Security · Retention

- **Encryption** — the DB is encrypted with SQLCipher. The key is a random 32 bytes stored in `EncryptedSharedPreferences` protected by the Android Keystore (never leaves the device). The drawer token is stored the same way and never committed.
- **Retention** — only the last **30 days** on the device. Originals that have been exported and are older than 30 days are auto-deleted (conversations are kept). Long-term storage is the job of the knowledge graph you export to.
- **Voice audio** — the audio behind each transcribed segment is kept as AAC (~0.35MB/min) in `Android/data/com.lorelog/files/voice_kept/` and can be replayed from the timeline (**▶ 듣기**). Audio for segments included in a folder/HTTP export is deleted right away; audio never exported is deleted after **7 days**. Files still waiting for transcription also show up in the timeline as "전사 대기" and can be played. Capture keeps running during playback: the acoustic echo canceller (AEC) subtracts the played sound from the mic, anything still captured is labelled "재생 중", and a transcript that merely repeats the played text is dropped (mixed-in conversation is kept).
- **Notification / display** — the persistent notification comes back if dismissed (always kept). In settings you can hide notifications (hide the idle one, minimize the recording one) and enable a floating "monitoring" badge over other apps (overlay).
- Note: the app doesn't add its own encryption to exported files or to what it sends to the drawer (records and audio). Keep the drawer address on HTTPS or a tailnet, and manage folder access yourself.

## Install & Setup

1. Download the APK (`LoreLog-<version>.apk`) from [Releases](https://github.com/jwgung/LoreLog/releases/latest) and install it.
2. **Play Protect warning**: with this many permissions, sideloading triggers a warning. It can't be removed in code; to use the app, turn off Play Protect scanning in the Play Store (or "install without scanning"), and on Samsung turn off Auto Blocker.
3. In the app, tap **Start** and grant the prompted permissions (it starts with just the mic granted).
4. In the drawer → 수집·연동 설정 → **서랍 연결** (drawer connection), paste the desktop app's connection code into **연결 코드 붙여 넣기** (see [Getting started](#getting-started)), or enter your drawer address (e.g. `https://<vault>/ingest/lorelog`) and token, then save. A token is sent only over https or to a tailnet (`*.ts.net`, `100.x`) or local address. Transcription, dictation and the diary all use the same address and token. Without an address, collection keeps going; records and audio wait on the phone and are sent once it's connected.

**Special permissions** (you enable these yourself): usage access, accessibility (on-screen content), notification access, background location.

> 🔒 **New Android / Samsung note**: accessibility (on-screen content capture) may be hard-blocked for sideloaded apps by security hardening. You have to go through App info → ⋮ → "Allow restricted settings", and if it's still blocked, give up on-screen capture on that device and use the rest. The sources that don't depend on accessibility are the backbone.

### Desktop app (Windows · macOS · Linux)

The desktop app **holds the drawer**: on first start choose "keep it on this PC" and the app runs the server itself (records, mail/calendar notes, the index); the phone and other PCs send to it over Tailscale. It also runs the assistant screen and the PC collector in the tray. Details: [desktop/README.md](desktop/README.md).

Download from the same [Releases](https://github.com/jwgung/LoreLog/releases/latest) page as the Android APK (every release carries both; the phone app keeps updating from the APK as before):

| OS | File | Install |
|---|---|---|
| Windows 10/11 (x64) | `LoreLog_<version>_x64-setup.exe` | Run it. Installs for your user only (no admin). Windows 10 may need the WebView2 Runtime. |
| macOS 11+ (Apple silicon and Intel) | `LoreLog_<version>_universal.dmg` | Open it and drag LoreLog to Applications. |
| Linux (x64) | `LoreLog_<version>_amd64.AppImage` or `.deb` | AppImage: `chmod +x LoreLog_*.AppImage` then run it. deb: `sudo apt install ./LoreLog_*.deb`. |

`life-agent-<version>-windows-x64.zip` is the optional command-line collector for Windows (not needed with the desktop app).

**The apps are not signed with an OS certificate yet**, so the first start shows a warning:
- **Windows SmartScreen** "Windows protected your PC": click **More info → Run anyway**.
- **macOS** "LoreLog can't be opened" / "Apple could not verify…": try to open it once, then go to **System Settings → Privacy & Security**, scroll down and click **Open Anyway** next to LoreLog, and confirm. (macOS 15 no longer offers right-click → Open.)
- **Linux**: no warning; the AppImage just needs the execute bit (`chmod +x`).

**Updates**: the app checks for a new version when it starts. If there is one, the drawer shows "새 버전 X가 있어요 [설치]"; it installs only when you click, then restarts. Updates are signed and checked by the app. On Linux this works for the AppImage; with the deb, install the new deb.

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
  RecordingVoiceCollector  AudioRecord + Silero VAD -> WAV -> drawer transcription
  PendingVoice             keeps voice while the drawer is unreachable + pending transcription
  Dictation                chat-screen dictation (drawer /transcribe)
  AppUsage / Location / Comms / Calendar Collector
  CallRecordingImporter    transcribe a call-recording folder (SAF)
  SensitiveApps · CollectorPrefs · AudioPrefs
service/  ScreenContentService (accessibility) · NotificationService (notification listener)
data/     LogStore (SQLCipher) · LogRepository (StateFlow) · DbKey · event models
net/      ServerTranscriber · EndpointConfig · HttpSink · DiaryClient · UpdateChecker
work/     Push (+PushPrefs) · Export · Prune · Conversation · PendingVoice · CallRecording · DiaryReminder Worker
OverlayBadge · NotifPrefs · NotifReviveReceiver · BootReceiver
```

**Design note** — the phone captures what only exists because the phone is beside you, or what the phone gives for free locally (the sources above); things that live only in the cloud and need auth (email, etc.) are pulled on the drawer side. Anything that needs a model, like transcription and summaries, runs in the drawer with the model you chose.

## Legal note

In Korea, recording a conversation **you take part in** is legal, but recording others' conversations you are not part of is not. The rules for recording calls and conversations differ by country (some require all-party consent). Keep logs personal, and don't publish or distribute other people's data.

## Roadmap

- An on-screen-capture alternative for new devices (MediaProjection + on-device OCR) to bypass the accessibility block.
- Real-device verification of call/meeting auto-linking; battery-impact measurement and tuning.
- Candidate life sources: health (steps, sleep), photo/media metadata. Email belongs on the drawer side, not the phone.

## Disclaimer

A personal learning/experimentation project. Provided as-is, no warranty. Complying with local laws on collection and recording is the user's responsibility.
