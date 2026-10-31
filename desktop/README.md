# life-agent (PC 수집기)

PC에서 어느 앱의 어느 창을 얼마나 썼는지를 서버(`../server`)로 보낸다. 서버는 이것을 하루 정리의 재료로 쓴다("PC에서 5분 넘게 머문 창").
Rust로 쓴다(의존성 없는 exe 하나, 세 OS 교차 빌드 가능). **Windows만 구현**돼 있고 리눅스·맥은 나중에 `agent/src/platform/`에 추가한다.
Omni 데스크톱 클라이언트(`dev` 브랜치의 `pc_activity`)를 읽고 코어 crate와 OS 셸을 나누는 구조, `windows-sys` 직접 호출, 툴체인 고정, 그리고 Windows 표본기의 함정 세 가지(제목 읽기가 멈출 수 있음, 권한 없는 창, UWP 프레임 호스트)를 따랐다. 코드는 복사하지 않았고, 단일 인스턴스 잠금·전송(`ureq`)은 우리가 따로 골랐다.

## 무엇을 기록하나
- `window`: 앞에 있던 창의 앱 이름(`chrome.exe`)과 창 제목, 시작·끝 시각, 초 단위 길이.
- 감시 폴더의 **산출물 파일**: 사용자가 고른 폴더(예: 다운로드)의 맨 위에 놓인 파일을 원본 그대로 서버에 보관한다. 폴더는 **사용자가 직접 고른다** — 아무것도 더하지 않으면 아무 폴더도 보지 않는다(`config.toml`의 `watch_dirs`, 기본 빈 목록). 자세한 규칙은 아래 "감시 폴더".
- `idle`: 키보드·마우스를 5분 넘게 안 쓴 시간(`reason: idle`), 화면이 잠긴 시간(`reason: locked`), 앞 창이 있지만 읽을 수 없는 시간(`reason: unreadable` — 관리자 권한으로 뜬 앱 등. 잠금으로 기록하지 않는다). 자리를 비운 시간이 일한 시간으로 잡히지 않는다. 비움은 알아챈 때가 아니라 **마지막 입력**에서 시작한다.
- 키 입력 내용, 화면, 파일 내용은 기록하지 않는다. 창 제목만 본다.
- 비밀번호 관리자 창과 시크릿(InPrivate) 창은 제목 대신 `(제목 숨김)`으로 보낸다. 목록은 `config.toml`의 `hide_title_apps`, `hide_title_contains`(대소문자 무시, 부분 일치).
- Store(UWP) 앱은 앞 창이 `ApplicationFrameHost.exe`로 보이므로 자식 창의 실제 앱 이름으로 바꿔 적는다. 창 제목은 상대 프로세스가 멈춰 있어도 막히지 않는 호출(`InternalGetWindowText`)로 읽고, 2048자를 넘는 제목은 통째로 버린다.
- 3초보다 짧게 스친 창(Alt+Tab)은 기록하지 않는다. 한 창에 오래 있으면 10분 단위로 끊어 보낸다(프로세스가 죽어도 최대 10분만 잃는다).
- 절전·최대 절전으로 표본이 끊긴 사이는 일한 시간으로 세지 않는다.
- 서버에 닿지 않으면 `spool.ndjson`에 쌓아 두었다가 닿을 때 보낸다. 서버가 성공을 답한 묶음만 지운다. 같은 줄을 다시 보내도 서버가 한 번만 저장한다.
- `life-agent run`은 사용자당 하나만 뜬다(`agent.lock`). 데스크톱 앱은 이 잠금을 잡지 않으므로 둘을 함께 돌리지 않는다.

## 구조
- `core/` 순수 로직과 전송: `segments`(표본 → 구간), `spool`, `sender`(전송·파일 업로더 `ServerUploader`), `intake`(감시 폴더 엔진), `agent`(루프 한 번), `config`. OS 호출이 없어 어디서나 테스트한다.
- `agent/` OS 쪽: `platform/windows.rs`(앞 창·프로세스·마지막 입력, `windows-sys`), 명령줄 `life-agent`. 수집 루프 `run::run_until(stop)`은 CLI와 데스크톱 앱이 함께 쓴다.
- `shell/` 데스크톱 앱(Tauri): 비서 화면(대화·할일·서랍·말하기)과 설정 화면을 **앱에 번들**해 띄우고, 백그라운드로 수집 루프를 돌리며, 트레이(열기·채팅 열기·설정·종료)를 둔다. 금고를 이 PC에 두면 서버도 돌린다(아래 "금고를 어디에 두나"). 화면은 Tauri webview로 그리지만 서버에서 당겨오지 않는다("웹 클라이언트"가 아니다) — 서버 주소·토큰은 앱 설정(`server_config` 커맨드)에서 받아 서버 API를 직접 호출한다. `ui/index.html`=비서, `ui/settings.html`=설정.

