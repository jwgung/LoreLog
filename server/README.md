# life

하루의 기록(메일·일정·메신저·폰·음성)을 한 서버에 모아 하루를 정리하고, 사용자의 정정과 회고로 "이 사람이 어떤 사람인지"를 쌓는 개인 시스템의 서버 쪽.

- 청사진: 사서(수집·색인)와 비서(능동)와 금고(그 사람 것). 문서는 `docs/`에 둔다.
- 앱(폰)은 이 저장소 루트의 `app/`. 서버는 `server/`.
- 개인 데이터와 비밀값은 이 저장소에 두지 않는다(`.gitignore` 참고). 설정은 `~/.config/vault-sync/`.

## 구성
- `bin/life` 수집 소스 설정·실행(`life add`, `life sync`, `life status`)
- `bin/life-ingest` 폰·PC가 보내는 기록 수신, 일기·할 일·전사 HTTP (코드는 `life/ingest/`: `settings`, `store`, `asr`, `jobs`, `http`, `cli`)
- `bin/life-diary` 하루 정리 (코드는 `life/assistant/`. `life-ingest`는 이 모듈을 프로세스 없이 직접 부른다), 대화, 할 일, 메신저 허용
- `bin/mail-sync`, `bin/calendar-sync` 메일·일정 수집
- `deploy/` systemd 유닛과 환경 파일 예시

## 설치
`deploy/install.sh`가 `bin/*`을 `~/.local/bin/`에 심볼릭 링크로 연결한다.
