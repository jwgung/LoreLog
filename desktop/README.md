# life-agent (PC 수집기)

PC에서 어느 앱의 어느 창을 얼마나 썼는지를 서버(`../server`)로 보낸다. 서버는 이것을 하루 정리의 재료로 쓴다("PC에서 5분 넘게 머문 창").
Rust로 쓴다(의존성 없는 exe 하나, 세 OS 교차 빌드 가능). **Windows만 구현**돼 있고 리눅스·맥은 나중에 `agent/src/platform/`에 추가한다.
구조와 크레이트 선택은 Omni 데스크톱 클라이언트(코어 crate와 OS 셸을 나눔, `windows-sys`, 자동 실행)를 따랐다. 코드는 복사하지 않았다.

## 무엇을 기록하나
- `window`: 앞에 있던 창의 앱 이름(`chrome.exe`)과 창 제목, 시작·끝 시각, 초 단위 길이.
- `idle`: 키보드·마우스를 5분 넘게 안 쓴 시간(`reason: idle`), 화면이 잠긴 시간(`reason: locked`). 자리를 비운 시간이 일한 시간으로 잡히지 않는다. 비움은 알아챈 때가 아니라 **마지막 입력**에서 시작한다.
- 키 입력 내용, 화면, 파일 내용은 기록하지 않는다. 창 제목만 본다.
- 비밀번호 관리자 창과 시크릿(InPrivate) 창은 제목 대신 `(제목 숨김)`으로 보낸다. 목록은 `config.json`의 `hide_title_apps`, `hide_title_contains`(대소문자 무시, 부분 일치).
- 3초보다 짧게 스친 창(Alt+Tab)은 기록하지 않는다. 한 창에 오래 있으면 10분 단위로 끊어 보낸다(프로세스가 죽어도 최대 10분만 잃는다).
- 절전·최대 절전으로 표본이 끊긴 사이는 일한 시간으로 세지 않는다.
- 서버에 닿지 않으면 `spool.ndjson`에 쌓아 두었다가 닿을 때 보낸다. 서버가 성공을 답한 묶음만 지운다. 같은 줄을 다시 보내도 서버가 한 번만 저장한다.
- 수집기는 사용자당 하나만 뜬다(`agent.lock`).

## 구조
- `core/` 순수 로직과 전송: `segments`(표본 → 구간), `spool`, `sender`, `agent`(루프 한 번), `config`. OS 호출이 없어 어디서나 테스트한다.
- `agent/` OS 쪽: `platform/windows.rs`(앞 창·프로세스·마지막 입력, `windows-sys`), 명령줄 `life-agent`, 콘솔 창 없는 백그라운드 `life-agentd`.

## Windows에서 시작하기
1. Rust 설치(rustup, Visual Studio Build Tools의 "C++를 사용한 데스크톱 개발"). 저장소의 `desktop/rust-toolchain.toml`이 버전을 고정한다.
2. `desktop` 폴더에서 `cargo build --release`. 결과: `target\release\life-agent.exe`, `life-agentd.exe`(같은 폴더에 둔다).
3. 설정 만들기와 채우기:

        life-agent config        # %APPDATA%\life-agent\config.json 을 만들고 경로를 알려 준다
        notepad %APPDATA%\life-agent\config.json    # url 과 token 을 직접 채운다

   `url`은 서버 주소(예: `https://gungbuntu.tail8d1ab7.ts.net:48765`, PC가 tailnet에 있어야 한다), `token`은 서버의 `INGEST_TOKEN`.
4. `life-agent once` — 지금 앞에 있는 창이 제대로 읽히는지만 본다(저장·전송 없음).
5. `life-agent run` — 수집 시작(Ctrl+C로 끝). 로그인 때마다 자동으로 시작하려면 `life-agent install`(사용자 시작 항목 하나를 만든다), 없애려면 `uninstall`.
6. `life-agent status`(설정·쌓인 기록 수), `life-agent flush`(쌓인 기록을 지금 보낸다). 로그는 `%APPDATA%\life-agent\agent.log`(토큰은 남기지 않는다).

## 테스트
`cargo test` — 구간 계산 10개(자리 비움 시작·복귀, 잠금, 절전, 비공개 제목 등)와, 가짜 수신 서버에 대한 전송 5개(인증 거절 시 보존, 재전송 중복 없음, 표본 실패에도 계속).
실제 `life-ingest`(파이썬)로도 `life-agent flush`를 보내 서버가 저장하고 하루 정리 재료(`gather_pc`)가 읽는 것을 확인했다.

## 아직 안 된 것
- Windows 코드(`platform/windows.rs`)는 이 저장소를 만든 리눅스 PC에서 `x86_64-pc-windows-gnu`/`msvc`로 **타입 검사만** 했고 실행해 보지 못했다. 실제 Windows에서 `once`로 확인이 필요하다. 이 PC에는 `ring`을 빌드할 mingw가 없어 exe 교차 빌드는 못 했다.
- 리눅스(X11/Wayland)와 맥 백엔드. 토큰을 설정 파일 대신 OS 키체인에 두는 것.