배포하는 것은 둘이다 — **`life-desktop`**(보통 사용자는 이것만: 대화 + 백그라운드 수집 + 트레이)와 **`life-agent`**(설정·진단·감시 폴더를 터미널에서 다루는 명령줄). 데스크톱 앱이 백그라운드 수집을 하므로 별도 데몬은 두지 않는다. 데스크톱 앱과 `life-agent run`을 동시에 돌리지 않는다(둘 다 수집한다).

## 감시 폴더 (산출물 입구)
사람이 일해서 만든 산출물을 미리 정한 구조 없이 원본 그대로 보관하기 위한 입구. Omni 데스크톱의 감시 폴더(`desktop-watched-folder-tidy`)를 참고하되 우리는 **보관만** 한다(서버가 파일을 되돌려 주는 양방향 동기화는 하지 않는다).
- 사용자가 고른 폴더의 **맨 위 파일만** 본다(하위 폴더는 들어가지 않는다 — 그건 사용자의 구조다).
- 파일이 `settle_s`(기본 10초) 동안 크기·수정시각 그대로일 때만 읽는다(아직 내려받는/저장 중인 파일을 피한다).
- `max_upload_mb`(기본 20)보다 큰 파일은 읽지도 올리지도 않고 건너뛴다(일지에 표시해 다시 시도하지 않는다). 서버의 `INGEST_MAX_MB` 이하로 둔다 — 그래야 올린 파일을 서버가 받는다.
- 작업 중 파일(`.crdownload`·`.part`·`~$…` 등)·숨김 파일·OS 부속 파일(`desktop.ini` 등)은 거른다.
- **원본은 절대 잃지 않고 두 번 보내지 않는다**: 내용 해시로 서버에 이미 있는지 묻고(`GET /ingest/file/<sha>`), 없으면 바이트를 올리고(`POST /ingest/file`) 있으면 목격만 기록한다(`X-File-Sighting`). 서버가 바이트를 보관했다고 답한 **뒤에만** 일지(`intake-state.json`)에 끝났다고 적는다. 중간에 끊기면 다음 훑기가 같은 파일을 다시 집는다. 우리는 복사만 보관하고 사용자의 원본 파일은 옮기거나 지우지 않는다.
- 내용 해시가 메일 첨부(`mail-files`)·창 제목 등 다른 채널에서 같은 산출물을 알아보는 열쇠다. 서버 보관 위치는 메일 첨부와 같은 금고(`raw/files/<앞 2자>/<sha256>`), 사건 기록은 `raw/pcfile/<날짜>.ndjson`.

감시 폴더 관리(CLI):

    life-agent watch list                 # 지금 보는 폴더
    life-agent watch add  C:\Users\나\Downloads
    life-agent watch remove C:\Users\나\Downloads

데스크톱 앱(`shell/`)의 설정 화면에서도 더하고 뺄 수 있다.

## 데스크톱 앱 (Tauri)
    cd desktop && cargo tauri dev   # 개발 실행 (리눅스는 WebKitGTK, Windows는 WebView2)
    cd desktop && cargo tauri build # 설치용 번들

처음 뜨면 금고를 어디에 둘지 묻는다(위). "다른 곳의 금고에 연결"을 고르면 서버 주소와 토큰을 넣는 화면이 나오고, 넣으면 앱에 번들된 비서 화면(대화·할일·서랍·말하기)으로 넘어간다. 화면은 앱 안에 있고(브라우저로 열리지 않는다) 서버 API만 호출한다. 창을 닫아도 트레이에 남아 수집을 계속하고, 트레이 "채팅 열기"·"설정"으로 화면을 오간다. 설정은 CLI와 같은 파일을 쓴다 — 비밀 아닌 설정은 `config.toml`, 토큰은 별도 파일 `token`(0600). 서버의 `sources.toml`+`env` 분리와 같은 방식.

