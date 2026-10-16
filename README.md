# LifeLog

내 안드로이드 기기에서 **내** 하루를 텍스트로 기록하는 개인용(quantified-self) 앱.
voice-keyboard(PC 음성 입력·자막)의 모바일 상시판에 해당한다. 폰은 늘 곁에 있는 센서이므로,
"폰에서 일어나는 일 + 폰이 곁에서 본 일상"을 모아 개인 지식그래프의 재료로 내보낸다.

## 원칙 (경계)
- **본인 기기, 본인 데이터만.** 제3자 감시·은닉 수집이 아니다.
- 모든 수집은 OS 표준 권한과 **지울 수 없는 상시 알림**으로 드러난다. 숨기지 않는다. 기록 중일 때만 수집한다.
- 데이터는 **기기 로컬에만** 저장(SQLCipher 암호화). 원본 오디오는 보관하지 않고 텍스트만 남긴다.
- 음성에 다른 사람과의 대화가 들어올 수 있다. 한국은 **본인이 참여한 대화** 녹음은 합법,
  끼지 않은 남들끼리의 대화 녹음은 불법. 기록은 개인용으로만 쓴다.

## 수집 범위
전부 같은 포그라운드 서비스에서 돌고, 하나의 시간순 타임라인으로 합쳐진다.

| 종류 | 출처 | 비고 |
|---|---|---|
| 음성 | 내장 SpeechRecognizer(구글) **또는** 녹음+클라우드(Groq Whisper) | 상시, 텍스트만 저장. 녹음+클라우드는 화면 꺼져도 안 끊기고 무음은 건너뜀 |
| 앱 사용 | UsageStatsManager | 어떤 앱을 언제 전면에 |
| 앱 내 화면 내용 | AccessibilityService | 화면 텍스트. **민감 앱(은행·결제·비밀번호)은 제외 목록** |
| 위치 | FusedLocationProvider | 간격·이동 거리마다 |
| 알림 | NotificationListenerService | 받은 알림 제목·본문(민감 앱·상시 알림 제외) |
| 통화·문자 | CallLog·SMS + 연락처 | 누구와 언제. 번호→이름 매칭 |
| 캘린더 | CalendarContract(+참석자) | 일정 시작 시각 자리에. 폰의 모든 캘린더(로컬 포함) |

## 대화 ("누구와 대화했나")
화자 식별(목소리 구분)은 원본 오디오 엔진이 필요해 보류 상태다. 대신:
- 음성 조각을 시간 간격(5분)으로 묶어 **대화** 단위로 만든다.
- 대화 시간대와 겹치는 **통화(연락처 이름)·회의(캘린더 참석자)**로 참석자를 **자동 연결**.
- 자동으로 못 채운 대화는 **검토 큐**("태그할 대화")에 모아 직접 입력.

## 요약
수집한 기록을 묶어 주기적으로 요약한다. 요약 엔진은 인터페이스(`Summarizer`) 뒤에 두어 바꿔 낀다.
- **클라우드 LLM(무료 티어)**: OpenAI 호환 방식. 기본값은 Gemini 2.0 Flash.
  앱 "요약 LLM" 칸의 **"Gemini 키 발급 (무료)"** 버튼으로 키를 받아 붙여넣으면 끝. URL·모델만 바꾸면 Groq 등도 가능.
- **온디바이스 Gemini Nano**: 지원 기기(Pixel 8+/Galaxy S24+·Android 14+)에서 데이터 안 내보내고 요약.
- 클라우드 키가 있으면 클라우드, 없으면 Nano 로 폴백. 매일 1회 워커 + 수동 "지금 요약".

## 내보내기 / 삭제
- **내보내기(JSON)**: 모든 기록을 한 JSON 으로 공유 시트(그때그때 아무 앱·위치).
- **폴더(증분 NDJSON)**: 지난 이후 새 기록만 앱 전용 외부 폴더에 append.
  `Android/data/com.lifelog/files/exports/lifelog-<날짜>.ndjson`. USB·adb 로 PC 에서 접근.
- **HTTP 엔드포인트**: 지정 URL 로 새 기록을 POST(Authorization: Bearer <토큰>, NDJSON).
  지식그래프 수집 엔드포인트로 직행. 폴더와 독립된 커서.
- 셋 다 줄마다 `type`(voice/app/screen/location/notification/comm/calendar/conversation/summary)이 붙는다.
- **전체 삭제**: 확인 후 모든 기록 삭제(되돌릴 수 없음).

## 보존 / 저장 보안
- **보존 정리**: 기기엔 최근 **30일**만. 이미 내보냈고 30일 지난 원본은 자동 삭제(요약·대화는 유지). 장기 보관은 내보낸 KG 쪽이 맡는다.
- **암호화**: DB 파일은 SQLCipher 로 암호화(`lifelog_enc.db`). 키는 랜덤 32바이트로,
  Android Keystore 로 보호되는 EncryptedSharedPreferences 에 보관(기기를 벗어나지 않음).
