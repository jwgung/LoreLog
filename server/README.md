# life (서버)

하루의 기록(메일·일정·메신저·폰·음성·PC)을 한 곳에 모아 하루를 정리하고, 사용자의 정정과 회고로 "이 사람이 어떤 사람인지"를 쌓는 개인 시스템의 서버 쪽.

- 청사진: 사서(수집·색인)와 비서(능동)와 서랍(그 사람 것). 문서는 `docs/`에 둔다.
- 앱(폰)은 이 저장소 루트의 `app/`, 데스크톱 앱과 PC 수집기는 `desktop/`, 서버는 `server/`.
- 개인 데이터와 비밀값은 이 저장소에 두지 않는다(`.gitignore` 참고). 설정은 OS 설정 위치의 `vault-sync/`(리눅스 `~/.config/vault-sync/`, `LIFE_CONF_DIR`로 옮길 수 있다).

## 구성
Rust 라이브러리 crate 하나(`life-server`)와 그 위의 실행 파일들이다. 같은 라이브러리를 데스크톱 앱도 쓸 수 있다(서버로도, 데스크톱 앱 안에서도 돈다. `docs/adr/0001-server-in-rust.md`).

| 실행 파일 | 하는 일 | 코드 |
|---|---|---|
| `life` | 수집 소스 설정·실행(`life add mail`, `life add calendar`, `life add contacts`, `life sync`, `life status`). `contacts`는 CardDAV 주소록(가진 것 다, 사진만 뺌, `src/collect/carddav.rs`). `sync`는 아래 수집기를 차례로 돌리고 마지막에 사서 색인을 갱신한다(`--dry-run`일 때는 갱신하지 않는다). 다른 실행 파일을 띄우지 않고 같은 프로세스에서 `collect::sync`의 함수를 부른다 | `src/bin/life.rs`, `src/collect/sync.rs`, `src/collect/setup.rs` |
| `life-ingest` | 폰·PC가 보내는 기록 수신, 일기·대화·할 일·받아쓰기 HTTP API | `src/http.rs`, `src/store.rs`, `src/filestore.rs`, `src/jobs.rs` |
| `life-diary` | 하루 정리, 대화, 할 일, 메신저 허용을 명령줄로 | `src/assistant/` |
| `life-librarian` | 사서. 모든 날의 기록에 색인을 얹고 찾아 준다(`update`, `rebuild`, `search`, `stats`). 사람 색인(메일 머리말·일정 참석자)으로 사람으로도 찾는다(#22). `embed`는 벡터가 없는 문서를 임베딩한다. `eval FILE`은 질문과 나와야 할 기록의 짝으로 재현율@8을 잰다(말 그대로, 비서가 고른 검색어. 임베딩이 설정돼 있으면 비서 쪽은 임베딩으로 찾고, `--no-embed`로 끈다). 평가셋은 실제 이름·제목이 들어 있어 저장소 밖(`<data>/eval/`)에 둔다 | `src/librarian/`, `src/librarian/eval.rs`, `src/assistant/recall.rs` |
| `mail-sync`, `calendar-sync` | 메일·일정을 서랍의 `Mail/`, `Events/` 노트로(서버에는 읽기만) | `src/collect/sync.rs`, `src/collect/` |
| `mail-files` | 보낸·받은 메일의 첨부 원본을 그대로 보관(`raw/files/<앞 2자>/<sha256>` + `raw/mailfile/<날짜>.ndjson`, 줄마다 `direction: sent|received`). 변환·요약 없이, 상대 주소는 복사하지 않는다 | `src/collect/sync.rs`, `src/collect/mailfiles.rs` |

- `web/` 비어 있다. 나중에 설정·관리자 웹(로그인, 사용자 관리)이 들어갈 자리. 비서 화면은 폰 앱과 데스크톱 앱(`desktop/shell/ui/`)이 가지고, `life-ingest`는 API만 낸다.
- `deploy/` systemd 유닛, 환경 파일 예시, 설치 스크립트.

### 사서
모든 날의 기록(메일 본문 포함·일정·폰·PC·사용자의 말)에 SQLite FTS5 색인을 얹는다. 색인은 `<data>/librarian.sqlite`(0600)이고 기록에서 언제든 다시 만드는 사본이다.
- 일기 규칙을 그대로 따른다. 허용한 메신저와 일기가 본인 대화로 치는 음성만 색인하고, 비서의 정리와 대답은 색인하지 않는다.
- 한국어는 두 글자 단위로 색인해 조사가 붙은 말과 두 글자 단어도 찾는다. 분해된 한글(macOS 첨부 이름)과 전각 글자도 찾는다.
- 같은 메일 스레드(References·In-Reply-To 헤더 기준)는 한 결과로 묶는다.
- 사용자가 지난 일을 묻거나 이야기하면 비서가 검색어를 골라 사서에게 묻고, 찾은 기록(오늘 메일 본문 포함, 이미 대화에 들어간 줄은 뺀다)을 사용자의 말 앞에 인용 자료로 붙인다. 대화 기록에는 남기지 않는다. 사용자의 말은 결과 중 최대 2개다. 색인 갱신에 실패하면 이전 색인에서 찾고 최신이 아닐 수 있다고 알린다. 정정할 때와 기분만 말할 때는 묻지 않는다.
- 임베딩(ADR 0007, 기본은 꺼짐): `LIFE_EMBED_MODEL`(ingest.env 또는 환경변수, 예 `Qwen/Qwen3-Embedding-4B`)을 두면 비서가 질문에 맞는 건을 낱말 대신 임베딩으로 찾는다. 서버는 대화 모델과 같은 `LIFE_LLM_URL`(사용자 자신의 사설망 서버, OpenAI 호환 `POST /embeddings`)이고 다른 곳으로는 보내지 않는다. 메일·첨부·일정의 제목과 본문 앞 1500자를 벡터로 만들어 같은 색인 파일(0600)에 두며, 색인처럼 지우면 사라지고 기록이 지워지면 다음 갱신에서 그 벡터도 지운다. 같은 글이면 다시 만들지 않으므로 색인을 다시 만들어도(스키마 18) 새로 묻지 않는다. `life-librarian update`·`rebuild`(`life sync`의 사서 갱신도 같은 경로)가 갱신 뒤에 새 문서만 임베딩하고, `life-librarian embed`로 따로 돌릴 수 있다(개수만 출력). 모델을 바꾸면 다른 모델의 벡터는 지우고 새로 만든다. 요청이 연속 3번 실패하면 멈추고 나머지는 다음 실행에서 이어 한다(`life-librarian embed`는 이때 실패 종료 코드). 채팅 중에는 새 문서를 임베딩하지 않으므로 다음 sync 전까지 새 메일·첨부·일정은 의미 검색에 잡히지 않는다(벡터가 일부만 있으면 있는 것만 검색한다). 서버가 없거나 응답하지 않거나 아직 벡터가 없으면 지금처럼 낱말로 찾는다(로그에 이유 한 줄, 질문·문서 내용은 남기지 않음). 설정 화면에는 없고 설정 화면 저장은 이 값을 지우지 않는다.

## 설치
Rust 툴체인(저장소 루트의 `rust-toolchain.toml`)이 필요하다. `deploy/install.sh`가 release로 빌드해 실행 파일을 `~/.local/lib/life/bin/`에 복사하고 `~/.local/bin/`에서 링크로 가리키게 하며, `deploy/systemd/`의 유닛을 `~/.config/systemd/user/`에 둔다(이전 실행 파일과 바뀌는 유닛은 `~/.local/share/life-install-backup/`에 백업). 복사해서 두므로, 나중에 어느 체크아웃에서 빌드해도 돌고 있는 서비스는 바뀌지 않는다. 바꾸려면 install.sh를 다시 돌린다.

    server/deploy/install.sh
    systemctl --user enable --now life-ingest life-sync.timer   # 처음 한 번
    systemctl --user restart life-ingest                         # 다시 설치한 뒤

## 어느 OS에서든

라이브러리는 어느 OS에서든 그대로 돌아야 한다. 기본 위치는 `config::OsDirs`로 OS에서 고르고(리눅스는 예전 그대로 `~/.config/vault-sync`, `~/.local/share/life`, `~/.local/share/vault-sync`. `XDG_CONFIG_HOME`·`XDG_DATA_HOME`을 두면 그쪽을 따른다), 한 OS의 경로를 코드에 적지 않는다. 1시간마다의 동기화 같은 예약 실행은 라이브러리가 아니라 그것을 띄우는 쪽의 일이다(상시 서버는 `deploy/systemd/life-sync.timer`, 서랍을 PC에 둔 데스크톱 앱은 앱이 1시간마다). 라이브러리 안의 스레드는 요청 하나에 딸린 일회성 작업(하루 정리 만들기, 할 일 뽑기)뿐이다. 받기 서버를 다른 프로그램 안에서 띄울 때는 `host`(설정 읽기 `IngestSettings`·`Models`, 모델 저장·실행 중 바꾸기 `save_models`·`LiveModels`, 토큰 만들기·바꾸기(`ensure_token`·`replace_token`, 실행 중인 서버는 `App::set_token`으로 바로 새 토큰만 받는다), 주소별로 띄우고 멈추기)를 쓴다. 모을 곳 설정(메일 계정·폴더, 달력)은 `collect::sources`가 하고, `life add`와 데스크톱 설정 화면이 같이 쓴다. 라이브러리가 남기는 기록은 `log` 매크로로 나가고 띄우는 쪽이 받을 곳을 한 번 정한다(`logging`): 명령줄 도구는 예전처럼 stderr, 데스크톱 앱은 파일. 동기화 작업(메일, 첨부, 일정, 사서 갱신)은 `collect::sync`의 함수이고, 출력하지 않고 줄 단위로 알린다. 명령줄 도구는 그 줄을 stdout·stderr로 옮기고, 데스크톱 앱은 같은 함수를 직접 부른다.

## 테스트

    server/check.sh

테스트, clippy, rustfmt, 그리고 Windows 대상 컴파일(`cargo xwin check`)을 차례로 본다. 이 라이브러리는 데스크톱 앱 안에서도 돌기 때문에(`docs/adr/0002-vault-on-the-users-device.md`) Windows에서 컴파일되지 않으면 실패로 친다. release 빌드는 하지 않으므로 돌고 있는 서비스의 실행 파일을 건드리지 않는다. 테스트만 돌리려면 `cargo test -p life-server`.

가짜 모델과 임시 폴더로 실제 HTTP 서버를 띄워 확인한다. 실제 메일·일정 서버에 닿는 부분(IMAP, CalDAV)은 테스트하지 않는다.


## HTTPS 주소
`life-ingest`는 기본으로 `127.0.0.1`에 HTTP로 뜬다(`INGEST_CERT`·`INGEST_KEY`에 PEM 파일을 주면 직접 HTTPS). 폰·PC에서 받으려면 `ingest.env`의 `INGEST_BIND`를 이 머신의 tailnet 주소로 바꾸고, 보통은 tailscale serve로 앞에 HTTPS를 붙여 쓴다(한 번만, sudo 필요).

`INGEST_BIND`에는 주소를 쉼표로 여럿 줄 수 있고, 하나하나에서 받는다. 같은 와이파이의 폰·PC가 Tailscale 없이 닿게 하려면 이 머신의 사설 주소(`10.x`, `172.16~31.x`, `192.168.x`)를 더한다:

    INGEST_BIND=100.101.171.95,192.168.0.20

같은 와이파이 주소는 그 와이파이 안에서만 닿고, 기록과 토큰이 암호화 없이 와이파이로 간다. 집이나 사무실처럼 믿는 와이파이에서만 쓴다. 폰과 데스크톱 앱은 https, tailnet, 이 기기, 같은 와이파이(사설 IPv4 주소를 숫자 그대로 쓴 것)의 http 말고는 토큰을 보내지 않는다. 주소 하나를 못 열면(그 주소가 이 머신에 없는 등) 시작하지 않는다. 바꾼 뒤 `systemctl --user restart life-ingest`.

    sudo tailscale serve --bg --https=48765 http://<tailnet 주소>:8765

주소는 `https://<머신>.<tailnet>.ts.net:48765/`. 폰 앱은 기존 HTTP 주소를 그대로 쓸 수 있다. 되돌리기: `sudo tailscale serve --https=48765 off`.

## 토큰 바꾸기
토큰(`ingest.env`의 `INGEST_TOKEN`) 하나가 서랍에 쓰고 읽는 모든 권한이다. 토큰이나 연결 코드를 아는 기기를 잃었거나 코드가 남에게 갔으면 새 토큰으로 바꾼다:

    life token new            # 새 토큰을 ingest.env에 쓴다. 토큰은 출력하지 않는다
    life token new --show     # 새 토큰을 화면에 보여 준다
    systemctl --user restart life-ingest

새 토큰은 처음 만들 때와 같이 OS의 난수 16바이트(16진수 32자)다. 다른 설정은 그대로 두고, 바꾸기 전의 파일은 `ingest.env.bak-token`에 남긴다(둘 다 0600). `life-ingest`를 다시 시작할 때까지는 이전 토큰을 받고, 다시 시작하면 이전 토큰은 바로 거절된다. 그 뒤 폰은 새 연결 코드를, 다른 PC는 새 토큰을 넣어 다시 잇는다. `ingest.env`가 없으면 아무것도 만들지 않고 실패한다. 서랍을 PC에 둔 데스크톱 앱은 설정의 '토큰 바꾸기'로 같은 일을 하고, 다시 시작하지 않아도 바로 새 토큰만 받는다.
