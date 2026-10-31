// The settings page. It talks to the Rust shell through tauri invoke; the shell owns the config and, when the vault is
// on this PC, the server's settings. Every value from the shell goes into the page as text (textContent / value).
const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const say = (id, text, ok) => { const m = $(id); m.textContent = text; m.className = "msg " + (ok ? "ok" : "err"); };
const msg = (text, ok) => say("msg", text, ok);
const el = (tag, text) => { const e = document.createElement(tag); if (text !== undefined) e.textContent = text; return e; };

let token = "";

async function load() {
  let cfg;
  try { cfg = await invoke("server_config"); } catch (e) { msg("설정을 읽지 못했습니다: " + e, false); return; }
  const host = cfg.mode === "host";
  $("hostBox").hidden = !host;
  $("connectBox").hidden = host;
  $("hostOnly").hidden = !host || !!cfg.host_error;
  if (host) {
    token = cfg.token || "";
    if (cfg.host_error) msg("이 PC의 서버를 띄우지 못했습니다: " + cfg.host_error, false);
    else await loadHost().catch((e) => msg("설정을 읽지 못했습니다: " + e, false));
  }
  try {
    const s = await invoke("get_settings");
    $("url").value = s.url || "";
    $("token").placeholder = s.has_token ? "저장됨 — 바꾸려면 새로 입력" : "서버의 INGEST_TOKEN";
    $("cfgpath").textContent = "설정 파일: " + s.config_path;
    renderWatch(s.watch_dirs || []);
  } catch (e) { msg("설정을 읽지 못했습니다: " + e, false); }
  try { $("autostart").checked = await invoke("get_autostart"); }
  catch (e) { $("autostart").disabled = true; $("autostartMsg").textContent = String(e); }
}

// ---------- the vault on this PC ----------
async function loadHost() {
  $("devToken").value = token;
  const info = await invoke("host_info").catch(() => null);
  if (info) {
    $("devAddr").value = info.address_for_devices || "";
    $("devAddrNote").textContent = info.address_for_devices
      ? "폰 앱의 서버 연결에 이 주소와 아래 토큰을 넣습니다."
      : "아직 tailnet 주소가 없습니다. 이 PC에 Tailscale을 켜면 몇십 초 뒤 생깁니다. 그때까지는 이 PC 안에서만 받습니다.";
    const failed = info.failed.length ? " · 열지 못한 주소: " + info.failed.join(", ") : "";
    $("hostNote").textContent = "받는 곳: " + info.listening.join(", ") + failed + " · 노트: " + info.notes_dir;
  }
  await Promise.all([loadSources(), loadModels(), loadSync()]);
  try { $("asks").checked = await invoke("asks_get"); }
  catch (e) { $("asks").disabled = true; $("asksMsg").textContent = "설정을 읽지 못했습니다: " + e; }
}

$("showToken").onclick = () => {
  const shown = $("devToken").type === "text";
  $("devToken").type = shown ? "password" : "text";
  $("showToken").textContent = shown ? "보기" : "숨기기";
};
const copy = async (text, what) => {
  try { await navigator.clipboard.writeText(text); msg(what + "을(를) 복사했습니다.", true); }
  catch (e) { msg("복사하지 못했습니다: " + e, false); }
};
$("copyAddr").onclick = () => copy($("devAddr").value, "주소");
$("copyToken").onclick = () => copy(token, "토큰");

$("toConnect").onclick = async () => {
  if (!confirm("이 앱이 서버를 멈추고 다른 곳의 서버에 연결합니다. 이 PC의 기록은 지우지 않고 그대로 둡니다. 바꿀까요?")) return;
  try { await invoke("choose_mode", { mode: "connect" }); location.reload(); } catch (e) { msg(String(e), false); }
};
$("toHost").onclick = async () => {
  if (!confirm("이 PC에 금고를 두고 이 앱이 서버를 돌립니다. 다른 서버의 기록은 옮겨지지 않습니다. 바꿀까요?")) return;
  msg("서버를 띄우는 중…", true);
  try { await invoke("choose_mode", { mode: "host" }); location.reload(); } catch (e) { msg(String(e), false); }
};

// ---------- mail ----------
function checks(ul, items) {
  ul.replaceChildren();
  for (const it of items) {
    const li = el("li");
    const label = el("label");
    const box = el("input");
    box.type = "checkbox";
    box.checked = it.chosen;
    box.value = it.value;
    label.append(box, document.createTextNode(it.label));
    li.append(label);
    ul.append(li);
  }
}
const checked = (ul) => [...ul.querySelectorAll("input:checked")].map((b) => b.value);

