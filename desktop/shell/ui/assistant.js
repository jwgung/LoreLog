"use strict";
// The assistant screen, bundled in the desktop app (Tauri webview). It draws the same chat/todos/drawer the phone
// app has, but it is NOT served to browsers — it lives in the app and talks to the server's API over the network.
// The server address and token come from the app's config (via Tauri commands), not a login form in the page.
const $ = (id) => document.getElementById(id);
const invoke = window.__TAURI__?.core?.invoke;

let API_BASE = "";     // the server's base URL; API calls are made absolute with it
let token = "";
let wrong = null;      // the summary sentence the user is correcting
let busy = false;

// The diary day rolls over at 04:00 (same as the phone app).
function diaryDay() {
  const d = new Date(Date.now() - 4 * 3600 * 1000);
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

class Unauthorized extends Error {}
class NotConfigured extends Error {}
async function api(path, body) {
  if (!API_BASE || !token) throw new NotConfigured();
  const res = await fetch(API_BASE + path, {
    method: body === undefined ? "GET" : "POST",
    headers: { Authorization: `Bearer ${token}`, ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (res.status === 401) throw new Unauthorized();
  let data = {};
  try { data = await res.json(); } catch { /* empty body */ }
  return { status: res.status, data };
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

// ---------- first run: where the vault lives ----------
function showChoose(msg = "") {
  $("app").hidden = true;
  $("setup").hidden = true;
  $("choose").hidden = false;
  $("chooseError").textContent = msg;
}
async function choose(mode) {
  $("chooseError").textContent = mode === "host" ? "서버를 띄우는 중…" : "";
  try {
    await invoke("choose_mode", { mode });
    $("choose").hidden = true;
    await boot();
  } catch (e) {
    $("chooseError").textContent = String(e);
  }
}
$("chooseHost").addEventListener("click", () => choose("host"));
$("chooseConnect").addEventListener("click", () => choose("connect"));

// ---------- first-run setup (server address + token) ----------
function showSetup(msg = "") {
  $("app").hidden = true;
  $("setup").hidden = false;
  $("setupError").textContent = msg;
  $("urlInput").focus();
}
$("setupForm").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const url = $("urlInput").value.trim();
  const tok = $("tokenInput").value.trim();
  try {
    await invoke("save_settings", { url, token: tok || null });
    $("tokenInput").value = "";
    await boot();
  } catch (e) {
    $("setupError").textContent = String(e);
  }
});

// ---------- chat ----------
function bubble(role, text, fixed) {
  const m = el("div", `msg ${role}`);
  if (fixed) m.append(el("span", "fixed", fixed));
  m.append(document.createTextNode(text));
  $("thread").append(m);
  scrollDown();
  return m;
}
function scrollDown() { const t = $("thread"); t.scrollTop = t.scrollHeight; }

function renderSummary(draft) {
  const box = el("div", "summary");
  box.append(el("h2", "", "오늘 하루, 이렇게 정리해 봤어요"));
  const ul = el("ul");
  for (const s of draft.summary || []) {
    const b = el("button", "", s);
    b.type = "button";
    b.setAttribute("aria-pressed", "false");
    b.addEventListener("click", () => {
      const on = b.getAttribute("aria-pressed") !== "true";
      ul.querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", "false"));
      b.setAttribute("aria-pressed", String(on));
      setWrong(on ? s : null);
    });
    const li = el("li");
    li.append(b);
    ul.append(li);
  }
  box.append(ul, el("p", "hint", "틀린 문장이 있으면 눌러서 맞는 내용을 알려 주세요. 가만히 두면 맞는 것으로 알아요."));
  $("thread").append(box);
}

function setWrong(s) {
  wrong = s;
  $("wrongChip").hidden = !s;
  $("wrongText").textContent = s ? `이 문장이 틀렸어요: ${s}` : "";
  $("input").placeholder = s ? "맞는 내용을 알려 주세요" : "오늘 하루에 대해 말해 주세요";
  if (s) $("input").focus();
}
$("wrongClear").addEventListener("click", () => {
  document.querySelectorAll(".summary button").forEach((x) => x.setAttribute("aria-pressed", "false"));
  setWrong(null);
});

async function loadChat() {
  const day = diaryDay();
  $("dayLabel").textContent = new Date(day + "T00:00").toLocaleDateString("ko-KR", { month: "long", day: "numeric", weekday: "short" });
  $("thread").replaceChildren();
  const pending = bubble("assistant", "오늘을 정리하는 중이에요…");
  pending.classList.add("pending");
  let draft = null;
  for (let i = 0; i < 60 && !draft; i++) {   // generation can take a minute or two
    const r = await api(`/diary/${day}`);
    if (r.status === 200) draft = r.data;
    else if (r.status === 202) await new Promise((ok) => setTimeout(ok, 3000));
    else { pending.textContent = `정리하지 못했어요. (${r.data.error || r.status})`; break; }
  }
  if (!draft) { if (pending.isConnected && pending.classList.contains("pending") && /정리하는/.test(pending.textContent)) pending.textContent = "정리가 오래 걸리고 있어요. 잠시 뒤 새로고침해 주세요."; return; }
  pending.remove();
  renderSummary(draft);
  const hist = (await api(`/diary/${day}/chat`)).data.messages || [];
  if (!hist.length && draft.question) bubble("assistant", draft.question);
  for (const m of hist) bubble(m.role, m.text, m.wrong);
}

async function send(text) {
  if (busy) return;
  busy = true;
  $("send").disabled = true;
  const w = wrong;
  bubble("user", text, w);
  const wait = bubble("assistant", "…");
  wait.classList.add("pending");
  document.querySelectorAll(".summary button").forEach((x) => x.setAttribute("aria-pressed", "false"));
  setWrong(null);
  try {
    const r = await api(`/diary/${diaryDay()}/chat`, { message: text, ...(w ? { wrong: w } : {}) });
    wait.classList.remove("pending");
    wait.textContent = r.status === 200 ? r.data.reply : "답을 받지 못했어요. 말씀하신 내용은 저장됐어요.";
  } catch (e) {
    if (e instanceof Unauthorized) return showSetup("토큰이 맞지 않거나 바뀌었어요. 다시 넣어 주세요.");
    wait.classList.remove("pending");
    wait.textContent = "서버에 연결하지 못했어요.";
  } finally {
    busy = false;
    $("send").disabled = false;
    scrollDown();
  }
}
$("composer").addEventListener("submit", (ev) => {
  ev.preventDefault();
  const text = $("input").value.trim();
  if (!text) return;
  $("input").value = "";
  autosize();
  send(text);
});
$("input").addEventListener("keydown", (ev) => {
  if (ev.key === "Enter" && !ev.shiftKey && !ev.isComposing) { ev.preventDefault(); $("composer").requestSubmit(); }
});
function autosize() { const i = $("input"); i.style.height = "auto"; i.style.height = Math.min(i.scrollHeight, 160) + "px"; }
$("input").addEventListener("input", autosize);

// ---------- dictation: record, convert to 16 kHz mono wav, let the server transcribe ----------
const ICON_MIC = $("mic").innerHTML;
const ICON_STOP = '<svg viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>';
const MAX_RECORD_MS = 8 * 60 * 1000;   // 8 minutes of 16 kHz mono 16-bit is about 15 MB; the server accepts 20 MB
let rec = null;

function encodeWav(samples, rate) {
  const buf = new ArrayBuffer(44 + samples.length * 2);
  const v = new DataView(buf);
  const str = (o, t) => [...t].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  str(0, "RIFF"); v.setUint32(4, 36 + samples.length * 2, true); str(8, "WAVEfmt ");
  v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true);
  v.setUint32(24, rate, true); v.setUint32(28, rate * 2, true); v.setUint16(32, 2, true); v.setUint16(34, 16, true);
  str(36, "data"); v.setUint32(40, samples.length * 2, true);
  samples.forEach((x, i) => v.setInt16(44 + i * 2, Math.max(-1, Math.min(1, x)) * 0x7fff, true));
  return new Blob([buf], { type: "audio/wav" });
}
async function toWav(blob) {
  const ctx = new AudioContext();
  const decoded = await ctx.decodeAudioData(await blob.arrayBuffer());
  ctx.close();
  const off = new OfflineAudioContext(1, Math.ceil(decoded.duration * 16000), 16000);
  const src = off.createBufferSource();
  src.buffer = decoded;
  src.connect(off.destination);
  src.start();
  return encodeWav((await off.startRendering()).getChannelData(0), 16000);
}
function micState(state) {   // idle | rec | work
  const m = $("mic");
  m.classList.toggle("rec", state === "rec");
  m.disabled = state === "work";
  m.innerHTML = state === "rec" ? ICON_STOP : ICON_MIC;
  m.title = m.ariaLabel = state === "rec" ? "끝내기" : "말하기";
  if (state === "work") $("input").placeholder = "받아 적는 중…";
  else if (!wrong) $("input").placeholder = "오늘 하루에 대해 말해 주세요";
}
async function startRecording() {
  if (!navigator.mediaDevices?.getUserMedia) { $("input").placeholder = "마이크를 쓸 수 없어요"; return; }
  let stream;
  try { stream = await navigator.mediaDevices.getUserMedia({ audio: true }); }
  catch { $("input").placeholder = "마이크 권한이 필요해요"; return; }
  const chunks = [];
  const mr = new MediaRecorder(stream);
  mr.ondataavailable = (e) => e.data.size && chunks.push(e.data);
  mr.onstop = async () => {
    stream.getTracks().forEach((t) => t.stop());
    clearTimeout(rec?.timer);
    rec = null;
    micState("work");
    let note = "";   // shown in the input after the mic returns to idle (idle resets the placeholder)
    try {
      const wav = await toWav(new Blob(chunks, { type: mr.mimeType }));
      const res = await fetch(API_BASE + "/transcribe", { method: "POST", headers: { Authorization: `Bearer ${token}` }, body: wav });
      if (res.status === 401) return showSetup("연결이 끊겼어요. 토큰을 다시 넣어 주세요.");
      const { text = "" } = await res.json().catch(() => ({}));
      if (text) { const i = $("input"); i.value = (i.value ? i.value + " " : "") + text; autosize(); i.focus(); }
      else note = "잘 들리지 않았어요. 다시 말해 주세요";
    } catch { note = "받아 적지 못했어요"; }
    finally { micState("idle"); if (note) $("input").placeholder = note; }
  };
  mr.start();
  rec = { mr, timer: setTimeout(() => mr.state === "recording" && mr.stop(), MAX_RECORD_MS) };
  micState("rec");
}
$("mic").addEventListener("click", () => (rec ? rec.mr.stop() : startRecording()));

// ---------- to-dos ----------
const ICON_CHECK = '<svg viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5"/></svg>';
const ICON_X = '<svg viewBox="0 0 24 24"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>';
function iconButton(svg, label, onClick) {
  const b = el("button");
  b.type = "button";
  b.title = b.ariaLabel = label;
  b.innerHTML = svg;   // constant markup defined above, never user text
  b.addEventListener("click", onClick);
  return b;
}
async function actTodo(id, action) { await api(`/todos/${id}`, { action }); await loadTodos(false); }

const ICON_PLUS = '<svg viewBox="0 0 24 24"><path d="M12 5v14"/><path d="M5 12h14"/></svg>';

// A proposal the assistant found in the records: the user takes it (it becomes a to-do) or dismisses it.
function proposedRow(t) {
  const li = el("li", "todo proposed");
  const body = el("div", "t");
  body.append(document.createTextNode(t.title));
  const meta = [t.source, t.due].filter(Boolean).join(" · ");
  if (meta) body.append(el("div", "src", meta));
  li.append(body, iconButton(ICON_PLUS, "할 일로 받기", () => actTodo(t.id, "accept")), iconButton(ICON_X, "아니에요", () => actTodo(t.id, "dismiss")));
  return li;
}

function todoRow(t, done) {
  const li = el("li", `todo${done ? " done" : ""}`);
  const body = el("div", "t");
  body.append(document.createTextNode(t.title));
  const meta = [t.source, t.due].filter(Boolean).join(" · ");
  if (meta) body.append(el("div", "src", meta));
  if (done && t.auto) body.append(el("div", "auto", "답장을 보내서 자동으로 끝냈어요"));
  li.append(body);
  if (done) li.append(iconButton(ICON_X, "다시 열기", () => actTodo(t.id, "reopen")));
  else li.append(iconButton(ICON_CHECK, "끝냈어요", () => actTodo(t.id, "done")), iconButton(ICON_X, "필요 없어요", () => actTodo(t.id, "dismiss")));
  return li;
}

let todoTimer = null;
async function loadTodos(extract) {
  if (extract) api("/todos/extract", {}).catch(() => {});
  const { data } = await api("/todos");
  const proposed = data.proposed || [];
  $("todoProposed").replaceChildren(...proposed.map(proposedRow));
  $("proposedBox").hidden = proposed.length === 0;
  $("todoOpen").replaceChildren(...(data.open || []).map((t) => todoRow(t, false)));
  $("todoDone").replaceChildren(...(data.done || []).map((t) => todoRow(t, true)));
  const n = (data.open || []).length + proposed.length;
  $("todoBadge").hidden = n === 0;
  $("todoBadge").textContent = n;
  $("extracting").hidden = !data.extracting;
  clearTimeout(todoTimer);
  if (data.extracting) todoTimer = setTimeout(() => loadTodos(false), 4000);
}
$("todoAdd").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const text = $("todoText").value.trim();
  if (!text) return;
  $("todoText").value = "";
  await api("/todos", { text });
  await loadTodos(false);
});
$("btnTodos").addEventListener("click", () => document.querySelector("main").classList.toggle("showTodos"));

