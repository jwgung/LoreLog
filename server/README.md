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
- `bin/life-librarian` 사서. 모든 날의 기록(메일 본문 포함·일정·폰·PC·사용자의 말)에 색인을 얹고 찾아 준다(`update`, `rebuild`, `search`, `stats`). 색인은 `<data>/librarian.sqlite`(0600)이고 기록에서 언제든 다시 만드는 사본이다. 일기 규칙을 그대로 따라 허용한 메신저와 일기가 본인 대화로 치는 음성만 색인하고, 비서의 정리와 대답은 색인하지 않는다. 한국어는 두 글자 단위로 색인해 조사가 붙은 말과 두 글자 단어도 찾는다. 같은 메일 스레드(References·In-Reply-To 헤더 기준)는 한 결과로 묶는다. 사용자가 지난 일을 묻거나 이야기하면 비서가 검색어를 골라 사서에게 묻고, 찾은 지난 기록을 사용자의 말 앞에 인용 자료로 붙인다(대화 기록에는 남기지 않는다. 사용자의 말은 결과 중 최대 2개, 색인 갱신에 실패하면 이전 색인에서 찾고 최신이 아닐 수 있다고 알린다. 정정할 때와 기분만 말할 때는 묻지 않는다, `life/assistant/recall.py`). `life sync`가 마지막에 `update`를 돌린다(`--dry-run`일 때는 돌리지 않는다). 코드는 `life/librarian/`
- `web/` 비어 있다. 나중에 설정·관리자 웹(로그인, 사용자 관리)이 들어갈 자리. 비서 화면은 폰 앱(`app/`)과 데스크톱 앱(`desktop/shell/ui/`)이 가지고, `life-ingest`는 API만 낸다.
- `deploy/` systemd 유닛과 환경 파일 예시

## 설치
`deploy/install.sh`가 `bin/*`을 `~/.local/bin/`에 심볼릭 링크로 연결한다.

## HTTPS 주소
`life-ingest`는 기본으로 tailnet 주소에 HTTP로 뜬다(`INGEST_CERT`·`INGEST_KEY`를 주면 직접 HTTPS). 보통은 tailscale serve로 앞에 HTTPS를 붙여 쓴다(한 번만, sudo 필요).

    sudo tailscale serve --bg --https=48765 http://<tailnet 주소>:8765

주소는 `https://<머신>.<tailnet>.ts.net:48765/`. 폰 앱은 기존 HTTP 주소를 그대로 쓸 수 있다. 되돌리기: `sudo tailscale serve --https=48765 off`.
