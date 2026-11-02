// The first-run guide (#69), for a vault on this PC: ① where the vault is ② the chat model, checked with a tiny request
// ③ mail and calendar (the settings' forms) ④ the phone: the same Wi-Fi or Tailscale, and the connection code for
// either address ⑤ the first sync and the graph. Every step can be skipped; closing the guide marks it done, and the
// settings page opens it again. Every value from the shell goes into the page as text. The connection code holds the token: it is fetched only when the user
// clicks to see it, and dropped again when they hide it or leave the step.
import { initSources, loadSources } from "./sources.js";
import { LAN_NOTE, chosenIsLan, fillAddresses } from "./reach.js";

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const say = (id, text, ok) => { const m = $(id); m.textContent = text; m.className = "msg " + (ok ? "ok" : "err"); };
const el = (tag, text) => { const e = document.createElement(tag); if (text !== undefined) e.textContent = text; return e; };
const DONE_KEY = "lorelog.welcomeDone";

const sections = [...document.querySelectorAll("section[data-step]")];
let step = 0;

function show(n) {
  step = Math.max(0, Math.min(sections.length - 1, n));
  sections.forEach((s, i) => { s.hidden = i !== step; });
  [...$("steps").children].forEach((li, i) => li.classList.toggle("on", i === step));
  $("prev").hidden = step === 0;
  $("next").textContent = step === sections.length - 1 ? "대화 시작" : "다음";
  hideCode();
  clearTimeout(polling);
  if (step === 3) loadPhone();
  if (step === 4) loadProgress();
}

function finish() {
  localStorage.setItem(DONE_KEY, "1");
  location.href = "index.html";
}
$("prev").onclick = () => show(step - 1);
$("next").onclick = () => (step === sections.length - 1 ? finish() : show(step + 1));
$("skip").onclick = finish;

// ---------- ② the model ----------
let models = null;
const EXAMPLES = {
  exOllama: ["http://127.0.0.1:11434/v1", "qwen3:8b"],
  exLmStudio: ["http://127.0.0.1:1234/v1", ""],
};
for (const [id, [url, model]] of Object.entries(EXAMPLES)) {
  $(id).onclick = () => { $("chatUrl").value = url; $("chatModel").value = model; say("modelMsg", "", true); };
}
$("modelCheck").onclick = async () => {
  $("modelCheck").disabled = true;
  say("modelMsg", "모델에게 짧게 물어보는 중… 처음이면 모델을 올리느라 1~2분 걸릴 수 있습니다.", true);
  try {
    const r = await invoke("model_check", { url: $("chatUrl").value, model: $("chatModel").value });
    say("modelMsg", `답했습니다(${r.seconds}초): "${r.reply}". 저장을 누르세요.`, true);
  } catch (e) { say("modelMsg", String(e), false); }
  $("modelCheck").disabled = false;
};
$("modelSave").onclick = async () => {
  const m = { ...(models || { asr_url: "", asr_model: "" }), chat_url: $("chatUrl").value.trim(), chat_model: $("chatModel").value.trim() };
  try {
    await invoke("models_save", { models: m });
    models = m;
    say("modelMsg", "저장했습니다. 다음 질문부터 이 모델을 씁니다.", true);
  } catch (e) { say("modelMsg", String(e), false); }
};

// ---------- ③ mail and calendar ----------
initSources(() => {});

// ---------- ④ the phone ----------
$("openTailscale").onclick = () => invoke("open_tailscale_download").catch((e) => say("codeMsg", String(e), false));

async function loadPhone() {
  const info = await invoke("host_info").catch(() => null);
  const offered = fillAddresses($("devAddr"), info);
  $("devAddrNote").textContent = !offered.length
    ? "아직 폰이 닿을 주소가 없습니다. 이 PC를 와이파이(공유기)에 잇거나, Tailscale을 켜고 로그인하면 몇십 초 뒤 생깁니다. 그 뒤 '다시 보기'."
    : info.address_for_devices
      ? "Tailscale 주소는 Tailscale이 켜진 폰에서 어디서나 닿습니다."
      : "아직 tailnet 주소가 없어 같은 와이파이 주소를 씁니다. 밖에서도 닿게 하려면 이 PC와 폰에 Tailscale을 켜세요.";
  $("codeShow").disabled = !offered.length;
  showLanNote();
}
function showLanNote() {
  const lan = chosenIsLan($("devAddr"));
  $("lanNote").textContent = lan ? LAN_NOTE : "";
  $("lanNote").hidden = !lan;
}
$("devRecheck").onclick = loadPhone;
$("devAddr").onchange = () => { hideCode(); say("codeMsg", "", true); showLanNote(); };