async function loadSources() {
  const v = await invoke("sources_view").catch(() => null);
  if (!v) return;
  if (v.mail) {
    $("mHost").value = v.mail.host;
    $("mPort").value = String(v.mail.port);
    $("mUser").value = v.mail.user;
    $("mPass").placeholder = v.mail.has_password ? "저장됨 — 바꾸려면 새로 입력" : "";
    say("mMsg", "모으는 폴더: " + v.mail.folder_labels.join(", "), true);
  }
  if (v.books && v.books.length) {
    try { const u = new URL(v.books[0]); $("bServer").value = `${u.protocol}//${u.host}/`; }
    catch { $("bServer").value = v.books[0]; }
    say("bMsg", `주소록 ${v.books.length}개를 모읍니다.`, true);
    $("bOff").hidden = false;
  }
  if (v.calendars.length) {
    try { const u = new URL(v.calendars[0]); $("cServer").value = `${u.protocol}//${u.host}/`; }
    catch { $("cServer").value = v.calendars[0]; }
    say("cMsg", `달력 ${v.calendars.length}개를 모읍니다.`, true);
    $("cOff").hidden = false;
  }
}

const mailForm = () => ({
  host: $("mHost").value.trim(),
  port: Number($("mPort").value.trim() || "993"),
  user: $("mUser").value.trim(),
  password: $("mPass").value || null,
});

$("mConnect").onclick = async () => {
  const f = mailForm();
  if (!Number.isInteger(f.port) || f.port <= 0 || f.port > 65535) { say("mMsg", "포트는 숫자입니다.", false); return; }
  say("mMsg", "연결하는 중…", true);
  try {
    const folders = await invoke("mail_folders", f);
    checks($("mFolders"), folders.map((x) => ({ value: x.name, label: x.label, chosen: x.chosen })));
    $("mSave").hidden = false;
    say("mMsg", `연결했습니다. 폴더 ${folders.length}개 — 모을 폴더를 고르고 저장하세요.`, true);
  } catch (e) { say("mMsg", String(e), false); }
};
// the folders shown belong to the server and account they were read from: change those and connect again
for (const id of ["mHost", "mPort", "mUser", "mPass"]) {
  $(id).addEventListener("input", () => { $("mSave").hidden = true; $("mFolders").replaceChildren(); });
}
$("mSave").onclick = async () => {
  try {
    await invoke("mail_save", { ...mailForm(), folders: checked($("mFolders")) });
    $("mPass").value = "";
    $("mPass").placeholder = "저장됨 — 바꾸려면 새로 입력";
    say("mMsg", "저장했습니다. 지금 메일을 가져옵니다.", true);
    setTimeout(loadSync, 1000);
  } catch (e) { say("mMsg", String(e), false); }
};

// ---------- calendars ----------
$("cFind").onclick = async () => {
  say("cMsg", "달력을 찾는 중…", true);
  try {
    const cals = await invoke("calendar_list", { server: $("cServer").value.trim() });
    checks($("cList"), cals.map((c) => ({ value: c.url, label: `${c.name} (${c.components})`, chosen: c.chosen })));
    $("cSave").hidden = false;
    say("cMsg", `달력 ${cals.length}개 — 모을 달력을 고르고 저장하세요.`, true);
  } catch (e) { say("cMsg", String(e), false); }
};
$("cSave").onclick = async () => {
  const urls = checked($("cList"));
  if (!urls.length) { say("cMsg", "달력을 하나 이상 고르세요. 그만 모으려면 '일정 모으지 않기'.", false); return; }
  try { await invoke("calendar_save", { urls }); say("cMsg", "저장했습니다. 지금 일정을 가져옵니다.", true); $("cOff").hidden = false; setTimeout(loadSync, 1000); }
  catch (e) { say("cMsg", String(e), false); }
};
$("cOff").onclick = async () => {
  try { await invoke("calendar_save", { urls: [] }); $("cList").replaceChildren(); $("cSave").hidden = true; $("cOff").hidden = true; say("cMsg", "일정을 모으지 않습니다. 이미 만든 노트는 그대로 둡니다.", true); }
  catch (e) { say("cMsg", String(e), false); }
};

// ---------- address books ----------
$("bFind").onclick = async () => {
  say("bMsg", "주소록을 찾는 중…", true);
  try {
    const books = await invoke("book_list", { server: $("bServer").value.trim() });
    checks($("bList"), books.map((b) => ({ value: b.url, label: b.name, chosen: b.chosen })));
    $("bSave").hidden = false;
    say("bMsg", `주소록 ${books.length}개 — 모을 주소록을 고르고 저장하세요.`, true);
  } catch (e) { say("bMsg", String(e), false); }
};
$("bSave").onclick = async () => {
  const urls = checked($("bList"));
  if (!urls.length) { say("bMsg", "주소록을 하나 이상 고르세요. 그만 모으려면 '주소록 모으지 않기'.", false); return; }
  try { await invoke("book_save", { urls }); say("bMsg", "저장했습니다. 지금 주소록을 가져옵니다.", true); $("bOff").hidden = false; setTimeout(loadSync, 1000); }
  catch (e) { say("bMsg", String(e), false); }
};
$("bOff").onclick = async () => {
  try { await invoke("book_save", { urls: [] }); $("bList").replaceChildren(); $("bSave").hidden = true; $("bOff").hidden = true; say("bMsg", "주소록을 모으지 않습니다.", true); }
  catch (e) { say("bMsg", String(e), false); }
};