// ---------- drawer: which messengers may feed the diary ----------
let senderRows = [];
let rawAllow = [];
async function openDrawer() {
  $("drawer").hidden = false;
  $("senderMsg").textContent = "";
  const [s, a] = await Promise.all([api("/diary/senders"), api("/diary/allow")]);
  senderRows = Array.isArray(s.data) ? s.data : [];
  rawAllow = a.data.allow || [];
  const groups = new Map();
  for (const r of senderRows) { if (!groups.has(r.label)) groups.set(r.label, []); groups.get(r.label).push(r); }
  const root = $("senders");
  root.replaceChildren();
  if (!senderRows.length) root.append(el("p", "muted", "최근 14일 동안 알림을 보낸 상대가 없어요."));
  for (const [label, rows] of groups) {
    const g = el("div", "group");
    const h = el("h4", "", label);
    const all = el("button", "", "모두 선택");
    all.type = "button";
    all.addEventListener("click", () => {
      const boxes = g.querySelectorAll("input");
      const target = [...boxes].some((b) => !b.checked);
      boxes.forEach((b) => (b.checked = target));
      all.textContent = target ? "모두 해제" : "모두 선택";
    });
    h.append(all);
    g.append(h);
    for (const r of rows) {
      const lab = el("label", "sender");
      const cb = el("input");
      cb.type = "checkbox";
      cb.checked = r.allowed;
      cb.dataset.key = `${r.label}: ${r.title}`;
      lab.append(cb, el("span", "", r.title), el("span", "n", `${r.count}건`));
      g.append(lab);
    }
    root.append(g);
  }
}
$("saveSenders").addEventListener("click", async () => {
  const shown = new Set(senderRows.map((r) => `${r.label}: ${r.title}`));
  // keep what the list cannot show (an "app: *" rule, or a sender that has not written in 14 days)
  const keep = rawAllow.filter((k) => !shown.has(k));
  const checked = [...document.querySelectorAll("#senders input:checked")].map((b) => b.dataset.key);
  const r = await api("/diary/senders", { allow: [...keep, ...checked] });
  $("senderMsg").textContent = r.status === 200 ? "저장했어요. 다음 정리부터 반영돼요." : "저장하지 못했어요.";
});
$("btnDrawer").addEventListener("click", () => openDrawer().catch(onError));
$("drawerClose").addEventListener("click", () => ($("drawer").hidden = true));
$("drawer").addEventListener("click", (ev) => { if (ev.target === $("drawer")) $("drawer").hidden = true; });
$("toSettings").addEventListener("click", () => { location.href = "settings.html"; });
document.addEventListener("keydown", (ev) => { if (ev.key === "Escape") $("drawer").hidden = true; });

// ---------- start ----------
function onError(e) {
  if (e instanceof Unauthorized) showSetup("토큰이 맞지 않거나 바뀌었어요.");
  else if (e instanceof NotConfigured) showSetup();
  else console.error(e);
}
async function start() {
  $("setup").hidden = true;
  $("app").hidden = false;
  try {
    await Promise.all([loadChat(), loadTodos(true)]);
  } catch (e) { onError(e); }
}

// Read the server address and token from the app's config (set in settings). If not configured, show setup.
async function boot() {
  if (!invoke) { showSetup("이 화면은 데스크톱 앱 안에서만 동작합니다."); return; }
  try {
    const s = await invoke("server_config");   // { mode, base, token }
    if (!s.mode) { showChoose(); return; }
    if (s.host_error) { showChoose(`이 PC의 서버를 띄우지 못했습니다: ${s.host_error}`); return; }
    API_BASE = (s.base || "").replace(/\/+$/, "");
    token = s.token || "";
    if (API_BASE && token) start(); else showSetup();
  } catch (e) {
    showSetup(String(e));
  }
}

boot();