## 금고를 어디에 두나 (처음 켤 때)
처음 켜면 고른다(`config.toml`의 `mode`, 결정은 `docs/adr/0002-vault-on-the-users-device.md`).
- **이 PC에 두기**(`host`): 앱이 서버 라이브러리(`life-server`)를 품는다(`shell/src-tauri/src/host.rs`). 데이터와 설정은 서버가 이 OS에서 쓰는 자리에 둔다(Windows `%LOCALAPPDATA%\life`, `%APPDATA%\vault-sync`). 그래서 같은 PC의 `life` 명령줄 도구도 같은 금고를 본다(명령줄의 `LIFE_*_DIR` 같은 위치 바꾸기는 앱이 따르지 않는다). 메일·일정 노트는 `config.toml`의 `vault_dir`(예: Obsidian 볼트)에, 비어 있으면 `%LOCALAPPDATA%\life\notes`에 쓴다.
  - 받기: loopback(`127.0.0.1:<포트>`, 이 앱의 화면과 PC 수집기)과 tailnet 주소(폰·다른 PC). 포트는 `ingest.env`의 `INGEST_PORT`(기본 8765), 토큰은 처음 켤 때 `ingest.env`에 만든다. Tailscale이 앱보다 늦게 올라와도 30초마다 다시 보고 tailnet 주소에서도 받기 시작한다. Windows는 처음 tailnet 주소로 받을 때 방화벽 허용을 묻는다.
  - 다른 기기에 넣을 주소는 `http://<PC의 tailnet 이름>:<포트>`. tailnet 안의 HTTP는 WireGuard로 이미 암호화된다. HTTPS를 원하면 사용자가 `tailscale serve`로 붙인다. tailnet 안에서는 토큰이 유일한 문이므로, 다른 사람과 tailnet을 함께 쓰면 Tailscale ACL로 이 포트에 닿을 기기를 좁힌다. 받는 주소는 Tailscale 인터페이스의 tailnet 주소뿐이다(같은 100.64.0.0/10 대역이라도 다른 인터페이스, 예컨대 통신사 CGNAT는 쓰지 않는다).
  - 서버를 띄우지 못하면(포트를 다른 프로그램이 쓰는 등) 처음 화면에 그 이유를 보여 준다. 금고를 이 PC에 둔 동안 설정의 서버 주소는 바꿀 수 없다.
  - 동기화: 켤 때 한 번, 그 뒤 1시간마다 메일·첨부·일정을 가져오고 사서 색인을 갱신한다(`life sync`와 같은 함수). 결과는 `life status`와 같은 `status.json`.
  - 설정 화면: 다른 기기에 넣을 주소와 토큰(보기·복사), 메일(서버·계정·비밀번호 → 연결하고 폴더 고르기), 일정(CalDAV 서버 → 달력 고르기), 주소록(CardDAV 서버 → 주소록 고르기), 모델(대화·음성 주소, `ingest.env`의 `LIFE_LLM_URL`·`LIFE_LLM_MODEL`·`LIFE_ASR_URL`·`LIFE_ASR_MODEL`), 동기화 결과와 "지금 동기화". 메일·일정을 저장하면 바로 동기화한다. 모델은 저장한 다음 질문부터 쓴다(다시 켜지 않아도 된다). 모델 주소가 비어 있으면 대화·음성은 "주소가 설정되지 않았다"고 답한다.
  - 금고 위치는 설정에서 바꿀 수 있다. 바꿔도 기록은 옮기거나 지우지 않는다.
  - 서버의 기록(받은 줄, 실패, 사서가 건너뛴 파일)은 `%LOCALAPPDATA%\life\desktop.log`.
- **다른 곳의 금고에 연결**(`connect`): 지금까지의 방식. 서버 주소와 토큰을 넣는다. 모드가 생기기 전에 주소를 넣어 둔 설정은 이쪽으로 읽는다.

## 빌드
이 저장소의 리눅스 PC에서 Windows exe까지 만든다(`cargo-xwin`이 Windows SDK를 받아 둔다).

    cd desktop && cargo xwin build --release --target x86_64-pc-windows-msvc -p life-desktop -p life-agent

