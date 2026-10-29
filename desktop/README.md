# life-agent (PC 수집기)

PC에서 어느 앱의 어느 창을 얼마나 썼는지를 서버(`../server`)로 보낸다. 서버는 이것을 하루 정리의 재료로 쓴다("PC에서 5분 넘게 머문 창").
파이썬 3 표준 라이브러리만 쓴다(설치할 패키지 없음). **Windows만 구현**돼 있고 리눅스·맥은 나중에 `life_agent/backends/`에 추가한다.

## 무엇을 기록하나
- `window`: 앞에 있던 창의 앱 이름(`chrome.exe`)과 창 제목, 시작·끝 시각, 초 단위 길이.
- `idle`: 키보드·마우스를 5분 넘게 안 쓴 시간(`reason: idle`), 화면이 잠긴 시간(`reason: locked`). 자리를 비운 시간이 일한 시간으로 잡히지 않게 한다.
- 키 입력 내용, 화면, 파일 내용은 기록하지 않는다. 창 제목만 본다.
- 비밀번호 관리자 창과 시크릿(InPrivate) 창은 제목 대신 `(제목 숨김)`으로 보낸다. 목록은 `config.json`의 `hide_title_apps`, `hide_title_patterns`.
- 3초보다 짧게 스친 창(Alt+Tab)은 기록하지 않는다. 한 창에 오래 있으면 10분 단위로 끊어 보낸다.
- 절전·최대 절전으로 기록이 끊긴 사이는 일한 시간으로 세지 않는다.
- 서버에 닿지 않으면 `spool.ndjson`에 쌓아 두었다가 닿을 때 보낸다. 서버가 돌려준 성공을 받은 줄만 지운다. 같은 줄을 다시 보내도 서버가 한 번만 저장한다.

## Windows에서 시작하기
1. Python 3.10 이상을 설치한다(python.org, "Add python.exe to PATH" 체크).
2. 이 `desktop` 폴더를 PC로 가져온다(저장소 복제 또는 폴더 복사).
3. `desktop` 폴더에서 PowerShell:

        python -m life_agent config      # %APPDATA%\life-agent\config.json 을 만들고 경로를 알려 준다
        notepad $env:APPDATA\life-agent\config.json    # url 과 token 을 직접 채운다
        python -m life_agent once        # 지금 앞에 있는 창이 제대로 읽히는지만 본다(저장·전송 없음)
        python -m life_agent run         # 수집 시작. 창을 닫으면 멈춘다
        python -m life_agent status      # 설정·쌓인 기록 수

4. `url`은 서버 주소다. 예: `https://gungbuntu.tail8d1ab7.ts.net:48765` (PC가 tailnet에 있어야 한다). `token`은 서버의 `INGEST_TOKEN`.
5. 로그인할 때마다 자동으로 시작하려면 `python -m life_agent install`(현재 사용자의 시작 프로그램 항목을 하나 만든다). 없애려면 `uninstall`.

로그는 `%APPDATA%\life-agent\agent.log`(토큰은 남기지 않는다).

## 구조
- `life_agent/segments.py` 샘플 → 구간 변환(OS 호출 없음, 테스트 대상)
- `life_agent/backends/windows.py` 앞 창과 마지막 입력 시각(user32, ctypes)
- `life_agent/agent.py` 루프(샘플, 구간, 쌓기, 보내기)
- `life_agent/spool.py`, `sender.py` 쌓아 두기와 전송
- `life_agent/cli.py` 명령

## 테스트
`python3 -m unittest discover -s tests` (서버 코드를 같은 프로세스에서 띄워 보내기까지 확인한다. Windows 호출만 빠진다.)

## 아직 안 된 것
- Windows 백엔드는 이 저장소를 만든 리눅스 PC에서 실행해 보지 못했다. 실제 Windows에서 `once`로 확인이 필요하다.
- 리눅스(X11/Wayland)와 맥 백엔드. exe 패키징(PyInstaller).
