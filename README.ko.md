<div align="center">

# LoreLog

**내 안드로이드에서 내 하루를 텍스트로 모아, 나에 관한 개인 지식그래프의 재료로 내보내는 라이프로그 앱.**

개인용 · 로컬 암호화 저장 · 클라우드 또는 온디바이스 전사 · 클라우드 요약(본인 키)

[**APK 다운로드**](https://github.com/jwgung/LoreLog/releases/latest) · [릴리스](https://github.com/jwgung/LoreLog/releases)

[English](README.md) · 한국어

</div>

---

폰은 늘 곁에 있는 센서다. LoreLog는 "폰에서 일어나는 일 + 폰이 곁에서 본 일상"(음성·앱 사용·화면 내용·위치·알림·통화/문자·일정)을 하나의 시간순 타임라인으로 모은다. 모은 기록은 기기에만 암호화해 두고, 원할 때 JSON·파일·HTTP로 내보내 개인 지식그래프(*your lore*)에 붙인다.

> ⚠️ **개인용 도구다.** 본인 기기에서 본인 데이터만 수집하도록 만들어졌다. 다른 사람을 몰래 감시하는 용도가 아니며, 그렇게 쓰면 안 된다.

## 원칙

- **본인 기기, 본인 데이터만.** 제3자 감시·은닉 수집이 아니다.
- **드러낸다.** 모든 수집은 OS 표준 권한 동의와 **상시 알림**으로 보인다. 숨기지 않는다. 기록 중일 때만 수집한다.
- **로컬 암호화 저장.** 기록은 기기 안 SQLCipher 암호화 DB에, API 키는 Keystore 기반 저장소에 둔다. 이 앱이 클라우드에 저장하는 것은 없다.
- **처리는 기본이 클라우드지만, 전사는 완전 온디바이스로 돌릴 수 있다.** 기본값은 음성 전사·요약이 본인 키로 클라우드에서 돌아 말소리·텍스트가 기기 밖으로 나간다. 전사는 **온디바이스 whisper.cpp**로 바꿀 수 있고(모델을 한 번 받으면 그 뒤론 아무것도 밖으로 안 나감), 요약은 지원 기기면 온디바이스 **Gemini Nano**로 폴백한다. 노출은 최소화하고(VAD로 말할 때만 전송), 엔드포인트·키는 전부 사용자가 통제한다. 본체는 텍스트고, 오디오는 "무슨 소리였는지" 다시 들어볼 수 있게 AAC로 잠시만 보관한다(내보낸 뒤 즉시, 내보내지 않으면 7일 뒤 삭제. 내보내기에는 포함되지 않는다).

## 수집하는 것

전부 같은 포그라운드 서비스에서 돌고, 설정에서 항목별로 켜고 끈다.

| 종류 | 출처 | 비고 |
|---|---|---|
| **음성** | 녹음 → 전사(Gemini / 온디바이스 whisper) | Silero VAD(신경망)로 **말할 때만** 캡처. 화면 꺼져도 유지. 텍스트만 저장 |
| **앱 사용** | UsageStatsManager | 어떤 앱을 언제 썼나 |
| **화면 내용** | AccessibilityService | 앱 안에서 본 텍스트. 은행·결제 등 **민감 앱 제외** |
| **위치** | FusedLocationProvider | 5분 간격·저전력·배치 |
| **알림** | NotificationListenerService | 받은 알림 제목·본문 |
| **통화·문자** | CallLog · SMS · 연락처 | 누구와 언제(번호→이름 매칭) |
| **캘린더** | CalendarContract (+참석자) | 폰의 모든 캘린더(로컬 포함) |

## 음성

LoreLog의 핵심이자 가장 까다로운 부분. 구글 실시간 인식기는 오디오 포커스를 계속 뺏어(미디어 끊김) 상시 수집에 안 맞아, **녹음 기반**으로 설계했다.

- **마이크 양보** — 안드로이드는 마이크를 한 앱만 쓰게 한다. 음성 녹음·카메라 앱이 맨 앞에 오거나(사용 정보 접근 권한 필요) 다른 앱이 실제로 녹음을 시작하면 마이크를 놓고, 끝나면 자동으로 다시 연다. 목록에 없는 앱은 알림의 "마이크 10분 양보"나 화면 버튼으로 수동 양보한다.
- **VAD 캡처** — `AudioRecord`로 원본 PCM을 받아 **Silero VAD**(작은 신경망, ONNX)로 사람 말이 있을 때만 구간을 모은다. 키보드·식기·에어컨 소리에는 깨어나지 않고, 말소리 합이 3초 미만인 배치는 버린다. 32ms 프레임당 추론 1ms 안쪽이라 부하가 거의 없다. 오디오 포커스를 뺏지 않아 미디어가 안 끊긴다.
- **전사 — 엔진을 고른다**(설정 → "음성 전사 방식"):
  - **클라우드 · Gemini** — 설정의 "Gemini API 키"를 사용(요약과 공용). 무료 티어(Flash/Flash-Lite 무료, 모델 드롭다운에 무료/유료·종료·느림 표시). (소리가 구글로 감)
  - **온디바이스 · whisper.cpp** — 폰 안에서 전사, 아무것도 밖으로 안 나감. 정확도를 모델로 선택: tiny(~75MB) / base(~142MB, 추천) / small(~466MB), 모델은 1회 다운로드. arm64 기기만. 클라우드로 바꾸면 모델은 RAM에서 해제된다.

  클라우드가 더 정확하고, 온디바이스는 정확도를 조금 내주는 대신 완전한 프라이버시와 키 불필요를 얻는다. 가까운 발화는 ~4분으로 묶어 전사한다(대화가 3분 넘게 끊기면 그때까지 모인 것으로 마감) — 요청 수가 줄고(무료 티어는 모델당 하루 500회, 4분 배치면 연속 대화도 시간당 15회) 짧은 토막보다 정확도도 낫다.
- **준비 안 됐으면 보관 후 전사** — 고른 엔진이 준비 안 됐으면(키 없음, 또는 온디바이스 모델 미다운로드) 말 구간을 음성파일로 앱 폴더에 쌓아 두고, 준비되면 **자동(주기 워커) 또는 수동("밀린 음성 전사")**으로 그 파일만 변환한다. 녹음·보관은 상관없이 이어진다.
- **블루투스 이어폰 마이크(선택)** — 녹음 입력을 BT 헤드셋 마이크로 돌린다.
- **통화 녹음 가져오기** — 라이브 통화 음성은 OS가 막는다. 대신 전화앱이 저장한 통화 녹음 폴더를 지정(SAF)하면 그 파일을 전사한다(삼성 등 내장 녹음 기종).

## 대화 · 요약

- **대화 묶기** — 음성 조각을 시간 간격으로 묶어 "대화" 단위로 만들고, 겹치는 통화(연락처)·회의(캘린더 참석자)로 참석자를 자동 연결한다. 애매하면 **검토 큐**("태그할 대화")에 올린다. 큐에는 그 대화의 전사 내용 일부와 ▶ 듣기 버튼이 같이 보여 누구였는지 확인한 뒤 적거나 "넘기기"로 닫을 수 있다.
- **요약(선택, 기본 꺼짐)** — 폰에서의 요약은 선택이다. 지식그래프엔 원본을 충실히 넘기고 요약은 KG 쪽에서(스키마가 바뀌면 다시 요약) 하는 게 낫기 때문. 켜면 앱에서 읽을 요약을 주기적으로 쌓고 내보내기에도 요약을 포함한다. 엔진은 Gemini Flash(기본 3.5 Flash-Lite, 한도가 차면 다른 Flash로 자동 전환 — 고르지 않는다), 지원 기기는 온디바이스 **Gemini Nano** 폴백.

## 내보내기 / 삭제

- **JSON** — 전체 기록을 공유 시트로.
- **폴더(증분 NDJSON)** — 새 기록만 앱 전용 외부 폴더에 append. `Android/data/com.lorelog/files/exports/`. USB·adb로 PC에서 접근.
- **HTTP 엔드포인트** — 지정 URL로 새 기록을 POST(`Authorization: Bearer <토큰>`, NDJSON). 지식그래프 수집기로 직행.
- 모든 줄에 `type`(voice/app/screen/location/notification/comm/calendar/conversation/summary)이 붙는다. `summary` 줄은 요약을 켰을 때만 포함된다.
- **전체 삭제** — 확인 후 모든 기록 삭제(되돌릴 수 없음).

## 저장 · 보안 · 보존

- **암호화** — DB는 SQLCipher로 암호화. 키는 랜덤 32바이트를 Android Keystore가 보호하는 `EncryptedSharedPreferences`에 보관(기기를 벗어나지 않음). API 키도 같은 방식으로 암호화 저장하며 커밋하지 않는다.
- **보존** — 기기엔 최근 **30일**만. 이미 내보냈고 30일 지난 원본은 자동 삭제(요약·대화는 유지). 장기 보관은 내보낸 지식그래프 쪽이 맡는다.
- **음성 오디오** — 전사된 조각의 오디오는 `Android/data/com.lorelog/files/voice_kept/`에 AAC(약 0.35MB/분)로 보관되고 타임라인에서 **▶ 듣기**로 재생할 수 있다. 폴더/HTTP 내보내기에 포함된 조각의 오디오는 그 즉시, 내보내지 않은 것은 **7일** 뒤 지운다. 전사 대기 중인 파일도 타임라인에 "전사 대기"로 보이고 바로 들을 수 있다. 재생 중에도 캡처는 계속된다. 에코 캔슬러(AEC)로 재생 소리를 마이크 입력에서 빼고, 그래도 잡힌 묶음은 "재생 중" 라벨이 붙으며, 전사 결과가 재생한 텍스트와 같으면 버린다(대화가 섞였으면 남긴다).
- **알림/표시** — 상시 알림은 지워도 복귀(항상 유지). 설정에서 알림 숨기기(멈춤 알림 숨김 + 기록 알림 최소화)와 백그라운드 "감시중" 플로팅 배지(오버레이)를 켤 수 있다.
- 주의: 내보낸 파일·HTTP 본문·클라우드 전사/요약 입력은 **평문**으로 밖으로 나간다. 민감하면 대상(폴더 접근·HTTPS 엔드포인트·학습 미사용 약관)을 직접 관리한다.

## 설치 & 설정

1. [릴리스](https://github.com/jwgung/LoreLog/releases/latest)에서 APK(`LoreLog-<버전>.apk`)를 받아 설치한다.
2. **Play 프로텍트 경고**: 권한이 많아 사이드로드 시 경고가 뜬다. 코드로 없앨 수 없으니, 쓰려면 Play 스토어 → Play 프로텍트 검사 끄기(또는 "검사 없이 설치"), 삼성은 Auto Blocker 끄기.
3. 앱에서 **기록 시작** → 뜨는 권한을 허용한다(마이크만 허용해도 시작된다).
4. 설정의 **Gemini API 키** 한 칸에 키를 넣는다(전사·요약 공용, 입력 즉시 저장, "키 확인"으로 검증). 전사는 온디바이스 whisper를 골라 키 대신 모델을 받을 수도 있다. 키(또는 받은 모델)가 없으면 그 기능만 쉬고 나머지는 동작한다.

**특수 권한**(사용자가 직접 켜야 함): 사용 정보 접근, 접근성(화면 내용), 알림 접근, 백그라운드 위치.

> 🔒 **신형 안드로이드/삼성 주의**: 접근성(화면 내용 수집)은 보안 강화로 사이드로드 앱에 하드 차단될 수 있다. 앱 정보 → ⋮ → "제한된 설정 허용"을 거쳐야 하고, 그래도 막히면 그 기기에선 화면 수집만 포기하고 나머지로 쓴다. 접근성에 의존하지 않는 소스가 뼈대다.

## 빌드

```bash
./gradlew :app:assembleDebug      # 디버그 APK
./gradlew :app:assembleRelease    # 릴리스 APK (서명 설정 필요)
```

- 툴체인: AGP 9.4.1(내장 Kotlin), Gradle 9.8.0, compileSdk 37 / targetSdk 36 / minSdk 29, Jetpack Compose.
- 주요 의존성: SQLCipher(net.zetetic), security-crypto, WorkManager, play-services-location, ML Kit genai-summarization, android-vad(Silero/ONNX), whisper-android(dev.ffmpegkit-maintained, NDK 불필요).
- `local.properties`의 `sdk.dir`로 Android SDK 경로 지정. 릴리스 서명 키스토어·비밀번호는 저장소 밖에 둔다.

## 구조

```
MainActivity (Compose)   시작/정지 · 타임라인 · 권한 안내 · 설정 · 검토 큐 · 내보내기/삭제
LoggerService (FGS)      상시 알림 + 수집기 묶음 (권한 있는 타입만 포그라운드 시작)
collect/
  RecordingVoiceCollector  AudioRecord + Silero VAD -> WAV -> 전사
  PendingVoice             오프라인 음성 보관 + 밀린 전사
  AppUsage / Location / Comms / Calendar Collector
  CallRecordingImporter    통화 녹음 폴더(SAF) 전사
  SensitiveApps · CollectorPrefs · AudioPrefs
service/  ScreenContentService(접근성) · NotificationService(알림 리스너)
data/     LogStore(SQLCipher) · LogRepository(StateFlow) · DbKey · 이벤트 모델
net/      GeminiTranscriber · LlmConfig · ModelCatalog · TranscribeConfig · WhisperModels · EndpointConfig · HttpSink
stt/      WhisperEngine(whisper.cpp) · ModelDownloader
summary/  Summarizer · CloudSummarizer · GeminiNanoSummarizer
work/     Export · Summary · Prune · Conversation · PendingVoice · CallRecording Worker
OverlayBadge · NotifPrefs · NotifReviveReceiver · BootReceiver
```

**설계 메모** — 폰은 곁에 있어야 생기거나 폰이 공짜 로컬로 주는 것을 잡고(위 소스들), 클라우드에만 있고 인증이 필요한 것(이메일 등)은 서버/지식그래프 쪽에서 당긴다. 음성 STT·요약은 인터페이스 뒤에 두어 엔진을 바꿔 낄 수 있다.

## 법적 주의

한국은 **본인이 참여한 대화** 녹음은 합법이지만, 끼지 않은 남들끼리의 대화 녹음은 불법이다. 통화·대화 녹음의 합법 요건은 나라마다 다르다(일부는 당사자 전원 동의 필요). 기록은 개인용으로만 쓰고, 다른 사람의 데이터를 공개·배포하지 않는다.

## 로드맵

- 신형 기기용 화면 수집 대안(MediaProjection + 온디바이스 OCR) — 접근성 차단 우회.
- 통화·회의 자동 연결의 실기기 검증, 배터리 영향 측정·튜닝.
- 생활 소스 후보: 헬스(걸음·수면), 사진·미디어 메타. 이메일은 폰이 아니라 서버/KG 쪽에서 당기는 게 맞다.

## 면책

개인 학습·실험용 프로젝트다. 무보증 제공. 수집·녹음에 관한 현지 법률 준수 책임은 사용자에게 있다.
