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
- `bin/mail-files` 보낸·받은 메일의 첨부 원본을 그대로 보관(`raw/files/<sha256>` + `raw/mailfile/<날짜>.ndjson`, 줄마다 `direction: sent|received`). 변환·요약 없이, 상대 주소는 복사하지 않는다(메일 노트에 있다). 읽기 전용, `--dry-run`은 크기만 센다. `life sync`가 mail-sync 다음에 실행한다(폴더당 200통씩). 코드는 `life/collect/`
- `web/` 데스크톱·폰 브라우저용 화면(대화, 할 일, 서랍, 말하기). `life-ingest`가 `/`에서 그대로 내준다. 토큰은 처음 한 번 입력하면 그 브라우저에 기억한다.
- `deploy/` systemd 유닛과 환경 파일 예시

## 설치
`deploy/install.sh`가 `bin/*`을 `~/.local/bin/`에 심볼릭 링크로 연결한다.

## 웹 화면 주소
`life-ingest`는 tailnet 주소에 HTTP로만 뜬다. 브라우저의 마이크와 홈 화면 설치는 HTTPS에서만 되므로 tailscale serve로 앞에 HTTPS를 붙인다(한 번만, sudo 필요).

    sudo tailscale serve --bg --https=48765 http://<tailnet 주소>:8765

주소는 `https://<머신>.<tailnet>.ts.net:48765/`. 폰 앱은 기존 HTTP 주소를 그대로 쓸 수 있다. 되돌리기: `sudo tailscale serve --https=48765 off`.