// ---------- models ----------
async function loadModels() {
  const m = await invoke("models_get").catch(() => null);
  if (!m) return;
  $("chatUrl").value = m.chat_url; $("chatModel").value = m.chat_model;
  $("asrUrl").value = m.asr_url; $("asrModel").value = m.asr_model;
}
$("modelSave").onclick = async () => {
  const models = {
    chat_url: $("chatUrl").value.trim(), chat_model: $("chatModel").value.trim(),
    asr_url: $("asrUrl").value.trim(), asr_model: $("asrModel").value.trim(),
  };
  try { await invoke("models_save", { models }); say("modelMsg", "저장했습니다. 다음 질문부터 이 모델을 씁니다.", true); }
  catch (e) { say("modelMsg", String(e), false); }
};

// ---------- sync ----------
const JOB = { mail: "메일", files: "첨부", calendar: "일정", contacts: "주소록", librarian: "색인" };
let polling = null;
async function loadSync() {
  const st = await invoke("sync_status").catch(() => null);
  if (!st) return;
  const body = $("syncTable").tBodies[0];
  body.replaceChildren();
  if (!st.jobs.length) {
    const tr = el("tr"); tr.append(el("td", "메일이나 일정을 넣으면 여기에 결과가 나옵니다.")); body.append(tr);
  }
  for (const j of st.jobs) {
    const r = j.result || {};
    const tr = el("tr");
    const state = el("td", r.last_run ? (r.ok ? "성공" : "실패") : "아직");
    state.className = "state " + (r.last_run ? (r.ok ? "ok" : "err") : "");
    tr.append(el("td", JOB[j.name] || j.name), state, el("td", (r.last_run || "").replace("T", " ").slice(0, 16)), el("td", r.detail || ""));
    body.append(tr);
  }
  $("syncNow").disabled = st.syncing;
  say("syncMsg", st.syncing ? "동기화하는 중…" : "", true);
  clearTimeout(polling);
  if (st.syncing) polling = setTimeout(loadSync, 3000);
}
$("syncNow").onclick = async () => {
  try { await invoke("sync_now"); say("syncMsg", "동기화하는 중…", true); setTimeout(loadSync, 1500); }
  catch (e) { say("syncMsg", String(e), false); }
};

$("asks").onchange = async () => {
  const on = $("asks").checked;
  try { await invoke("asks_set", { on }); $("asksMsg").textContent = on ? "질문을 남깁니다." : "질문을 남기지 않습니다. 이미 남은 것은 90일이 지나면 지워집니다."; }
  catch (e) { $("asks").checked = !on; $("asksMsg").textContent = String(e); }
};

// ---------- connecting to a server elsewhere ----------
$("save").onclick = async () => {
  try {
    await invoke("save_settings", { url: $("url").value, token: $("token").value || null });
    location.href = "index.html";   // back to the assistant, which now reads the saved config
  } catch (e) { msg(String(e), false); }
};

// ---------- watched folders, starting at login ----------
// Starting at login is the user's choice (off until they turn it on); the OS keeps it, so the box shows what the OS says.
$("autostart").onchange = async () => {
  const want = $("autostart").checked;
  try {
    $("autostart").checked = await invoke("set_autostart", { on: want });
    $("autostartMsg").textContent = $("autostart").checked ? "로그인할 때 자동으로 시작합니다." : "자동으로 시작하지 않습니다.";
  } catch (e) { $("autostart").checked = !want; $("autostartMsg").textContent = String(e); }
};

function renderWatch(dirs) {
  const ul = $("watchlist");
  ul.replaceChildren();
  if (dirs.length === 0) { const li = el("li", "아직 없음"); li.style.opacity = ".6"; li.style.border = "0"; ul.append(li); return; }
  for (const d of dirs) {
    const li = el("li");
    const b = el("button", "빼기");
    b.onclick = async () => { try { renderWatch(await invoke("remove_watch", { dir: d })); } catch (e) { msg(String(e), false); } };
    li.append(el("span", d), b);
    ul.append(li);
  }
}

$("addwatch").onclick = async () => {
  const d = $("watchdir").value.trim();
  if (!d) return;
  try { renderWatch(await invoke("add_watch", { dir: d })); $("watchdir").value = ""; msg("더했습니다.", true); }
  catch (e) { msg(String(e), false); }
};

load();
