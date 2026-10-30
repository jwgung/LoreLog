# life (서버)

하루의 기록(메일·일정·메신저·폰·음성·PC)을 한 곳에 모아 하루를 정리하고, 사용자의 정정과 회고로 "이 사람이 어떤 사람인지"를 쌓는 개인 시스템의 서버 쪽.

- 청사진: 사서(수집·색인)와 비서(능동)와 금고(그 사람 것). 문서는 `docs/`에 둔다.
- 앱(폰)은 이 저장소 루트의 `app/`, 데스크톱 앱과 PC 수집기는 `desktop/`, 서버는 `server/`.
- 개인 데이터와 비밀값은 이 저장소에 두지 않는다(`.gitignore` 참고). 설정은 OS 설정 위치의 `vault-sync/`(리눅스 `~/.config/vault-sync/`, `LIFE_CONF_DIR`로 옮길 수 있다).

## 구성
Rust 라이브러리 crate 하나(`life-server`)와 그 위의 실행 파일들이다. 같은 라이브러리를 데스크톱 앱도 쓸 수 있다(서버로도, 데스크톱 앱 안에서도 돈다. `docs/adr/0001-server-in-rust.md`).

| 실행 파일 | 하는 일 | 코드 |
|---|---|---|
| `life` | 수집 소스 설정·실행(`life add mail`, `life add calendar`, `life sync`, `life status`). `sync`는 아래 수집기를 차례로 돌리고 마지막에 사서 색인을 갱신한다(`--dry-run`일 때는 갱신하지 않는다). 다른 실행 파일을 띄우지 않고 같은 프로세스에서 `collect::sync`의 함수를 부른다 | `src/bin/life.rs`, `src/collect/sync.rs`, `src/collect/setup.rs` |
| `life-ingest` | 폰·PC가 보내는 기록 수신, 일기·대화·할 일·받아쓰기 HTTP API | `src/http.rs`, `src/store.rs`, `src/filestore.rs`, `src/jobs.rs` |
| `life-diary` | 하루 정리, 대화, 할 일, 메신저 허용을 명령줄로 | `src/assistant/` |
| `life-librarian` | 사서. 모든 날의 기록에 색인을 얹고 찾아 준다(`update`, `rebuild`, `search`, `stats`) | `src/librarian/`, `src/assistant/recall.rs` |
| `mail-sync`, `calendar-sync` | 메일·일정을 금고의 `Mail/`, `Events/` 노트로(서버에는 읽기만) | `src/collect/` |
| `mail-files` | 보낸·받은 메일의 첨부 원본을 그대로 보관(`raw/files/<앞 2자>/<sha256>` + `raw/mailfile/<날짜>.ndjson`, 줄마다 `direction: sent|received`). 변환·요약 없이, 상대 주소는 복사하지 않는다 | `src/collect/mailfiles.rs` |

- `web/` 비어 있다. 나중에 설정·관리자 웹(로그인, 사용자 관리)이 들어갈 자리. 비서 화면은 폰 앱과 데스크톱 앱(`desktop/shell/ui/`)이 가지고, `life-ingest`는 API만 낸다.
- `deploy/` systemd 유닛, 환경 파일 예시, 설치 스크립트.

### 사서
모든 날의 기록(메일 본문 포함·일정·폰·PC·사용자의 말)에 SQLite FTS5 색인을 얹는다. 색인은 `<data>/librarian.sqlite`(0600)이고 기록에서 언제든 다시 만드는 사본이다.
- 일기 규칙을 그대로 따른다. 허용한 메신저와 일기가 본인 대화로 치는 음성만 색인하고, 비서의 정리와 대답은 색인하지 않는다.
- 한국어는 두 글자 단위로 색인해 조사가 붙은 말과 두 글자 단어도 찾는다. 분해된 한글(macOS 첨부 이름)과 전각 글자도 찾는다.
- 같은 메일 스레드(References·In-Reply-To 헤더 기준)는 한 결과로 묶는다.
- 사용자가 지난 일을 묻거나 이야기하면 비서가 검색어를 골라 사서에게 묻고, 찾은 기록(오늘 메일 본문 포함, 이미 대화에 들어간 줄은 뺀다)을 사용자의 말 앞에 인용 자료로 붙인다. 대화 기록에는 남기지 않는다. 사용자의 말은 결과 중 최대 2개다. 색인 갱신에 실패하면 이전 색인에서 찾고 최신이 아닐 수 있다고 알린다. 정정할 때와 기분만 말할 때는 묻지 않는다.

