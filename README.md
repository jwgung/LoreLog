# LifeLog

내 안드로이드 기기에서 **내** 하루를 텍스트로 기록하는 개인용(quantified-self) 앱.
voice-keyboard(PC 음성 입력·자막)의 모바일 상시판에 해당한다.

## 원칙 (경계)
- **본인 기기, 본인 데이터만.** 제3자 감시·은닉 수집이 아니다.
- 모든 수집은 OS 표준 권한과 **지울 수 없는 상시 알림**으로 드러난다. 숨기지 않는다.
- 데이터는 **기기 로컬에만** 저장(암호화 예정). 원본 오디오는 보관하지 않고 텍스트만 남긴다.
  밖으로 나가는 유일한 경로는 사용자가 직접 누르는 내보내기.
- 음성에 다른 사람과의 대화가 들어올 수 있다. 한국은 **본인이 참여한 대화** 녹음은 합법,
  본인이 끼지 않은 남들끼리의 대화 녹음은 불법. 기록은 개인용으로만 쓰고 공개·공유하지 않는다.

## 수집 범위 (계획)
- 음성 상시 → 텍스트 (화자 "나/상대" 라벨 예정)
- 앱 사용 (어떤 앱을 언제)
- 앱 내 화면 내용 (AccessibilityService, 은행·결제 등 민감 앱은 제외 목록)
- 위치, 받은 알림

## STT 엔진
- 안드로이드 내장 `SpeechRecognizer`(구글), 가능 기기는 온디바이스 오프라인 우선.
- 엔진은 인터페이스(`SttEngine`) 뒤에 두어 나중에 Vosk 등으로 교체 가능.

## 구조 (진행 중)
```
MainActivity (Compose)  — 시작/정지, 상태
LoggerService (FGS)     — 상시 알림 + 수집기(단계적으로 추가)
LifeLogApp              — 알림 채널
```

## 빌드
```bash
./gradlew :app:assembleDebug      # 디버그 APK
```
- 툴체인: AGP 9.4.1(내장 Kotlin), Gradle 9.8.0, compileSdk 37 / targetSdk 36 / minSdk 29, Jetpack Compose.
- `local.properties` 의 `sdk.dir` 로 Android SDK 경로 지정.

## 내보내기 / 삭제
- **내보내기(JSON)**: 모든 기록을 voice/apps/screen/locations/notifications 로 나눈
  JSON 파일로 내보내 공유(지식그래프 등 외부로 넣는 연결 고리).
- **전체 삭제**: 기기에 쌓인 모든 기록을 지운다(확인 후, 되돌릴 수 없음).

## 저장 보안
- DB 파일은 **SQLCipher 로 암호화**(lifelog_enc.db). 키는 랜덤 32바이트로,
  Android Keystore 로 보호되는 EncryptedSharedPreferences 에 보관(기기를 벗어나지 않음).

## 현재 상태 (1~7단계 구현 완료)
1. 스캐폴드 + 상시 알림 포그라운드 서비스
2. 음성 상시 수집(내장 SpeechRecognizer, 재시작 루프)
3. 앱 사용 수집(UsageStatsManager)
4. 앱 내 화면 내용 수집(AccessibilityService) + 민감 앱 제외 목록
5. 위치 수집(FusedLocationProvider)
6. 알림 수집(NotificationListenerService)
7. 내보내기(JSON) + 전체 삭제 + DB 암호화(SQLCipher)

SM-P200(Android 11) 실기기에서 음성·앱·화면·알림 수집, 통합 타임라인,
내보내기·삭제, 암호화 DB 동작을 확인. 위치는 GPS 없는 Wi-Fi 모델이라 측위 신호가
없어 실측은 GPS 폰/실외에서 확인 필요(등록·저장 경로는 확인됨).

## 남은 후속 과제
- 수집기별 on/off 설정 화면, 민감 앱 제외 목록 편집 UI(현재 기본 목록 + prefs).
- 화자 식별(내 목소리 등록 → 나/상대/모름 라벨), 배터리 영향 측정·튜닝.
- 시스템 UI(상태바) 화면 캡처 노이즈 필터, 위치 실측 검증.