- 주의: 내보낸 파일·HTTP 본문·클라우드 요약 입력은 **평문**으로 밖으로 나간다. 민감하면 대상(폴더 접근·https 엔드포인트·학습 미사용 약관)을 관리한다.

## 설계 원칙
- **폰 vs 서버 역할 분담**: 폰은 곁에 있어야 생기거나 폰이 공짜 로컬로 주는 것(음성·화면·앱·위치·알림·통화/문자·연락처·캘린더)을 잡는다.
  클라우드에만 있고 인증이 필요한 것(이메일 등)은 **서버/KG 쪽**에서 당긴다. 캘린더는 폰 로컬 provider 가 모든 계정+로컬을 공짜로 주므로 폰이 더 완전하다.
- **엔진 교체 가능**: 음성 STT(`SttEngine`), 요약(`Summarizer`)을 인터페이스 뒤에 둔다.

## 구조
```
MainActivity (Compose)  — 시작/정지, 타임라인, 권한 안내, 설정(HTTP·요약 LLM), 검토 큐, 내보내기/삭제
LoggerService (FGS)     — 상시 알림 + 수집기 묶음
 ├─ VoiceCollector / AppUsageCollector / LocationCollector / CommsCollector / CalendarCollector
service/ ScreenContentService(접근성) · NotificationService(알림 리스너)
data/   LogStore(SQLCipher) · LogRepository(StateFlow) · DbKey · 각 이벤트 모델
stt/    SttEngine · AndroidSpeechEngine (음성→텍스트)
summary/ Summarizer · GeminiNanoSummarizer · CloudSummarizer (요약)
net/    EndpointConfig(HTTP) · HttpSink · LlmConfig(요약 LLM) · SttConfig · GroqTranscriber(음성 전사)
work/   ExportWorker(폴더+HTTP) · SummaryWorker · PruneWorker · ConversationWorker
```

## 빌드
```bash
./gradlew :app:assembleDebug      # 디버그 APK
```
- 툴체인: AGP 9.4.1(내장 Kotlin), Gradle 9.8.0, compileSdk 37 / targetSdk 36 / minSdk 29, Jetpack Compose.
- 주요 의존성: play-services-location, SQLCipher(net.zetetic), security-crypto, WorkManager, ML Kit genai-summarization.
- `local.properties` 의 `sdk.dir` 로 Android SDK 경로 지정.

## 권한 (전부 사용자가 직접 켬)
- 마이크(`RECORD_AUDIO`) + 포그라운드 서비스(마이크·위치 타입), `POST_NOTIFICATIONS`
- 사용 정보 접근(`PACKAGE_USAGE_STATS`, 특수), 접근성 서비스(특수), 알림 접근(특수)
- 위치(`ACCESS_FINE/COARSE/BACKGROUND_LOCATION`)
- 통화·문자·연락처(`READ_CALL_LOG`/`READ_SMS`/`READ_CONTACTS`), 캘린더(`READ_CALENDAR`)
- `INTERNET`(HTTP 내보내기·클라우드 요약), 앱 이름 해석용 `QUERY_ALL_PACKAGES`

## 현재 상태 / 검증
실기기 SM-P200(Android 11)에서 확인:
- 음성·앱·화면·알림 수집, 통합 타임라인, 상시 알림, 내보내기(JSON·폴더 NDJSON·HTTP), 보존 정리, 암호화 DB, 클라우드 요약(모의 서버).
- 알림: 지워도 복귀(상시 유지), '알림 숨기기'(멈춤 알림 숨김+기록 알림 최소화), 백그라운드 '감시중' 플로팅 배지(오버레이).
- 대화 묶기·검토 큐·수동 태그, 캘린더 일정(참석자 포함).
- **미검증(기기 한계)**: 위치 실측(Wi-Fi 전용 모델이라 측위 없음), 통화·문자 실제 캡처(통신 없음),
  온디바이스 Gemini Nano(미지원 기기). 코드·등록·저장 경로는 다른 수집기와 동일 패턴으로 확인.

## 남은 후속 과제
- 화자 식별(내 목소리 vs 상대): 원본 오디오 엔진(Vosk/Picovoice + 음성 임베딩)으로 교체해야 가능.
- 통화·회의 자동 연결의 실기기 검증, 배터리 영향 측정·튜닝.
- 생활 소스 추가 후보: 헬스(걸음·수면), 사진·미디어 메타. 이메일은 폰이 아니라 **서버/KG** 쪽에서 당기는 게 맞다.