## 설치
Rust 툴체인(저장소 루트의 `rust-toolchain.toml`)이 필요하다. `deploy/install.sh`가 release로 빌드해 실행 파일을 `~/.local/lib/life/bin/`에 복사하고 `~/.local/bin/`에서 링크로 가리키게 하며, `deploy/systemd/`의 유닛을 `~/.config/systemd/user/`에 둔다(이전 실행 파일과 바뀌는 유닛은 `~/.local/share/life-install-backup/`에 백업). 복사해서 두므로, 나중에 어느 체크아웃에서 빌드해도 돌고 있는 서비스는 바뀌지 않는다. 바꾸려면 install.sh를 다시 돌린다.

    server/deploy/install.sh
    systemctl --user enable --now life-ingest life-sync.timer   # 처음 한 번
    systemctl --user restart life-ingest                         # 다시 설치한 뒤

## 어느 OS에서든

라이브러리는 어느 OS에서든 그대로 돌아야 한다. 기본 위치는 `config::OsDirs`로 OS에서 고르고(리눅스는 예전 그대로 `~/.config/vault-sync`, `~/.local/share/life`, `~/.local/share/vault-sync`. `XDG_CONFIG_HOME`·`XDG_DATA_HOME`을 두면 그쪽을 따른다), 한 OS의 경로를 코드에 적지 않는다. 1시간마다의 동기화 같은 예약 실행은 라이브러리가 아니라 그것을 띄우는 쪽의 일이다(지금은 `deploy/systemd/life-sync.timer`, 나중에는 데스크톱 앱). 라이브러리 안의 스레드는 요청 하나에 딸린 일회성 작업(하루 정리 만들기, 할 일 뽑기)뿐이다. 동기화 작업(메일, 첨부, 일정, 사서 갱신)은 `collect::sync`의 함수이고, 출력하지 않고 줄 단위로 알린다. 명령줄 도구는 그 줄을 stdout·stderr로 옮기고, 데스크톱 앱은 같은 함수를 직접 부른다.

## 테스트

    server/check.sh

테스트, clippy, rustfmt, 그리고 Windows 대상 컴파일(`cargo xwin check`)을 차례로 본다. 이 라이브러리는 데스크톱 앱 안에서도 돌기 때문에(`docs/adr/0002-vault-on-the-users-device.md`) Windows에서 컴파일되지 않으면 실패로 친다. release 빌드는 하지 않으므로 돌고 있는 서비스의 실행 파일을 건드리지 않는다. 테스트만 돌리려면 `cargo test -p life-server`.

가짜 모델과 임시 폴더로 실제 HTTP 서버를 띄워 확인한다. 실제 메일·일정 서버에 닿는 부분(IMAP, CalDAV)은 테스트하지 않는다.


## HTTPS 주소
`life-ingest`는 기본으로 `127.0.0.1`에 HTTP로 뜬다(`INGEST_CERT`·`INGEST_KEY`에 PEM 파일을 주면 직접 HTTPS). 폰·PC에서 받으려면 `ingest.env`의 `INGEST_BIND`를 이 머신의 tailnet 주소로 바꾸고, 보통은 tailscale serve로 앞에 HTTPS를 붙여 쓴다(한 번만, sudo 필요).

    sudo tailscale serve --bg --https=48765 http://<tailnet 주소>:8765

주소는 `https://<머신>.<tailnet>.ts.net:48765/`. 폰 앱은 기존 HTTP 주소를 그대로 쓸 수 있다. 되돌리기: `sudo tailscale serve --https=48765 off`.
