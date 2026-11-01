// The settings page. It talks to the Rust shell through tauri invoke; the shell owns the config and, when the vault is
// on this PC, the server's settings. Every value from the shell goes into the page as text (textContent / value).
import { initSources, loadSources } from "./sources.js";

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
  $("welcomeBox").hidden = !host;
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
  await Promise.all([loadSources(), loadModels(), loadSync(), loadHeld(), loadBackup()]);
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

// ---------- mail, calendars, address books (shared with the first-run guide) ----------
initSources(() => setTimeout(loadSync, 1000));

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

$("modelCheck").onclick = async () => {
  $("modelCheck").disabled = true;
  say("modelMsg", "모델에게 짧게 물어보는 중…", true);
  try {
    const r = await invoke("model_check", { url: $("chatUrl").value, model: $("chatModel").value });
    say("modelMsg", `답했습니다(${r.seconds}초): "${r.reply}"`, true);
  } catch (e) { say("modelMsg", String(e), false); }
  $("modelCheck").disabled = false;
};

// ---------- sync ----------
const JOB = { mail: "메일", files: "첨부", calendar: "일정", contacts: "주소록", librarian: "색인", graph: "그래프", backup: "백업" };
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

// ---------- what the vault holds, and forgetting part of it ----------
const mb = (b) => (b / 1e6 < 1 ? "1MB 미만" : Math.round(b / 1e6) + "MB");
async function loadHeld() {
  let held = [];
  try { held = await invoke("vault_holdings"); } catch (e) { $("forgetMsg").textContent = String(e); return; }
  const rows = $("heldRows"); rows.replaceChildren();
  const pick = $("forgetSource"); pick.replaceChildren();
  for (const h of held) {
    const tr = el("tr");
    for (const t of [h.label, String(h.files), mb(h.bytes), h.first + " ~ " + h.last]) tr.append(el("td", t));
    rows.append(tr);
    const o = el("option", h.label); o.value = h.source; pick.append(o);
  }
}
const span = () => ({ from: $("forgetFrom").value, to: $("forgetTo").value });
let planned = null; // the request the plan was shown for: 지우기 forgets exactly that
for (const id of ["forgetSource", "forgetFrom", "forgetTo"]) $(id).onchange = () => { planned = null; $("forgetGo").disabled = true; };
$("forgetPlan").onclick = async () => {
  const req = { source: $("forgetSource").value, span: span() };
  try {
    const p = await invoke("vault_plan", req);
    planned = p.files.length || p.todos ? req : null;
    $("forgetGo").disabled = !planned;
    $("forgetMsg").textContent = planned
      ? `파일 ${p.files.length}개(${mb(p.bytes)})와 받지 않은 할 일 제안 ${p.todos}개가 지워집니다.` +
        (p.comes_back ? " 이 출처는 동기화 때마다 원래 서비스에서 다시 읽어 옵니다. 거기에 남아 있으면 다시 들어오니, 원래 서비스에서 지우거나 수집을 끄세요." : "")
      : "그 기간에는 지울 것이 없습니다.";
  } catch (e) { planned = null; $("forgetGo").disabled = true; $("forgetMsg").textContent = String(e); }
};
$("forgetGo").onclick = async () => {
  if (!planned) return;
  const label = $("forgetSource").selectedOptions[0]?.textContent || planned.source;
  if (!confirm(`${label}의 ${planned.span.from} ~ ${planned.span.to} 기록을 지웁니다. 되돌릴 수 없습니다. 먼저 내보내기를 해 두면 안전합니다. 지울까요?`)) return;
  $("forgetGo").disabled = true;
  try {
    const p = await invoke("vault_forget", planned);
    $("forgetMsg").textContent = `지웠습니다: 파일 ${p.files.length}개, 할 일 제안 ${p.todos}개. 검색에서는 다음 갱신 때 빠집니다.`;
  } catch (e) { $("forgetMsg").textContent = String(e); }
  planned = null;
  await loadHeld();
};

// ---------- the daily sealed copy ----------
async function loadBackup() {
  let st;
  try { st = await invoke("backup_status"); } catch (e) { $("backupMsg").textContent = String(e); return; }
  if (st.folder) $("backupDir").value = st.folder;
  $("backupNow").disabled = !st.folder;
  const when = (t) => (t || "").replace("T", " ").slice(0, 16);
  $("backupMsg").textContent = !st.folder ? "아직 설정하지 않았습니다."
    : st.error ? `마지막 시도(${when(st.last_run)})가 실패했습니다: ${st.error}`
    : st.last_ok ? `마지막 백업: ${when(st.last_ok)} · ${st.file}` : "다음 동기화 때 첫 백업을 만듭니다.";
}
$("backupSave").onclick = async () => {
  const dir = $("backupDir").value.trim(), password = $("backupPw").value;
  if (!password) { $("backupMsg").textContent = "비밀번호를 넣어 주세요(바꾸지 않아도 다시 넣습니다)."; return; }
  try { await invoke("backup_set", { dir, password }); $("backupPw").value = ""; await loadBackup(); }
  catch (e) { $("backupMsg").textContent = String(e); }
};
$("backupNow").onclick = async () => {
  $("backupNow").disabled = true;
  $("backupMsg").textContent = "백업하는 중… 기록이 많으면 몇 분 걸립니다.";
  try { $("backupMsg").textContent = await invoke("backup_now"); } catch (e) { $("backupMsg").textContent = String(e); }
  $("backupNow").disabled = false;
};

$("exportVault").onclick = async () => {
  $("exportVault").disabled = true;
  $("exportMsg").textContent = "내보내는 중… 기록이 많으면 몇 분 걸립니다.";
  try { $("exportMsg").textContent = "내보냈습니다: " + await invoke("export_vault"); }
  catch (e) { $("exportMsg").textContent = String(e); }
  finally { $("exportVault").disabled = false; }
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