결과(이 PC는 `CARGO_TARGET_DIR`가 `~/.cache/cargo-target`): `.../x86_64-pc-windows-msvc/release/life-desktop.exe`(데스크톱 앱)와 `life-agent.exe`(명령줄). Windows에서 직접 빌드하려면 Rust와 Visual Studio Build Tools(C++)를 설치하고 `cargo build --release -p life-desktop -p life-agent`. 데스크톱 앱은 실행할 때 **WebView2 런타임**이 필요하다(Windows 11 기본 포함, Windows 10은 Edge WebView2 Evergreen Runtime 설치).

## Windows에서 시작하기 (데스크톱 앱)
1. `life-desktop.exe`를 실행한다. 처음에는 금고를 어디에 둘지 묻는다. "이 PC에 두기"면 바로 대화로 넘어간다. 설정에서 메일·일정·모델을 넣고, 폰에 넣을 주소·토큰을 본다. "다른 곳의 금고에 연결"이면 서버 주소(예: `https://gungbuntu.tail8d1ab7.ts.net:48765`, PC가 tailnet에 있어야 한다)와 토큰(서버의 `INGEST_TOKEN`)을 넣고 "저장하고 대화 열기".
2. 그 자리에서 앱에 번들된 비서 화면(대화)으로 넘어간다. 창을 닫아도 트레이에 남아 수집을 계속한다. 트레이: 열기 / 채팅 열기 / 설정 / 종료.
3. "설정"에서 감시 폴더를 더한다(예: 다운로드). 아무것도 더하지 않으면 파일은 보지 않고 창 활동만 수집한다.

## 명령줄로 (life-agent, 선택)
설정·진단만 터미널에서 하고 싶을 때. 데스크톱 앱과 동시에 수집을 돌리지는 않는다.

    life-agent config        # %APPDATA%\life-agent\config.toml 을 만들고 token 파일 자리를 알려 준다 (url·token 을 직접 채운다)
    life-agent once          # 지금 앞 창이 제대로 읽히는지만 본다(저장·전송 없음)
    life-agent watch add "C:\Users\나\Downloads"
    life-agent run           # 수집 시작(Ctrl+C로 끝)
    life-agent status        # 설정·쌓인 기록·감시 폴더
    life-agent flush         # 쌓인 기록을 지금 보낸다

로그는 `%APPDATA%\life-agent\agent.log`(토큰은 남기지 않는다).

## 테스트
`cargo test` — 구간 계산(자리 비움 시작·복귀, 잠금, 절전, 비공개 제목 등), 가짜 수신 서버에 대한 전송(인증 거절 시 보존, 재전송 중복 없음, 표본 실패에도 계속), 감시 폴더(안정 전 안 올림, 안정 후 올림, 작업중/숨김 파일 거름, 하위 폴더 제외, 이미 있는 바이트는 목격만, 중단 뒤 재전송·원본 보존, 재시작 뒤 재업로드 없음, 큰 파일은 안 읽고 건너뜀, 거부된 파일이 뒤를 막지 않음).
서버 쪽은 `cargo test -p life-server`에 파일 입구 테스트(해시로 보관·목격 기록, 재전송 한 블롭, 해시 불일치 거부, 조회, 목격 전용, 잘못된 sha 거부)가 있다.
서버가 Python이던 때 실제 `life-ingest`로 end-to-end도 확인했다(Rust 서버는 같은 HTTP 계약을 테스트로 지킨다) — 수집기가 감시 폴더의 파일을 서버에 올리고(한글 파일명 포함), 서버가 내용 해시로 보관하고 `pcfile` 기록을 남기며, 원본 파일은 그대로 남고, 큰 파일은 건너뛰는 것.

## 아직 안 된 것
- Windows 코드(`platform/windows.rs`)는 이 저장소를 만든 리눅스 PC에서 **교차 빌드만** 했고 실행해 보지 못했다. 데스크톱 앱(`life-desktop.exe`)도 디스플레이 없는 PC에서 빌드만 했고 GUI를 띄워 보지 못했다. 실제 Windows에서 확인이 필요하다.
- 로그인 때 자동 시작: 설정 화면의 "로그인할 때 자동으로 시작"으로 사용자가 켠다(기본 꺼짐). 켜고 끈 상태는 OS가 갖고(Windows는 Run 키), 화면은 그 값을 읽어 보여준다. 로그인으로 시작하면 창 없이 트레이에서 수집만 한다.
- 리눅스(X11/Wayland)와 맥 백엔드. 토큰을 설정 파일 대신 OS 키체인에 두는 것.