function hideCode() {
  $("code").value = "";
  $("code").hidden = true;
  $("qr").removeAttribute("src");
  $("qr").hidden = true;
  $("codeCopy").hidden = true;
  $("codeHide").hidden = true;
  $("codeShow").hidden = false;
}
$("codeShow").onclick = async () => {
  try {
    const c = await invoke("phone_code", { url: $("devAddr").value });
    $("code").value = c.code;
    $("qr").src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(c.qr_svg);
    for (const id of ["code", "qr", "codeCopy", "codeHide"]) $(id).hidden = false;
    $("codeShow").hidden = true;
    say("codeMsg", "폰의 '서랍 연결'에서 '연결 코드 붙여 넣기'에 넣습니다. QR은 폰의 카메라나 QR 앱으로 읽어 나온 글을 복사해 붙여 넣으면 됩니다.", true);
  } catch (e) { say("codeMsg", String(e), false); }
};
$("codeHide").onclick = () => { hideCode(); say("codeMsg", "", true); };
$("codeCopy").onclick = async () => {
  try { await navigator.clipboard.writeText($("code").value); say("codeMsg", "복사했습니다. 폰에 붙여 넣은 뒤 클립보드를 비워 두면 안전합니다.", true); }
  catch (e) { say("codeMsg", "복사하지 못했습니다: " + e, false); }
};

// ---------- ⑤ the first sync and the graph ----------
const JOB = { mail: "메일", files: "첨부", calendar: "일정", contacts: "주소록", librarian: "색인", graph: "그래프", backup: "백업" };
let polling = null;
async function loadProgress() {
  const [st, p] = await Promise.all([invoke("sync_status").catch(() => null), invoke("first_progress").catch(() => null)]);
  if (p) {
    $("progress").textContent = `색인한 기록 ${p.records}건 · 그래프: 메일 ${p.mail}건 중 ${p.graph_read}건 읽음, ${p.graph_waiting}건 기다림`;
    $("graphNote").textContent = !models || !models.chat_url
      ? "대화 모델이 없어 그래프는 기다립니다(② 모델)."
      : p.graph_waiting ? "그래프는 동기화마다 이어서 읽습니다. 작은 모델을 CPU로 돌리면 메일 한 건에 몇 분 걸려 며칠이 걸릴 수 있습니다." : "";
  }
  const body = $("syncTable").tBodies[0];
  body.replaceChildren();
  const jobs = (st && st.jobs) || [];
  if (!jobs.length) { const tr = el("tr"); tr.append(el("td", "메일이나 일정을 넣으면 여기에 결과가 나옵니다(③).")); body.append(tr); }
  for (const j of jobs) {
    const r = j.result || {};
    const state = el("td", r.last_run ? (r.ok ? "성공" : "실패") : "아직");
    state.className = "state " + (r.last_run ? (r.ok ? "ok" : "err") : "");
    const tr = el("tr");
    tr.append(el("td", JOB[j.name] || j.name), state, el("td", r.detail || ""));
    body.append(tr);
  }
  const syncing = !!(st && st.syncing);
  $("syncNow").disabled = syncing;
  say("syncMsg", syncing ? "동기화하는 중…" : "", true);
  clearTimeout(polling);
  if (step === 4) polling = setTimeout(loadProgress, syncing ? 3000 : 15000);
}
$("syncNow").onclick = async () => {
  try { await invoke("sync_now"); say("syncMsg", "동기화하는 중…", true); setTimeout(loadProgress, 1500); }
  catch (e) { say("syncMsg", String(e), false); }
};

// ---------- start ----------
async function start() {
  const cfg = await invoke("server_config").catch((e) => ({ error: String(e) }));
  if (cfg.mode !== "host") { location.href = "index.html"; return; } // the guide is for a vault on this PC
  if (cfg.host_error) { $("hostErr").textContent = "이 PC의 서버를 띄우지 못했습니다: " + cfg.host_error; }
  const info = await invoke("host_info").catch(() => null);
  if (info) $("vaultDir").textContent = "노트 폴더: " + info.notes_dir;
  models = await invoke("models_get").catch(() => null);
  if (models) { $("chatUrl").value = models.chat_url; $("chatModel").value = models.chat_model; }
  await loadSources();
  show(0);
}
start();
