"use strict";
const $ = (id) => document.getElementById(id);
const TOKEN_KEY = "life.token";
const store = {
  get() { try { return localStorage.getItem(TOKEN_KEY) || ""; } catch { return ""; } },
  set(v) { try { localStorage.setItem(TOKEN_KEY, v); } catch { /* private window: the token lasts for this page only */ } },
  clear() { try { localStorage.removeItem(TOKEN_KEY); } catch { /* nothing stored */ } },
};
let token = store.get();
let wrong = null;      // the summary sentence the user is correcting
let busy = false;

// The diary day rolls over at 04:00 (same as the phone app).
function diaryDay() {
  const d = new Date(Date.now() - 4 * 3600 * 1000);
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

class Unauthorized extends Error {}
async function api(path, body) {
  const res = await fetch(path, {
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

// ---------- login ----------
function showLogin(msg = "") {
  $("app").hidden = true;
  $("login").hidden = false;
  $("loginError").textContent = msg;
  $("tokenInput").focus();
}
$("loginForm").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  token = $("tokenInput").value.trim();
  try {
    await api("/todos");
    store.set(token);
    $("tokenInput").value = "";
    start();
  } catch (e) {
    showLogin(e instanceof Unauthorized ? "토큰이 맞지 않아요." : "서버에 연결하지 못했어요.");
  }
});
$("logout").addEventListener("click", () => { store.clear(); token = ""; $("drawer").hidden = true; showLogin(); });

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
    if (e instanceof Unauthorized) return showLogin("연결이 끊겼어요. 토큰을 다시 입력해 주세요.");
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
  $("todoOpen").replaceChildren(...(data.open || []).map((t) => todoRow(t, false)));
  $("todoDone").replaceChildren(...(data.done || []).map((t) => todoRow(t, true)));
  const n = (data.open || []).length;
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
document.addEventListener("keydown", (ev) => { if (ev.key === "Escape") $("drawer").hidden = true; });

// ---------- start ----------
function onError(e) {
  if (e instanceof Unauthorized) showLogin("토큰이 맞지 않거나 바뀌었어요.");
  else console.error(e);
}
async function start() {
  $("login").hidden = true;
  $("app").hidden = false;
  try {
    await Promise.all([loadChat(), loadTodos(true)]);
  } catch (e) { onError(e); }
}
if (token) start(); else showLogin();
